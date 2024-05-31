use std::sync::Arc;

use anyhow::anyhow;
use sqlx::prelude::FromRow;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    core::{db::ProjectDb, runner::Runner},
    integrations::obs::ObsCommand,
    send_message, ActorRef, ObsActor, Rto,
};

#[derive(PartialEq, Debug, FromRow)]
pub struct StreamState {
    pub event: String,
    pub obs_host: String,
    /// Semicolon-separated list of commentators
    pub active_commentators: String,
    pub ignored_commentators: String,
    pub audible_runner: Option<String>,
    pub requested_layout: Option<String>,

    #[sqlx(skip)]
    pub stream_runners: Vec<Runner>,
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

#[derive(Debug, Clone)]
pub enum StreamCommand {
    Toggle(String),
    Swap(String, String),
    SetPlayers(Vec<String>),
    SetAudibleRunner(String),
    Refresh(Vec<String>),
    Layout(String),
    Commentary(Vec<String>),
    CommentaryIgnore(Vec<String>),
}

/// Requests that can be sent to a StateActor
pub enum StreamRequest {
    CreateStream(String, String, Rto<()>),
    UpdateStream(Option<String>, StreamCommand, Rto<()>),
    DeleteStream(String, Rto<()>),
}

pub type StreamActor = ActorRef<StreamRequest>;

/// Elements of ProjectState that were modified during a state change.
#[derive(PartialEq, Debug)]
pub enum ModifiedStreamState {
    PlayerView(String),
    PlayerStream(String),
    Layout,
    Commentary,
}

/// Verify the name of a streamed event.
///
/// This function returns the contents of `event_name`,
/// or attempts to get the name of the single active
/// stream if `event_name` is `None`
pub async fn validate_streamed_event_name(
    db: &ProjectDb,
    event_name: Option<String>,
) -> anyhow::Result<String> {
    if let Some(event_id) = event_name {
        match db.get_stream(&event_id).await {
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

pub async fn run_state_manager(
    db: Arc<ProjectDb>,
    mut rx: UnboundedReceiver<StreamRequest>,
    obs: ObsActor,
) -> Result<(), anyhow::Error> {
    log::debug!("Started stream state manager");
    while let Some(msg) = rx.recv().await {
        match msg {
            StreamRequest::UpdateStream(event_name, cmd, rto) => {
                log::info!("Applying command {:?}", cmd);

                let event_name = match validate_streamed_event_name(&db, event_name).await {
                    Ok(name) => name,
                    Err(err) => {
                        rto.reply(Err(err));
                        continue;
                    }
                };

                match db.get_stream(&event_name).await {
                    Ok(mut stream) => match stream.apply_cmd(&db, &cmd).await {
                        Ok(mods) => {
                            db.save_stream(&stream).await?;
                            rto.reply(send_message!(
                                obs,
                                ObsCommand,
                                UpdateState,
                                event_name,
                                mods
                            ));
                        }
                        Err(e) => rto.reply(Err(e)),
                    },
                    Err(e) => {
                        rto.reply(Err(anyhow!(
                            "No stream found for event '{}': {:?}.",
                            event_name,
                            e
                        )));
                    }
                }
            }
            StreamRequest::CreateStream(event, host, rto) => {
                log::debug!("Creating stream for {}", event);
                if let Ok(host_in_use) = db.is_host_in_use(&host).await {
                    if host_in_use {
                        rto.reply(Err(anyhow!(
                            "Host '{}' is already in use, cannot create stream for event {}.",
                            host,
                            event
                        )));
                        continue;
                    }
                }

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
            StreamRequest::DeleteStream(event, rto) => {
                rto.reply(db.delete_stream(&event).await);
            }
        }
    }

    Ok(())
}

impl StreamState {
    /// Apply a command onto the current project state, providing a new state.
    pub async fn apply_cmd(
        &mut self,
        db: &ProjectDb,
        cmd: &StreamCommand,
    ) -> anyhow::Result<Vec<ModifiedStreamState>> {
        let mut modifications = Vec::<ModifiedStreamState>::new();
        match cmd {
            StreamCommand::Toggle(player_name) => {
                let mut player = db.find_runner(player_name).await?;

                modifications.push(ModifiedStreamState::PlayerView(player.name.to_string()));
                if self.is_active(&player) {
                    self.stream_runners.retain(|p| &p.name != &player.name);
                } else {
                    self.stream_runners.push(player.clone());
                    if player.find_and_save_stream(db).await? {
                        modifications
                            .push(ModifiedStreamState::PlayerStream(player.name.to_string()));
                    }
                }

                modifications.push(ModifiedStreamState::Layout);
            }
            StreamCommand::Swap(p1_name, p2_name) => {
                let mut p1 = db.find_runner(p1_name).await?;
                let mut p2 = db.find_runner(p2_name).await?;

                let pos_p1 = self.stream_runners.iter().position(|p| &p.name == &p1.name);
                let pos_p2 = self.stream_runners.iter().position(|p| &p.name == &p2.name);

                if let (Some(pos_p1), Some(pos_p2)) = (pos_p1, pos_p2) {
                    self.stream_runners.swap(pos_p1, pos_p2);
                } else if let Some(pos_p1) = pos_p1 {
                    self.stream_runners.insert(pos_p1, p2.clone());
                    self.stream_runners.remove(pos_p1 + 1);

                    modifications.push(ModifiedStreamState::PlayerView(p2.name.to_string()));
                    if p2.find_and_save_stream(db).await? {
                        modifications.push(ModifiedStreamState::PlayerStream(p2.name.to_string()));
                    }
                } else if let Some(pos_p2) = pos_p2 {
                    self.stream_runners.insert(pos_p2, p2.clone());
                    self.stream_runners.remove(pos_p2 + 1);

                    modifications.push(ModifiedStreamState::PlayerView(p1.name.to_string()));
                    if p1.find_and_save_stream(db).await? {
                        modifications.push(ModifiedStreamState::PlayerStream(p1.name.to_string()));
                    }
                }
            }
            StreamCommand::SetPlayers(players_strs) => {
                let mut players = vec![];
                for player_str in players_strs {
                    let player = db.find_runner(player_str).await?;
                    players.push(player);
                }

                for player in &mut players {
                    if !self.is_active(player) && player.find_and_save_stream(db).await? {
                        modifications
                            .push(ModifiedStreamState::PlayerStream(player.name.to_string()));
                    }
                }

                if self.stream_runners.len() != players.len() {
                    modifications.push(ModifiedStreamState::Layout);
                }

                self.stream_runners.clear();

                for player in players {
                    modifications.push(ModifiedStreamState::PlayerView(player.name.to_string()));
                    self.stream_runners.push(player);
                }
            }
            StreamCommand::SetAudibleRunner(runner) => {
                let _ = db.find_runner(runner).await?;

                self.audible_runner = Some(runner.to_string());
            }
            StreamCommand::Refresh(players_strs) if !players_strs.is_empty() => {
                let mut players = vec![];
                for player_str in players_strs {
                    let player = db.find_runner(player_str).await?;
                    players.push(player);
                }

                for mut player in players {
                    player.find_and_save_stream(db).await?;
                    modifications.push(ModifiedStreamState::PlayerStream(player.name.to_string()));
                }
            }
            StreamCommand::Refresh(_) => {
                for player in &mut self.stream_runners {
                    player.find_and_save_stream(db).await?;
                    modifications.push(ModifiedStreamState::PlayerStream(player.name.to_string()));
                }
            }
            StreamCommand::Layout(layout_str) => {
                self.requested_layout = Some(layout_str.clone());
                modifications.push(ModifiedStreamState::Layout);
            }
            StreamCommand::CommentaryIgnore(ignored) => {
                if !self.ignored_commentators.eq(&ignored.join(";")) {
                    modifications.push(ModifiedStreamState::Commentary);
                    self.ignored_commentators = ignored.join(";");
                }
            }
            StreamCommand::Commentary(comms) => {
                if !self.active_commentators.eq(&comms.join(";")) {
                    modifications.push(ModifiedStreamState::Commentary);
                    self.active_commentators = comms.join(";");
                }
            }
        };

        Ok(modifications)
    }

    /// Returns if the provided player is active and visible onscreen
    pub fn is_active(&self, runner: &Runner) -> bool {
        self.stream_runners.iter().any(|a| &a.name == &runner.name)
    }
}
