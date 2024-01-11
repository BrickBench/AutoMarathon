use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{collections::HashMap, path::PathBuf, process, sync::Arc, u64};
use tokio::{fs, sync::mpsc::UnboundedReceiver};

use crate::{
    error::Error,
    integrations::therun::Run,
    obs::{LayoutFile, ObsCommand},
    player::Player,
    project::{Project, ProjectTypeSettings},
    ActorRef, ObsActor, Rto,
};

/// Holds the runtime project state for an active project.
#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub struct ProjectState {
    pub running: bool,
    pub active_players: Vec<String>,
    active_commentators: Vec<String>,
    ignored_commentators: Vec<String>,
    pub streams: HashMap<String, String>,
    pub event_fields: HashMap<String, String>,
    pub player_fields: HashMap<String, HashMap<String, String>>,
    pub layouts_by_count: HashMap<u32, String>,
    pub event_state: ProjectTypeState,
}

/// Project type dependent state
#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProjectTypeState {
    Marathon { event_pbs: HashMap<String, f64> },
    Relay(RelayState),
}

/// State specific to a relay project
#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub struct RelayState {
    pub start_date_time: u64,
    pub end_date_time: Option<u64>,
    pub runner_start_date_time: HashMap<String, u64>,
    pub runner_split_times: HashMap<String, Vec<(String, Option<f64>)>>,
    pub runner_end_time: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub enum Command {
    Toggle(String),
    Swap(String, String),
    SetPlayers(Vec<String>),
    Refresh(Vec<String>),
    Layout(String),
    Commentary(Vec<String>),
    CommentaryIgnore(Vec<String>),
    SetRelayStartTime(u64),
    SetRelayRunTime(String, String),
    SetRelayEndTime(Option<u64>),
    UpdateLiveData(String, Run),
}

/// Requests that can be sent to a StateActor
pub enum StateRequest {
    UpdateState(Command, Rto<()>),
    GetState(Rto<ProjectState>),
}

pub type StateActor = ActorRef<StateRequest>;

/// Elements of ProjectState that were modified during a state change.
#[derive(PartialEq)]
pub enum ModifiedState {
    PlayerView(String),
    PlayerStream(String),
    Layout,
    Commentary,
}

/// Returns a .m3u8 link corresponding to the current players' stream.
pub fn find_stream(player: &Player) -> Result<String, anyhow::Error> {
    let output = process::Command::new("streamlink")
        .arg("-Q")
        .arg("-j")
        .arg(&player.get_stream())
        .output()
        .map_err(|_| {
            Error::FailedStreamAcq(player.name.clone(), "Failed to aquire stream.".to_owned())
        })?;

    let json = std::str::from_utf8(output.stdout.as_slice())?;

    let parsed_json: Value = serde_json::from_str(json).expect("Unable to parse streamlink output");
    if parsed_json.get("error").is_some() {
        Err(Error::FailedStreamAcq(
            player.name.clone(),
            parsed_json["error"].to_string(),
        ))?
    } else {
        Ok(parsed_json["streams"]["best"]["url"]
            .to_string()
            .replace('\"', ""))
    }
}

pub async fn run_state_manager(
    project: Arc<Project>,
    layouts: Arc<LayoutFile>,
    state_file: PathBuf,
    mut rx: UnboundedReceiver<StateRequest>,
    obs: ObsActor,
) -> Result<(), anyhow::Error> {
    // Load or create project state
    let mut state = ProjectState::load_project_state(&state_file, &project, &layouts).await?;

    while let Some(msg) = rx.recv().await {
        match msg {
            StateRequest::UpdateState(cmd, rto) => {
                match state.apply_cmd(&cmd, &project, &layouts) {
                    Ok((new_state, mods)) => {
                        let (tx, rrx) = Rto::<()>::new();
                        obs.send(ObsCommand::UpdateObs(new_state.clone(), mods, tx));

                        let obs_resp = rrx.await.unwrap();
                        if obs_resp.is_ok() {
                            println!("Saving");
                            state = new_state;
                            let state_save = serde_json::to_string_pretty(&state).unwrap();
                            fs::write(&state_file, state_save)
                                .await
                                .expect("Failed to save file.");
                        }

                        rto.reply(obs_resp);
                    }
                    Err(e) => rto.reply(Err(e)),
                }
            }
            StateRequest::GetState(rto) => rto.reply(Ok(state.clone())),
        }
    }

    Ok(())
}

impl ProjectState {
    /// Apply a command onto the current project state, providing a new state.
    pub fn apply_cmd(
        &self,
        cmd: &Command,
        project: &Project,
        layouts: &LayoutFile,
    ) -> Result<(ProjectState, Vec<ModifiedState>), anyhow::Error> {
        let mut new_state = self.clone();
        let mut modifications = Vec::<ModifiedState>::new();
        match cmd {
            Command::Toggle(player_name) => {
                let player = project.find_player(player_name)?;

                modifications.push(ModifiedState::PlayerView(player.name.to_string()));
                if self.active_players.contains(&player.name) {
                    new_state.active_players.retain(|p| p != &player.name);
                } else {
                    new_state.active_players.push(player.name.to_string());
                    if new_state.update_stream(player)? {
                        modifications.push(ModifiedState::PlayerStream(player.name.to_string()));
                    }
                }

                modifications.push(ModifiedState::Layout);
            }
            Command::Swap(p1_name, p2_name) => {
                let p1 = project.find_player(p1_name)?;
                let p2 = project.find_player(p2_name)?;

                let pos_p1 = self.active_players.iter().position(|p| p == &p1.name);
                let pos_p2 = self.active_players.iter().position(|p| p == &p2.name);

                modifications.push(ModifiedState::PlayerView(p1.name.to_string()));
                modifications.push(ModifiedState::PlayerView(p2.name.to_string()));

                if let (Some(pos_p1), Some(pos_p2)) = (pos_p1, pos_p2) {
                    new_state.active_players.swap(pos_p1, pos_p2);
                } else if let Some(pos_p1) = pos_p1 {
                    new_state.active_players.insert(pos_p1, p2.name.to_string());
                    new_state.active_players.remove(pos_p1 + 1);

                    if new_state.update_stream(p2)? {
                        modifications.push(ModifiedState::PlayerStream(p2.name.to_string()));
                    }
                } else if let Some(pos_p2) = pos_p2 {
                    new_state.active_players.insert(pos_p2, p1.name.to_string());
                    new_state.active_players.remove(pos_p2 + 1);

                    if new_state.update_stream(p1)? {
                        modifications.push(ModifiedState::PlayerStream(p1.name.to_string()));
                    }
                }
            }
            Command::SetPlayers(players_strs) => {
                let players = players_strs
                    .iter()
                    .map(|e| project.find_player(e))
                    .collect::<Result<Vec<&Player>, anyhow::Error>>()?;

                for player in &players {
                    if !new_state.active_players.contains(&player.name)
                        && new_state.update_stream(player)?
                    {
                        modifications.push(ModifiedState::PlayerStream(player.name.to_string()));
                    }
                }

                if new_state.active_players.len() != players.len() {
                    modifications.push(ModifiedState::Layout);
                }

                new_state.active_players.clear();

                for player in players {
                    modifications.push(ModifiedState::PlayerView(player.name.to_string()));
                    new_state.active_players.push(player.name.to_string());
                }
            }
            Command::Refresh(players_strs) if !players_strs.is_empty() => {
                let players = players_strs
                    .iter()
                    .map(|e| project.find_player(e))
                    .collect::<Result<Vec<&Player>, anyhow::Error>>()?;

                for player in players {
                    if new_state.active_players.contains(&player.name) {
                        new_state.update_stream(player)?;
                        modifications.push(ModifiedState::PlayerStream(player.name.to_string()));
                    }
                }
            }
            Command::Refresh(_) => {
                for player in &project.players {
                    if new_state.active_players.contains(&player.name) {
                        new_state.update_stream(player)?;
                        modifications.push(ModifiedState::PlayerStream(player.name.to_string()));
                    }
                }
            }
            Command::Layout(layout_str) => {
                let layout = layouts.get_layout_by_name(layout_str)?;

                modifications.push(ModifiedState::Layout);
                new_state
                    .layouts_by_count
                    .insert(layout.players.try_into().unwrap(), layout_str.to_string());
            }
            Command::CommentaryIgnore(ignored) => {
                if !new_state.ignored_commentators.eq(ignored) {
                    modifications.push(ModifiedState::Commentary);
                    new_state.ignored_commentators = ignored.clone();
                }
            }
            Command::Commentary(comms) => {
                if !new_state.active_commentators.eq(comms) {
                    modifications.push(ModifiedState::Commentary);
                    new_state.active_commentators = comms.clone();
                }
            }
            Command::UpdateLiveData(runner, run) => {
                if let ProjectTypeSettings::Relay { teams: _ } = &project.project_type {
                    if let ProjectTypeState::Relay(relay_state) = &mut new_state.event_state {
                        println!("{}", runner);
                        if self.is_active(project.find_player(runner).unwrap()) {
                            relay_state
                                .runner_start_date_time
                                .insert(runner.to_string(), run.startedAt.parse::<u64>().unwrap());

                            let splits_vec = run
                                .splits
                                .iter()
                                .map(|s| (s.name.clone(), s.splitTime))
                                .collect::<_>();

                            relay_state
                                .runner_split_times
                                .insert(runner.to_string(), splits_vec);
                        }
                    }
                }
            }
            Command::SetRelayStartTime(time) => {
                if let ProjectTypeState::Relay(relay_state) = &mut new_state.event_state {
                    relay_state.start_date_time = *time;
                }
            }
            Command::SetRelayEndTime(time) => {
                if let ProjectTypeState::Relay(relay_state) = &mut new_state.event_state {
                    relay_state.end_date_time = time.clone();
                }
            }
            Command::SetRelayRunTime(player, time) => {
                if let ProjectTypeState::Relay(relay_state) = &mut new_state.event_state {
                    relay_state
                        .runner_end_time
                        .insert(player.to_string(), time.clone());
                }
            }
        };

        Ok((new_state, modifications))
    }

    /// Returns all active commentators
    pub fn get_commentators(&self) -> Vec<String> {
        let mut commentators = self.active_commentators.clone();
        commentators.retain(|c| !&self.ignored_commentators.contains(c));

        commentators
    }

    /// Updates the provided player's stream in this state, returning if the stream changed.
    pub fn update_stream(&mut self, player: &Player) -> Result<bool, anyhow::Error> {
        let new_stream = find_stream(player)?;
        log::debug!("Acquired new stream for {}", player.name);

        self.streams.insert(player.name.to_string(), new_stream);
        Ok(true)
    }

    /// Returns if the provided player is active and visible onscreen
    pub fn is_active(&self, player: &Player) -> bool {
        self.active_players.iter().any(|a| a == &player.name)
    }

    /// Load a project state from a file, creating a default state if the file does not exist 
    pub async fn load_project_state(
        path: &PathBuf,
        project: &Project,
        layouts: &LayoutFile,
    ) -> Result<ProjectState, anyhow::Error> {
        let event_state_default = match &project.project_type {
            crate::project::ProjectTypeSettings::Marathon => ProjectTypeState::Marathon {
                event_pbs: HashMap::new(),
            },
            crate::project::ProjectTypeSettings::Relay { teams } => {
                ProjectTypeState::Relay(RelayState {
                    start_date_time: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                    end_date_time: None,
                    runner_start_date_time: HashMap::new(),
                    runner_split_times: HashMap::new(),
                    runner_end_time: HashMap::new(),
                })
            }
        };

        match tokio::fs::read_to_string(path).await {
            Ok(state_str) => {
                let state = serde_json::from_str::<ProjectState>(&state_str)?;

                state.verify_project_state(project, layouts)?;

                Ok(state)
            }
            Err(_) => Ok({
                ProjectState {
                    running: false,
                    active_players: vec![],
                    active_commentators: vec![],
                    ignored_commentators: vec![],
                    streams: HashMap::new(),
                    event_fields: HashMap::new(),
                    player_fields: HashMap::new(),
                    layouts_by_count: HashMap::new(),
                    event_state: event_state_default,
                }
            }),
        }
    }

    /// Verify that the provided project state works for the given project and layouts
    fn verify_project_state(
        &self,
        project: &Project,
        layouts: &LayoutFile,
    ) -> Result<(), anyhow::Error> {
        for player in &self.active_players {
            if project.players.iter().all(|p| &p.name != player) {
                Err(Error::UnknownPlayer(player.to_owned()))?;
            }
        }

        for layout in self.layouts_by_count.values() {
            if layouts.layouts.iter().all(|p| &p.name != layout) {
                Err(Error::UnknownLayout(layout.to_owned()))?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
struct RelayPosition {
    runner: usize,
    split: isize,
}

impl RelayState {
    pub fn calculate_deltas_to_leader(&self, project: &Project) -> HashMap<String, u64> {
        if let ProjectTypeSettings::Relay { teams } = &project.project_type {
            let first_place_splits = teams
                .iter()
                .map(|t| {
                    (t.0, {
                        let relaypos = self.get_latest_relay_split(t.1);
                        relaypos
                    })
                })
                .max_by_key(|p| p.1.clone());

            match first_place_splits {
                Some(first) => {
                    let fast_team = teams.get(first.0).unwrap();
                    let mut result_map = HashMap::new();

                    for team in teams.keys() {
                        if team == first.0 {
                            if !result_map.contains_key(team) {
                                result_map.insert(team.to_owned(), 0);
                            }
                        } else {
                            let slow_team = teams.get(team).unwrap();
                            let slow_latest = self.get_latest_relay_split(slow_team);

                            if slow_latest == first.1 {
                                println!("Sameo");
                                let fast_time =
                                    self.get_time_at_relay_split(fast_team, &slow_latest) as i128;
                                let slow_time =
                                    self.get_time_at_relay_split(slow_team, &slow_latest) as i128;

                                println!("{:?} {:?}", fast_time, slow_time);
                                let diff = slow_time - fast_time;
                                if diff >= 0 {
                                    result_map.insert(team.to_owned(), diff as u64);
                                } else {
                                    result_map.insert(team.to_owned(), 0);
                                    result_map.insert(first.0.to_owned(), (-diff) as u64);
                                }
                            } else {
                                let diff =
                                    self.get_delta_at_split_to_timer(fast_team, &slow_latest);

                                result_map.insert(team.to_owned(), diff);
                            };
                        }
                    }

                    println!("Results: {:?}", result_map);
                    result_map
                }
                None => HashMap::new(),
            }
        } else {
            HashMap::new()
        }
    }

    /// Returns the last-completed split for the given team, in (runner, split) format.
    /// If the runner is on their first split, it will return split -1.
    fn get_latest_relay_split(&self, team: &[String]) -> RelayPosition {
        let mut last_split: isize = 0;
        let mut last_runner: usize = 0;
        for (runner_idx, runner) in team.iter().enumerate() {
            if self.runner_start_date_time.contains_key(runner) {
                last_runner = runner_idx;
                for (split_idx, (_split, time)) in self
                    .runner_split_times
                    .get(runner)
                    .unwrap()
                    .iter()
                    .enumerate()
                {
                    if time.is_none() {
                        return RelayPosition {
                            runner: runner_idx,
                            split: split_idx as isize,
                        };
                    }

                    last_split = split_idx as isize;
                }
            }
        }

        return RelayPosition {
            runner: last_runner,
            split: last_split,
        };
    }

    fn get_delta_at_split_to_timer(
        &self,
        winning_team: &[String],
        losing_pos: &RelayPosition,
    ) -> u64 {
        let winning_losing_pos_time = self.get_time_at_relay_split(winning_team, losing_pos);
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        current_time - winning_losing_pos_time
    }

    /// Returns the epoch time since the start of the relay at which the given split ended.
    fn get_time_at_relay_split(&self, team: &[String], relay_pos: &RelayPosition) -> u64 {
        let runner_name = &team[relay_pos.runner];
        let start_time = *self
            .runner_start_date_time
            .get(runner_name)
            .unwrap_or(&self.start_date_time);

        if relay_pos.split == 0 {
            return start_time;
        } else {
            let splits = self
                .runner_split_times
                .get(runner_name)
                .cloned()
                .unwrap_or(vec![]);

            start_time
                + splits
                    .get(relay_pos.split as usize - 1)
                    .and_then(|m| m.1)
                    .unwrap_or(0.0) as u64
        }
    }
}
