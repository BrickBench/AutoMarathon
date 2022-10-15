use std::{collections::HashMap, fmt::Display, process, time::Duration};
use serde::{Serialize, Deserialize};
use serde_json::{Value};

use crate::{user::Player, cmd::Command};

#[derive(PartialEq, Debug)]
pub enum ProjectType {
    Marathon,
    Race
}

#[derive(PartialEq, Debug)]
pub struct Project {
    pub kind: ProjectType,
    pub players: Vec<Player>
}

#[derive(PartialEq, Debug)]
pub enum ProjectTypeState<'a> {
    MarathonRaceState{ runner_times: HashMap<&'a Player, Duration> }
}

#[derive(Serialize, Deserialize)]
pub struct ProjectSaveState {
    active_players: Vec<String>
}

#[derive(PartialEq, Debug, Clone)]
pub struct ProjectState<'a> {
    pub running: bool,
    pub active_players: Vec<&'a Player>,
    pub streams: HashMap<&'a Player, String>
}

#[derive(Debug)]
pub enum ProjectError {
    ProjectLoadError(String),
    StreamAcqError(String, String),
    ProjectStateError(String)
}

impl Display for ProjectError{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectError::StreamAcqError(name, msg) => write!(f, "Failed to acquire stream for {}: {}", name, msg),
            ProjectError::ProjectStateError(msg) => write!(f, "Invalid state: {}", msg),    
            ProjectError::ProjectLoadError(msg) => write!(f, "Failed to load project: {}", msg)
        }
    }
}

impl Project {
    pub fn find_by_nick(&self, name: &str) -> Option<&Player> {
       self.players.iter().find(|p| p.name_match(name)) 
    }
}

pub fn find_stream(player: &Player) -> Result<String, ProjectError> {
    let output = process::Command::new("streamlink")
        .arg("-Q")
        .arg("-j")
        .arg("https://twitch.tv/summit1g")
        .output().map_err(|_| ProjectError::StreamAcqError(player.name.clone(), "Failed to aquire stream.".to_owned()))?;

    let json = std::str::from_utf8(output.stdout.as_slice())
            .map_err(|_| ProjectError::StreamAcqError(player.name.clone(), "Failed to decipher stream.".to_owned()))?;

    let parsed_json: Value = serde_json::from_str(json).unwrap();
    if parsed_json.get("error").is_some() {
        Err(ProjectError::StreamAcqError(player.name.clone(), parsed_json["error"].to_string()))
    } else {
        println!("{}", parsed_json["streams"]["best"]["url"].to_string().replace("\"", ""));
        Ok(parsed_json["streams"]["best"]["url"].to_string().replace("\"", ""))
    }
}

impl<'a> ProjectState<'a> {
    pub fn apply_cmd(&self, cmd: &Command<'a>) -> Result<ProjectState<'a>, ProjectError> {
        let mut new_state = self.clone();
        match cmd {
            Command::Toggle(player) => {
                if self.active_players.contains(player) {
                    new_state.active_players.retain(|p| p != player);
                } else {
                    new_state.active_players.push(player);
                    if self.running { new_state.streams.insert(player, find_stream(player)?); }
                }

                Ok(new_state)
            }
            Command::Swap(p1, p2) => {
                let pos_p1 = self.active_players.iter().position(|p| p == p1);
                let pos_p2 = self.active_players.iter().position(|p| p == p2);

                if pos_p1.is_some() && pos_p2.is_some() {
                    new_state.active_players.swap(pos_p1.unwrap(), pos_p2.unwrap());
                } else if pos_p1.is_some() {
                    new_state.active_players.insert(pos_p1.unwrap(), p2);
                    new_state.active_players.remove(pos_p1.unwrap() + 1);

                    if self.running { new_state.streams.insert(p2, find_stream(p2)?); };
                } else if pos_p2.is_some() {
                    new_state.active_players.insert(pos_p2.unwrap(), p1);
                    new_state.active_players.remove(pos_p2.unwrap() + 1);

                    if self.running { new_state.streams.insert(p1, find_stream(p1)?); };
                }

                Ok(new_state)
            },
            Command::Set(players) => {
                new_state.active_players.clear();
                for player in players {
                    new_state.active_players.push(player);
                    if self.running { new_state.streams.insert(player, find_stream(player)?); };
                }
                Ok(new_state)
            }
            Command::Refresh(players) if self.running => {
                for player in players {
                    new_state.streams.insert(player, find_stream(player)?);
                }
                Ok(new_state)
            }
            Command::Refresh(_) => Err(ProjectError::ProjectStateError("Cannot refresh streams while project is inactive.".to_owned()))
        }
    }

    fn get_save_state(&self) -> ProjectSaveState {
        ProjectSaveState {
            active_players: self.active_players.iter().map(|a| a.name.to_owned()).collect()
        }
    }

    fn from_save_state(save: &ProjectSaveState, project: &'a Project) -> ProjectState<'a> {
        ProjectState { 
            running: false, 
            active_players: save.active_players.map(|p| proj), 
            streams: HashMap::new() 
        }
    }
}
