use std::sync::Arc;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    core::db::ProjectDb, integrations::obs::ObsCommand, send_message, ActorRef, Directory, Rto,
};

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug, FromRow)]
pub struct StreamState {
    pub event: i64,
    pub obs_host: String,
    /// Semicolon-separated list of commentators
    pub active_commentators: String,
    pub ignored_commentators: String,
    pub audible_runner: Option<i64>,
    pub requested_layout: Option<String>,

    #[sqlx(skip)]
    pub stream_runners: Vec<i64>,
}

impl StreamState {
    /// Returns all active commentators
    pub fn get_commentators(&self) -> Vec<String> {
        let mut commentators = self
            .active_commentators
            .split(";")
            .map(|s| s.to_string())
            .collect::<Vec<String>>();

        let ignored_vec = self
            .ignored_commentators
            .split(";")
            .map(|s| s.to_string())
            .collect::<Vec<String>>();

        commentators.retain(|c| !&ignored_vec.contains(c));

        commentators
    }
}

/// Requests that can be sent to a StateActor
pub enum StreamRequest {
    CreateStream(i64, String, Rto<()>),
    ReloadStream(i64, Rto<()>),
    UpdateStream(StreamState, Rto<()>),
    DeleteStream(i64, Rto<()>),
}

pub type StreamActor = ActorRef<StreamRequest>;

/// Elements of ProjectState that were modified during a state change.
#[derive(PartialEq, Debug)]
pub enum ModifiedStreamState {
    RunnerView(i64),
    Layout,
    Commentary,
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
            Ok(db.get_streamed_events().await?[0].clone())
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
            StreamRequest::CreateStream(event, host, rto) => {
                log::debug!("Creating stream for {}", event);
                if let Ok(host_in_use) = db.is_host_in_use(&host).await {
                    if host_in_use {
                        rto.reply(Err(anyhow!(
                            "Host '{}' is already in use, cannot create stream for event {}.",
                            host,
                            event
                        )));
                    }
                } else if let Ok(_) = db.get_stream(event).await {
                    rto.reply(Err(anyhow!(
                        "Stream for event {} already exists, cannot create a new stream.",
                        event
                    )));
                } else {
                    let state = StreamState {
                        event,
                        obs_host: host,
                        active_commentators: "".to_string(),
                        ignored_commentators: "".to_string(),
                        requested_layout: None,
                        stream_runners: vec![],
                        audible_runner: None,
                    };

                    rto.reply(db.save_stream(&state).await);
                }
            }
            StreamRequest::UpdateStream(new_stream, rto) => {
                match db.get_stream(new_stream.event).await {
                    Ok(stream) => {
                        let diffs = new_stream.determine_modified_state(&stream);
                        db.save_stream(&new_stream).await?;
                        rto.reply(send_message!(
                            directory.obs_actor,
                            ObsCommand,
                            UpdateState,
                            new_stream.event,
                            diffs
                        ));
                    }
                    Err(e) => {
                        rto.reply(Err(anyhow!(
                            "No stream found for event '{}': {:?}.",
                            new_stream.event,
                            e
                        )));
                    }
                }
            }
            StreamRequest::ReloadStream(stream, rto) => rto.reply(send_message!(
                directory.obs_actor,
                ObsCommand,
                UpdateState,
                stream,
                Vec::<ModifiedStreamState>::new()
            )),
            StreamRequest::DeleteStream(event, rto) => {
                rto.reply(db.delete_stream(event).await);
            }
        }
    }

    Ok(())
}

impl StreamState {
    pub fn determine_modified_state(&self, old: &StreamState) -> Vec<ModifiedStreamState> {
        let mut modifications = Vec::<ModifiedStreamState>::new();
        for (idx, runner) in self.stream_runners.iter().enumerate() {
            if old.stream_runners.iter().position(|r| r == runner) != Some(idx) {
                // Runner was added or moved
                modifications.push(ModifiedStreamState::RunnerView(*runner));
            }
        }

        if (self.stream_runners.len() != old.stream_runners.len())
            || self.requested_layout != old.requested_layout
        {
            modifications.push(ModifiedStreamState::Layout);
        }

        if self.get_commentators() != old.get_commentators() {
            modifications.push(ModifiedStreamState::Commentary);
        }

        modifications
    }
}
