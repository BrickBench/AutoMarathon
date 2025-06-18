use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    core::{db::ProjectDb, runner::RunnerRequest},
    integrations::obs::HostCommand,
    send_message, ActorRef, Directory, Rto,
};

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug, FromRow)]
pub struct StreamState {
    pub event: i64,
    pub obs_host: String,
    pub audible_runner: Option<i64>,
    pub requested_layout: Option<String>,

    #[sqlx(skip)]
    /// Map of view IDs to runner IDs
    pub stream_runners: HashMap<i64, i64>,
}

/// Requests that can be sent to a StateActor
pub enum StreamRequest {
    Create(i64, String, Rto<()>),
    Reload(i64, Rto<()>),
    Update(StreamState, Rto<()>),
    Delete(i64, Rto<()>),
}

pub type StreamActor = ActorRef<StreamRequest>;

/// Elements of ProjectState that were modified during a state change.
#[derive(PartialEq, Debug)]
pub enum ModifiedStreamState {
    RunnerView(i64),
    Layout,
}

/// Verify the ID of a streamed event.
///
/// This function returns the contents of `event_id`,
/// or attempts to get the name of the single active
/// stream if `event_id` is `None`
pub async fn validate_streamed_event_id(
    db: &ProjectDb,
    event_id: Option<i64>,
) -> anyhow::Result<i64> {
    if let Some(event_id) = event_id {
        match db.get_stream(event_id).await {
            Ok(_) => Ok(event_id),
            Err(e) => Err(anyhow!(
                "No stream for event '{}' exists: {:?}.",
                &event_id,
                e
            )),
        }
    } else {
        let count = db.get_stream_count().await?;
        if count == 1 {
            Ok(db.get_streamed_events().await?[0])
        } else if count == 0 {
            Err(anyhow!("Cannot determine event, no streams are active."))
        } else {
            Err(anyhow!(
                "Multiple streams are active, please specify the streamed event to use."
            ))
        }
    }
}

pub async fn run_stream_manager(
    db: Arc<ProjectDb>,
    mut rx: UnboundedReceiver<StreamRequest>,
    directory: Directory,
) -> Result<(), anyhow::Error> {
    log::debug!("Started stream state manager");
    while let Some(msg) = rx.recv().await {
        match msg {
            StreamRequest::Create(event, host, rto) => {
                log::debug!("Creating stream for {} using {}", event, host);

                let obs_host_data = send_message!(directory.obs_actor, HostCommand, GetState);

                if obs_host_data.is_err() {
                    let msg = format!(
                        "Failed to get OBS host data, cannot create stream for event {}.",
                        event
                    );
                    log::warn!("{}", msg);
                    rto.reply(Err(anyhow!(msg)));
                } else if !obs_host_data.as_ref().unwrap().contains_key(&host) {
                    let msg = format!(
                        "Host '{}' is not a valid OBS host, cannot create stream for event {}.",
                        host, event
                    );
                    log::warn!("{}", msg);
                    rto.reply(Err(anyhow!(msg)));
                } else if !obs_host_data.unwrap().get(&host).unwrap().connected {
                    let msg = format!(
                        "Host '{}' is not connected, cannot create stream for event {}.",
                        host, event
                    );
                    log::warn!("{}", msg);
                    rto.reply(Err(anyhow!(msg)));
                } else if db
                    .is_host_in_use(&host)
                    .await
                    .expect("Failed to get host usage")
                {
                    let msg = format!(
                        "Host '{}' is already in use, cannot create stream for event {}.",
                        host, event
                    );
                    log::warn!("{}", msg);
                    rto.reply(Err(anyhow!(msg)));
                } else if (db.get_stream(event).await).is_ok() {
                    let msg = format!(
                        "Stream for event {} already exists, cannot create a new stream.",
                        event
                    );
                    log::warn!("{}", msg);
                    rto.reply(Err(anyhow!(msg)));
                } else {
                    let state = StreamState {
                        event,
                        obs_host: host,
                        requested_layout: None,
                        stream_runners: HashMap::new(),
                        audible_runner: None,
                    };

                    match db.save_stream(&state).await {
                        Ok(_) => rto.reply(Ok(())),
                        Err(e) => {
                            log::error!("Failed to create stream for event {}: {:?}", event, e);
                            rto.reply(Err(e));
                        }
                    }
                }
            }
            StreamRequest::Update(new_stream, rto) => match db.get_stream(new_stream.event).await {
                Ok(stream) => {
                    let _bad_runners = new_stream.trigger_refreshes(&stream, &directory).await;
                    let diffs = new_stream.determine_modified_state(&stream);
                    db.save_stream(&new_stream).await?;

                    let worked = send_message!(
                        directory.obs_actor,
                        HostCommand,
                        UpdateState,
                        new_stream.event,
                        diffs
                    );

                    if worked.is_err() {
                        log::warn!(
                            "Failed to update OBS state for event {}: {:?}",
                            new_stream.event,
                            worked
                        );
                    }

                    rto.reply(worked);
                }
                Err(e) => {
                    rto.reply(Err(anyhow!(
                        "No stream found for event '{}': {:?}.",
                        new_stream.event,
                        e
                    )));
                }
            },
            StreamRequest::Reload(stream, rto) => rto.reply(send_message!(
                directory.obs_actor,
                HostCommand,
                UpdateState,
                stream,
                Vec::<ModifiedStreamState>::new()
            )),
            StreamRequest::Delete(event, rto) => {
                rto.reply(db.delete_stream(event).await);
            }
        }
    }

    Ok(())
}

impl StreamState {
    /// Find the runners that changed between the previous and current stream states,
    /// and trigger stream refreshes for those runners.
    ///
    /// Return the runners which failed to get streams.
    pub async fn trigger_refreshes(&self, old: &StreamState, directory: &Directory) -> Vec<i64> {
        let old_runners = HashSet::<i64>::from_iter(old.stream_runners.values().cloned());
        let new_runners = HashSet::<i64>::from_iter(self.stream_runners.values().cloned());
        let added_runners = new_runners.difference(&old_runners);

        let mut bad_runners = vec![];
        for runner in added_runners {
            if let Err(e) = send_message!(
                directory.runner_actor,
                RunnerRequest,
                RefreshStream,
                *runner
            ) {
                log::warn!(
                    "Failed to update runner stream for {} when entering view: {:?}",
                    runner,
                    e
                );
                bad_runners.push(*runner);
            }
        }

        bad_runners
    }

    /// Determine which stream elements were modified between the previous and current states.
    pub fn determine_modified_state(&self, old: &StreamState) -> Vec<ModifiedStreamState> {
        let mut modifications = Vec::<ModifiedStreamState>::new();
        for (pos, runner) in self.stream_runners.iter() {
            if !old.stream_runners.contains_key(pos) {
                // Runner was added
                modifications.push(ModifiedStreamState::RunnerView(*runner));
            } else if old.stream_runners.get(pos) != Some(runner) {
                // Runner was moved
                modifications.push(ModifiedStreamState::RunnerView(*runner));
            }
        }

        if (self.stream_runners.len() != old.stream_runners.len())
            || self.requested_layout != old.requested_layout
        {
            modifications.push(ModifiedStreamState::Layout);
        }

        modifications
    }

    /// Determine the stream slot for the given runner.
    pub fn get_runner_slot(&self, runner: i64) -> Option<i64> {
        self.stream_runners
            .iter()
            .find_map(|(k, v)| if *v == runner { Some(*k) } else { None })
    }

    /// Find the slot not currently assigned to a runner.
    pub fn get_first_empty_slot(&self) -> i64 {
        let mut i = 1;
        while self.stream_runners.contains_key(&i) {
            i += 1;
        }
        i
    }
}
