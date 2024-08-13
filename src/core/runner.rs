use anyhow::anyhow;
use futures::StreamExt;
use std::{collections::HashMap, process, sync::Arc, time};
use tokio::{sync::broadcast, time::sleep};
use url::Url;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;

use crate::{error::Error, integrations::therun::TheRunReturnJson, ActorRef, Rto};

use super::db::ProjectDb;

pub enum RunnerRequest {
    Create(Runner, Rto<()>),
    Update(Runner, Rto<()>),
    RefreshStream(i64, Rto<bool>),
    Delete(i64, Rto<()>),
}

/// Notifies the TheRun.gg poller of a change in runner TheRun.gg status
enum TheRunAlert {
    AddRunner(Runner),
    RemoveRunner(Runner),
}

/// A list of TheRun.gg users who are to be polled
type LiveRunners = Arc<tokio::sync::Mutex<Vec<String>>>;

/// Worker to manage TheRun.gg connections
async fn therun_poller(
    db: Arc<ProjectDb>,
    mut therun_rx: tokio::sync::mpsc::UnboundedReceiver<TheRunAlert>,
) -> anyhow::Result<()> {
    let live_runners = LiveRunners::default();
    let (death_tx, _) = broadcast::channel(16);
    while let Some(alert) = therun_rx.recv().await {
        match alert {
            TheRunAlert::AddRunner(runner) => {
                live_runners
                    .lock()
                    .await
                    .push(runner.get_therun_username().expect("No username"));
                tokio::spawn(create_therun_websocket_monitor(
                    db.clone(),
                    runner.id,
                    runner.get_therun_username().expect("No username"),
                    live_runners.clone(),
                    death_tx.clone(),
                ));
            }
            TheRunAlert::RemoveRunner(runner) => {
                live_runners
                    .lock()
                    .await
                    .retain(|r| r != &runner.get_therun_username().expect("No username"));
                death_tx
                    .send(runner.get_therun_username().expect("No username"))
                    .ok();
            }
        }
    }

    Ok(())
}

/// Creates a player info websocket, restarting it on failure.
async fn create_therun_websocket_monitor(
    db: Arc<ProjectDb>,
    runner: i64,
    therun: String,
    runners: LiveRunners,
    death_monitor: broadcast::Sender<String>,
) -> Result<(), anyhow::Error> {
    loop {
        let res = tokio::spawn(run_runner_websocket(
            db.clone(),
            runner,
            therun.clone(),
            death_monitor.subscribe(),
        ))
        .await;

        if runners.lock().await.contains(&therun) {
            match res {
                Ok(_) => log::warn!(
                    "TheRun.gg WebSocket closed for {} ({}), reattempting in 30 seconds...",
                    runner,
                    therun
                ),
                Err(error) => log::warn!(
                    "TheRun.gg WebSocket closed for {} ({}, {}), reattempting in 30 seconds...",
                    runner,
                    therun,
                    error.to_string()
                ),
            }
            sleep(time::Duration::from_secs(30)).await;
        } else {
            log::info!("TheRun.gg WebSocket closed for {} ({})", runner, therun);
            return Ok(());
        }
    }
}

/// Creates a websocket for the provided runner.
///
/// This function will occasionally completely bypass the return or panic when failing,
/// so it is restarted by ```create_player_websocket```.
async fn run_runner_websocket(
    db: Arc<ProjectDb>,
    runner: i64,
    therun: String,
    mut death_monitor: broadcast::Receiver<String>,
) -> Result<(), anyhow::Error> {
    let (mut stream, _) = tokio_tungstenite::connect_async(
        Url::parse(&format!("wss://ws.therun.gg/?username={}", therun)).unwrap(),
    )
    .await?;

    log::info!("TheRun.gg WebSocket open for {} ({})", runner, therun);

    loop {
        tokio::select! {
            kill_name = death_monitor.recv() => {
                if let Ok(kill_name) = kill_name {
                    if kill_name == therun {
                        log::debug!("Killing TheRun.gg websocket for {} ({})", runner, therun);
                        return Ok(());
                    }
                }
            }
            message = stream.next() => {
                match message {
                    Some(Ok(msg)) => {
                        match serde_json::from_str::<TheRunReturnJson>(
                            msg.to_text().expect("Failed to get text"),
                        ) {
                            Ok(stats) => {
                                log::debug!("Received TheRun.gg data for {}", runner);

                                match db.set_runner_run_data(runner, &stats.run).await {
                                    Ok(_) => {}
                                    Err(e) => log::error!("Failed to update runner {}'s run data: {}", runner, e),
                                };
                            }
                            Err(err) => {
                                log::warn!("Failed to parse {} endpoint: {}", runner, err.to_string());
                            }
                        };
                    }
                    Some(Err(err)) => log::error!("{}", err.to_string().replace("\\\"", "\"")),
                    None => return Ok(()),
                }
            }
        }
    }
}

pub async fn run_runner_actor(
    db: Arc<ProjectDb>,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<RunnerRequest>,
) -> anyhow::Result<()> {
    let (therun_tx, therun_rx) = tokio::sync::mpsc::unbounded_channel::<TheRunAlert>();
    tokio::spawn(therun_poller(db.clone(), therun_rx));

    // Add all existing runners to TheRun.gg poller.
    for runner in db.get_runners().await? {
        if runner.get_therun_username().is_some() {
            let _ = therun_tx.send(TheRunAlert::AddRunner(runner.clone()));
        }
    }

    while let Some(msg) = rx.recv().await {
        match msg {
            RunnerRequest::Create(mut runner, rto) => {
                log::info!("Creating runner {}", runner.name);
                match db.add_runner(&mut runner).await {
                    Ok(_) => {
                        if runner.get_therun_username().is_some() {
                            let _ = therun_tx.send(TheRunAlert::AddRunner(runner.clone()));
                        }
                        rto.reply(Ok(()));
                    }
                    Err(e) => rto.reply(Err(e)),
                }
            }
            RunnerRequest::Update(runner, rto) => {
                let old_runner = db.get_runner(runner.id).await;
                match old_runner {
                    Ok(old_runner) => {
                        // Check for changes in TheRun.gg username
                        if old_runner.get_therun_username() != runner.get_therun_username() {
                            let _ = therun_tx.send(TheRunAlert::RemoveRunner(old_runner.clone()));
                            if runner.get_therun_username().is_some() {
                                let _ = therun_tx.send(TheRunAlert::AddRunner(runner.clone()));
                            }
                        }

                        match db.update_runner(&runner).await {
                            Ok(_) => rto.reply(Ok(())),
                            Err(e) => {
                                log::error!(
                                    "Failed to update runner {} ({}): {}",
                                    runner.name,
                                    runner.id,
                                    e
                                );
                                rto.reply(Err(e))
                            }
                        }
                    }
                    Err(e) => rto.reply(Err(e)),
                }
            }
            RunnerRequest::RefreshStream(runner, rto) => match db.get_runner(runner).await {
                Ok(mut runner) => match runner.find_and_save_stream(&db).await {
                    Ok(changed) => rto.reply(Ok(changed)),
                    Err(e) => rto.reply(Err(e)),
                },
                Err(_) => rto.reply(Err(anyhow!("Runner {} not found", runner))),
            },
            RunnerRequest::Delete(id, rto) => match db.get_events_for_runner(id).await {
                Err(e) => rto.reply(Err(e)),
                Ok(ev) => {
                    let runner_name = db
                        .get_name_for_runner(id)
                        .await
                        .expect("Failed to get runner name");

                    log::info!("Deleting runner {}", runner_name);
                    if ev.is_empty() {
                        let runner = db.get_runner(id).await?;
                        if runner.get_therun_username().is_some() {
                            let _ = therun_tx.send(TheRunAlert::RemoveRunner(runner));
                        }
                        rto.reply(db.delete_runner(id).await)
                    } else {
                        rto.reply(Err(anyhow!(
                            "Cannot delete runner {} as they are associated with
                             the following events: {:?}",
                            runner_name,
                            ev.to_vec()
                        )))
                    }
                }
            },
        }
    }

    Ok(())
}

pub type RunnerActor = ActorRef<RunnerRequest>;

#[derive(PartialEq, Eq, Debug, FromRow, Clone, Serialize, Deserialize)]
pub struct Runner {
    /// Unique runner ID
    pub id: i64,

    /// Player's display name
    pub name: String,

    /// Player's stream link
    /// If the link is an https:// address,
    /// it is used as-is, otherwise it is treated
    /// as a Twitch handle.
    ///
    /// This is assumed to be the same as the name if None
    pub stream: Option<String>,

    /// Player's TheRun.gg username.
    ///
    /// This is assumed to be the same as the name if None
    pub therun: Option<String>,

    /// A value to manually specify a .m3u8 link in case Streamlink
    /// fails to acquire it
    pub override_stream_url: Option<String>,

    /// Player's location in ISO 3166-2
    pub location: Option<String>,

    /// Encoded player photo
    #[serde(skip)]
    pub photo: Option<Vec<u8>>,

    /// User volume in percent
    pub volume_percent: u32,

    #[sqlx(skip)]
    pub nicks: Vec<String>,

    #[sqlx(skip)]
    /// A map of streamlink resolutions (including `best`) to .m3u8 links.
    /// This is None if streamlink does not return values.
    pub stream_urls: HashMap<String, String>,
}

#[derive(PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct FieldDefault {
    pub field: String,
    pub value: String,
}

impl Runner {
    pub fn get_therun_username(&self) -> Option<String> {
        self.therun.clone()
    }

    pub fn get_stream(&self) -> String {
        match &self.stream {
            Some(stream) => {
                if stream.starts_with("https://") || stream.starts_with("http://") {
                    stream.to_string()
                } else {
                    format!("https://twitch.tv/{}", stream)
                }
            }
            None => format!("https://twitch.tv/{}", self.name),
        }
    }

    /// Returns a .m3u8 link corresponding to the current players' stream.
    pub fn find_stream(&mut self) -> anyhow::Result<bool> {
        let output = process::Command::new("streamlink")
            .arg("-Q")
            .arg("-j")
            .arg(&self.get_stream())
            .output()
            .map_err(|e| anyhow!("Failed to acquire stream for {}: {:?}", &self.name, e))?;

        let json = std::str::from_utf8(output.stdout.as_slice())?;

        let parsed_json: Value =
            serde_json::from_str(json).expect("Unable to parse streamlink output");

        if parsed_json.get("error").is_some() {
            Err(Error::FailedStreamAcq(
                self.name.clone(),
                parsed_json["error"].to_string(),
            ))?
        } else {
            let mut new_urls = HashMap::new();

            for (resolution, contents) in parsed_json["streams"].as_object().unwrap() {
                let url = contents["url"].to_string().replace('\"', "");
                new_urls.insert(resolution.to_string(), url);
            }

            if new_urls != self.stream_urls {
                self.stream_urls = new_urls;
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }

    async fn find_and_save_stream(&mut self, db: &ProjectDb) -> anyhow::Result<bool> {
        match self.find_stream() {
            Ok(changed) => {
                if changed {
                    log::info!("Updating stream url for {}", self.name);
                    db.update_runner(self).await?;
                } else {
                    log::info!("Stream url for {} is up to date", self.name);
                }
                Ok(changed)
            }
            Err(e) => {
                log::warn!("Failed to find stream for {}: {}", self.name, e);
                db.update_runner(self).await?;
                Err(e)?
            },
        }
    }
}
