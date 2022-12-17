use std::{collections::HashMap, process, time::Duration};
use clap::ValueEnum;
use obws::requests::scene_items::{SetTransform, SceneItemTransform, Position};
use serde::{Serialize, Deserialize};
use serde_json::Value;

use crate::{user::Player, cmd::Command, obs::{Layout, ObsConfiguration}, error::Error};

/// A type used to determine the project type
#[derive(PartialEq, Debug, Serialize, Deserialize, Clone, ValueEnum)]
pub enum ProjectType {
    Marathon
}

/// Contains the immutable state of a project. 
#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub struct Project {
    pub kind: ProjectType,
    pub players: Vec<Player>
}

/// Holds state related to a specific project type
#[derive(PartialEq, Debug, Clone)]
pub enum ProjectTypeState<'a> {
    MarathonState{ runner_times: HashMap<&'a Player, Duration> }
}

/// Serializer utility type for ProjectTypeState
#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub enum ProjectTypeStateSerializer {
    MarathonState{ runner_times: HashMap<String, Duration> }
}



/// Holds the runtime project state for an active project.
#[derive(PartialEq, Debug, Clone)]
pub struct ProjectState<'a> {
    pub running: bool,
    pub active_players: Vec<&'a Player>,
    pub streams: HashMap<&'a Player, String>,
    pub type_state: ProjectTypeState<'a>,
    pub layout: HashMap<u32, &'a Layout>
}

/// Serializizer utility type for ProjectState
/// Certain transient elements of a project, such as the .m3u8 streams in use, are not serialized.
#[derive(Serialize, Deserialize)]
pub struct ProjectStateSerializer {
    pub active_players: Vec<String>,
    pub type_state: ProjectTypeStateSerializer,
    pub layout: HashMap<u32, String>
}


impl Project {
    pub fn find_by_nick(&self, name: &str) -> Option<&Player> {
       self.players.iter().find(|p| p.name_match(name)) 
    }
}

pub fn find_stream(player: &Player) -> Result<String, Error> {
    let output = process::Command::new("streamlink")
        .arg("-Q")
        .arg("-j")
        .arg(&player.twitch)
        .output().map_err(|_| Error::StreamAcqError(player.name.clone(), "Failed to aquire stream.".to_owned()))?;

    let json = std::str::from_utf8(output.stdout.as_slice())
            .map_err(|_| Error::ParseError("Failed to decipher stream.".to_owned()))?;

    let parsed_json: Value = serde_json::from_str(json).unwrap();
    if parsed_json.get("error").is_some() {
        Err(Error::StreamAcqError(player.name.clone(), parsed_json["error"].to_string()))
    } else {
        println!("{}", parsed_json["streams"]["best"]["url"].to_string().replace("\"", ""));
        Ok(parsed_json["streams"]["best"]["url"].to_string().replace("\"", ""))
    }
}

impl<'a> ProjectState<'a> {
    pub fn apply_cmds(&self, cmds: &Vec<Command<'a>>) -> Result<ProjectState<'a>, Error> {
        let mut new_state = self.clone();

        for cmd in cmds {
            new_state = new_state.apply_cmd(cmd)?;
        }

        Ok(new_state)
    }

    pub fn apply_cmd(&self, cmd: &Command<'a>) -> Result<ProjectState<'a>, Error> {
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
            Command::SetPlayers(players) => {
                new_state.active_players.clear();
                for player in players {
                    new_state.active_players.push(player);
                    if self.running { new_state.streams.insert(player, find_stream(player)?); };
                }
                Ok(new_state)
            }
            Command::SetScore(player, time) => {
                match &mut new_state.type_state {
                    ProjectTypeState::MarathonState { runner_times } => {
                        runner_times.insert(player, time.to_owned());
                    },
                }

                Ok(new_state)
            },
            Command::Refresh(players) if self.running => {
                for player in players {
                    new_state.streams.insert(player, find_stream(&player)?);
                }
                Ok(new_state)
            }
            Command::Refresh(_) => Err(Error::ProjectStateError("Cannot refresh streams while project is inactive.".to_owned())),
            Command::Layout(count, layout) => {
                new_state.layout.insert(*count, layout);
                Ok(new_state)
            }
        }
    }

    pub async fn apply_layout(&self, obs_cfg: &ObsConfiguration, obs: &obws::Client) -> Result<(), Error> {
        let items = obs.scene_items().list(&obs_cfg.scene).await?;

        println!("{:?}", items);
        match self.layout.get(&self.active_players.len().try_into().unwrap()).map(|l| *l)
                .or_else(|| obs_cfg.layouts.values().find(|l| l.default.unwrap_or(false) && l.displays.len() == self.active_players.len()))
                .or_else(|| obs_cfg.layouts.values().find(|l| l.displays.len() == self.active_players.len())) {
            Some(layout) => {
                for (idx, view) in layout.displays.iter().enumerate() {
                    let stream_id = "stream_".to_string() + &idx.to_string();
                    let stream = items.iter().find(|item| item.source_name == stream_id).ok_or(Error::GeneralError(format!("Unknown source {}", stream_id)))?;

                    let stream_transform = SetTransform {
                        scene: &obs_cfg.scene,
                        item_id: stream.id,
                        transform: SceneItemTransform {
                            position: Some(Position {
                                x: Some(view.stream.x as f32),
                                y: Some(view.stream.y as f32),
                            }),
                            rotation: None,
                            scale: None,
                            alignment: None,
                            bounds: None,
                            crop: None,
                        },
                    };

                    obs.scene_items().set_transform(stream_transform).await?;
                }
                Ok(())
            }
            _ => Err(Error::LayoutError("No known layout for the current player count.".to_string())),
        }
    }

    pub fn to_save_state(&self) -> ProjectStateSerializer {
        ProjectStateSerializer {
            active_players: self.active_players.iter().map(|a| a.name.to_owned()).collect(),
            type_state: self.type_state.to_save_state(),
            layout: self.layout.iter()
                .map(|(k, v)| (k.to_owned(), v.name.to_owned()))
                .collect()
        }
    }

    pub fn from_save_state(save: &ProjectStateSerializer, project: &'a Project, obs_cfg: &'a ObsConfiguration) -> Result<ProjectState<'a>, Error> {
        Ok(ProjectState { 
            running: false, 
            active_players: save.active_players
                .iter()
                .map(|p| 
                    project.find_by_nick(p)
                        .ok_or(Error::ProjectLoadError(format!("Failed to find player {} when loading file", p))))
                .collect::<Result<Vec<&Player>, Error>>()?, 
            streams: HashMap::new(),
            type_state: ProjectTypeState::from_save_state(&save.type_state, project)?,
            layout: save.layout.iter()
                .map(|(k, v)| obs_cfg.layouts.get(v)
                    .ok_or(Error::ProjectLoadError(format!("Failed to find layout {}", k)))
                    .map(|l| (k.to_owned(), l)))
                .collect::<Result<HashMap<u32, &Layout>, Error>>()?
        })
    }
}

impl<'a> ProjectTypeState<'a> {
    pub fn to_save_state(&self) -> ProjectTypeStateSerializer {
        match self {
            ProjectTypeState::MarathonState { runner_times } => 
                ProjectTypeStateSerializer::MarathonState { 
                    runner_times: runner_times
                        .iter()
                        .map(|record| (record.0.name.to_owned(), record.1.to_owned()))
                        .collect() 
                },
        }
    }

    pub fn from_save_state(save: &ProjectTypeStateSerializer, project: &'a Project) -> Result<ProjectTypeState<'a>, Error> {
        match save {
            ProjectTypeStateSerializer::MarathonState { runner_times } => 
                runner_times
                    .iter()
                    .map(|(name, time)| 
                        project.find_by_nick(name)
                            .ok_or(Error::ProjectLoadError(format!("Failed to find player {} when loading file", name)))
                            .map(|usr| (usr, time.to_owned())))
                    .collect::<Result<Vec<(&Player, Duration)>, Error>>()
                    .map(|records| ProjectTypeState::MarathonState { runner_times: records.into_iter().collect() })
        }
    }
}
