use anyhow::anyhow;
use std::{collections::HashMap, process, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;

use crate::{
    error::Error, integrations::therun::TheRunCommand, send_nonblocking, ActorRef, Directory, Rto,
};

use super::{db::ProjectDb, participant::Participant, settings::Settings};

pub type ParticipantId = i64;

pub enum RunnerRequest {
    Create(Runner, Rto<()>),
    Update(Runner, Rto<()>),
    RefreshStream(i64, Rto<bool>),
    Delete(i64, Rto<()>),
}

pub async fn run_runner_actor(
    directory: Directory,
    db: Arc<ProjectDb>,
    settings: Arc<Settings>,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<RunnerRequest>,
) -> anyhow::Result<()> {
    // Add all existing runners to TheRun.gg poller.
    for runner in db.get_runners().await? {
        if let Some(therun_username) = runner.get_therun_username() {
            if !therun_username.is_empty() {
                send_nonblocking!(
                    directory.therun_actor,
                    TheRunCommand,
                    AddRunner,
                    runner.clone()
                );
            }
        }
    }

    while let Some(msg) = rx.recv().await {
        match msg {
            RunnerRequest::Create(mut runner, rto) => {
                log::info!("Creating runner for person {}", runner.participant);
                match db.add_runner(&mut runner).await {
                    Ok(_) => {
                        if runner.get_therun_username().is_some()
                            && runner.get_therun_username().unwrap() != ""
                        {
                            send_nonblocking!(
                                directory.therun_actor,
                                TheRunCommand,
                                AddRunner,
                                runner.clone()
                            );
                        }
                        rto.reply(Ok(()));
                    }
                    Err(e) => rto.reply(Err(e)),
                }
            }
            RunnerRequest::Update(runner, rto) => {
                let old_runner = db.get_runner(runner.participant).await;
                match old_runner {
                    Ok(old_runner) => {
                        log::info!("Updating runner {}", runner.participant);
                        // Check for changes in TheRun.gg username
                        if old_runner.get_therun_username() != runner.get_therun_username() {
                            send_nonblocking!(
                                directory.therun_actor,
                                TheRunCommand,
                                RemoveRunner,
                                old_runner.clone()
                            );
                            if runner.get_therun_username().is_some()
                                && runner.get_therun_username().unwrap() != ""
                            {
                                send_nonblocking!(
                                    directory.therun_actor,
                                    TheRunCommand,
                                    AddRunner,
                                    runner.clone()
                                );
                            }
                        }

                        match db.update_runner(&runner).await {
                            Ok(_) => rto.reply(Ok(())),
                            Err(e) => {
                                log::error!(
                                    "Failed to update runner {}: {}",
                                    runner.participant,
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
                Ok(mut runner) => match runner.find_and_save_stream(&db, &settings).await {
                    Ok(changed) => rto.reply(Ok(changed)),
                    Err(e) => rto.reply(Err(e)),
                },
                Err(_) => rto.reply(Err(anyhow!("Runner {} not found", runner))),
            },
            RunnerRequest::Delete(id, rto) => match db.get_events_for_runner(id).await {
                Err(e) => rto.reply(Err(e)),
                Ok(events) => {
                    let runner = db.get_runner(id).await.expect("Failed to get runner");

                    log::info!("Deleting runner {}", id);
                    if events.is_empty() {
                        if runner.get_therun_username().is_some() {
                            send_nonblocking!(
                                directory.therun_actor,
                                TheRunCommand,
                                RemoveRunner,
                                runner.clone()
                            );
                        }
                        rto.reply(db.delete_runner(id).await)
                    } else {
                        rto.reply(Err(anyhow!(
                            "Cannot delete runner {} as they are associated with
                             the following events: {:?}",
                            runner.participant_data.name,
                            events.to_vec()
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
    /// The participant this runner information relates to
    pub participant: ParticipantId,

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

    /// Twitch stream volume in percent
    pub stream_volume_percent: u32,

    #[sqlx(skip)]
    /// A map of streamlink resolutions (including `best`) to .m3u8 links.
    /// This is None if streamlink does not return values.
    pub stream_urls: HashMap<String, String>,

    #[sqlx(skip)]
    #[serde(skip)]
    pub participant_data: Participant,
}

impl Runner {
    pub fn get_therun_username(&self) -> Option<String> {
        self.therun.clone()
    }

    /// Determine the stream link for this player.
    pub fn get_stream(&self) -> String {
        match &self.stream {
            Some(stream) => {
                if stream.starts_with("https://") || stream.starts_with("http://") {
                    stream.to_string()
                } else {
                    format!("https://twitch.tv/{}", stream)
                }
            }
            None => format!("https://twitch.tv/{}", self.participant_data.name),
        }
    }

    /// Returns a .m3u8 link corresponding to the current players' stream.
    /// Returns if the link changed.
    pub fn find_stream(&mut self, settings: &Settings) -> anyhow::Result<bool> {
        let mut command = process::Command::new("streamlink");

        if let Some(twitch_key) = &settings.twitch_client_id {
            command.arg("--twitch-api-header=Authorization=OAuth ".to_owned() + twitch_key);
        }

        let output = command
            .arg("-j")
            .arg(self.get_stream())
            .output()
            .map_err(|e| {
                anyhow!(
                    "Failed to acquire stream for {}: {:?}",
                    &self.participant_data.name,
                    e
                )
            })?;

        let json = std::str::from_utf8(output.stdout.as_slice())?;

        let parsed_json: Value = serde_json::from_str(json)?;

        if parsed_json.get("error").is_some() {
            Err(Error::FailedStreamAcq(
                self.participant_data.name.clone(),
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

    /// Update the stream link in the database for this player.
    /// Returns if the link changed.
    async fn find_and_save_stream(
        &mut self,
        db: &ProjectDb,
        settings: &Settings,
    ) -> anyhow::Result<bool> {
        match self.find_stream(settings) {
            Ok(changed) => {
                if changed {
                    log::info!("Updating stream url for {}", self.participant_data.name);
                    db.update_runner(self).await?;
                } else {
                    log::info!(
                        "Stream url for {} is up to date",
                        self.participant_data.name
                    );
                }
                Ok(changed)
            }
            Err(e) => {
                log::warn!(
                    "Failed to find stream for {}: {}",
                    self.participant_data.name,
                    e
                );
                db.update_runner(self).await?;
                Err(e)?
            }
        }
    }

    /// Determine the correct streamlink URL for the given
    /// element width and desired FPS.
    pub fn calculate_best_url(&self, width: u32, stream_fps: u32) -> Option<(String, u32)> {
        if let Some(ref stream) = self.override_stream_url {
            let stream = stream.trim().to_string();
            let url = warp::http::Uri::from_maybe_shared(stream.clone());
            if url.is_ok() {
                return Some((stream.to_owned(), 1080));
            }
        }

        if self.stream_urls.is_empty() {
            return None;
        }

        let desire_60fps = stream_fps >= 30;

        let mut closest_title = "best";
        let mut closest_url = self.stream_urls["best"].clone();
        let mut closest_width = 1080;
        let mut closest_diff = u32::MAX;

        for (res, url) in self.stream_urls.iter() {
            let elements: Vec<&str> = res.split('p').collect();
            let res_width = elements[0].parse::<u32>();

            if let Ok(stream_width) = res_width {
                let stream_fps = if elements.len() > 1 && elements[1].contains("60") {
                    60
                } else {
                    30
                };

                let stream_diff = (stream_width as i32 - width as i32).unsigned_abs();
                if stream_width >= width && stream_diff <= closest_diff {
                    if stream_diff == closest_diff {
                        if (desire_60fps && stream_fps == 60) || (!desire_60fps && stream_fps == 30)
                        {
                            closest_title = res;
                            closest_url.clone_from(url);
                        }
                    } else {
                        closest_title = res;
                        closest_url.clone_from(url);
                        closest_width = stream_width;
                        closest_diff = stream_width - width;
                    }
                }
            }
        }

        log::debug!("Selected stream URL {} for width {}", closest_title, width);

        Some((closest_url, closest_width))
    }
}
