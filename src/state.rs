use clap::ValueEnum;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, process};

use crate::{
    cmd::Command,
    error::Error,
    obs::{Layout, LayoutFile},
    player::Player,
};

/// Contains the immutable state of a project.
#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub struct Project {
    pub template: ProjectTemplate,
    pub integrations: Vec<Integration>,
    pub extra_fields: Vec<String>,
    pub players: Vec<Player>,
}

/// A type used to determine the project type
#[derive(PartialEq, Debug, Serialize, Deserialize, Clone, ValueEnum)]
pub enum ProjectTemplate {
    Marathon,
}

/// A type used to determine the project type
#[derive(PartialEq, Debug, Serialize, Deserialize, Clone, ValueEnum)]
pub enum Integration {
    Discord,
    TheRun,
}

/// Holds the runtime project state for an active project.
#[derive(PartialEq, Debug, Clone)]
pub struct ProjectState<'a> {
    pub running: bool,
    pub active_players: Vec<&'a Player>,
    pub active_commentators: Vec<String>,
    pub ignored_commentators: Vec<String>,
    pub streams: HashMap<&'a Player, String>,
    pub player_fields: FieldState<'a>,
    pub layouts_by_count: HashMap<u32, &'a Layout>,
}

/// Holds state related to a specific project type
#[derive(PartialEq, Debug, Clone)]
pub struct FieldState<'a> {
    player_fields: HashMap<&'a Player, HashMap<String, String>>,
}

/// Serializer utility type for ProjectState
/// Certain transient elements of a project, such as the .m3u8 streams in use, are not serialized.
#[derive(Serialize, Deserialize, Debug)]
pub struct ProjectStateSerializer {
    pub active_players: Vec<String>,
    pub active_commentators: Vec<String>,
    pub ignored_commentators: Vec<String>,
    pub player_fields: FieldStateSerializer,
    pub layouts_by_count: HashMap<u32, String>,
}

/// Holds state related to a specific project type
#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub struct FieldStateSerializer {
    pub player_fields: HashMap<String, HashMap<String, String>>,
}

impl Project {
    pub fn find_by_nick(&self, name: &str) -> Option<&Player> {
        self.players.iter().find(|p| p.name_match(name))
    }

    pub fn find_or_err(&self, user: &'_ str) -> Result<&Player, Error> {
        self 
            .find_by_nick(user)
            .ok_or(Error::PlayerError(user.to_owned()))
    }
}

/// Elements of ProjectState that were modified during a state change.
#[derive(PartialEq)]
pub enum ModifiedState<'a> {
    PlayerView(&'a Player),
    PlayerStream(&'a Player),
    Layout,
    Commentary,
    PlayerFields,
}

/// Returns a .m3u8 link corresponding to the current players' stream.
pub fn find_stream(player: &Player) -> Result<String, Error> {
    let output = process::Command::new("streamlink")
        .arg("-Q")
        .arg("-j")
        .arg(&player.stream)
        .output()
        .map_err(|_| {
            Error::StreamAcqError(player.name.clone(), "Failed to aquire stream.".to_owned())
        })?;

    let json = std::str::from_utf8(output.stdout.as_slice())
        .map_err(|_| Error::ParseError("Failed to decipher stream.".to_owned()))?;

    let parsed_json: Value = serde_json::from_str(json).unwrap();
    if parsed_json.get("error").is_some() {
        Err(Error::StreamAcqError(
            player.name.clone(),
            parsed_json["error"].to_string(),
        ))
    } else {
        Ok(parsed_json["streams"]["best"]["url"]
            .to_string()
            .replace("\"", ""))
    }
}


impl<'a> ProjectState<'a> {

    /// Apply a command onto the current project state, providing a new state.
    pub fn apply_cmd(
        &self,
        cmd: &Command<'a>,
    ) -> Result<(ProjectState<'a>, Vec<ModifiedState<'a>>), Error> {
        let mut new_state = self.clone();
        let mut modifications = Vec::<ModifiedState<'a>>::new();
        match cmd {
            Command::Toggle(player) => {
                modifications.push(ModifiedState::PlayerView(player));
                if self.active_players.contains(player) {
                    new_state.active_players.retain(|p| p != player);
                } else {
                    new_state.active_players.push(player);
                    if self.running && new_state.update_stream(player)? {
                        modifications.push(ModifiedState::PlayerStream(player));
                    }
                }

                Ok((new_state, modifications))
            }
            Command::Swap(p1, p2) => {
                let pos_p1 = self.active_players.iter().position(|p| p == p1);
                let pos_p2 = self.active_players.iter().position(|p| p == p2);

                modifications.push(ModifiedState::PlayerView(p1));
                modifications.push(ModifiedState::PlayerView(p2));

                if pos_p1.is_some() && pos_p2.is_some() {
                    new_state
                        .active_players
                        .swap(pos_p1.unwrap(), pos_p2.unwrap());
                } else if pos_p1.is_some() {
                    new_state.active_players.insert(pos_p1.unwrap(), p2);
                    new_state.active_players.remove(pos_p1.unwrap() + 1);

                    if self.running && new_state.update_stream(p1)? {
                        modifications.push(ModifiedState::PlayerStream(p1));
                    }
                } else if pos_p2.is_some() {
                    new_state.active_players.insert(pos_p2.unwrap(), p1);
                    new_state.active_players.remove(pos_p2.unwrap() + 1);

                    if self.running && new_state.update_stream(p2)? {
                        modifications.push(ModifiedState::PlayerStream(p2));
                    }
                }

                Ok((new_state, modifications))
            }
            Command::SetPlayers(players) => {
                for player in players {
                    if !new_state.active_players.contains(player) && self.running && new_state.update_stream(player)? {
                        modifications.push(ModifiedState::PlayerStream(player));
                    }
                }

                new_state.active_players.clear();
                
                for player in players {
                    modifications.push(ModifiedState::PlayerView(player));
                    new_state.active_players.push(player);
                }

                Ok((new_state, modifications))
            }
            Command::Refresh(players) if self.running => {
                for player in players {
                    if self.running && new_state.update_stream(player)? {
                        modifications.push(ModifiedState::PlayerStream(player));
                    }
                }
                Ok((new_state, modifications))
            }
            Command::Refresh(_) => Err(Error::ProjectStateError(
                "Cannot refresh streams while project is inactive.".to_owned(),
            )),
            Command::Layout(count, layout) => {
                modifications.push(ModifiedState::Layout);
                new_state.layouts_by_count.insert(*count, layout);
                Ok((new_state, modifications))
            }
            Command::SetPlayerField(player, field, value) => {
                let player_state = new_state.player_fields
                    .player_fields.get_mut(player).unwrap();

                match value {
                    Some(val) => player_state.insert(field.to_string(), val.to_string()),
                    None => player_state.remove(field)
                };
                
                modifications.push(ModifiedState::PlayerFields);

                Ok((new_state, modifications))
            }
            Command::SetPlayerFields(player, fields) => {
                new_state.player_fields.player_fields.get_mut(player).unwrap().extend(fields.clone());
                modifications.push(ModifiedState::PlayerFields);

                Ok((new_state, modifications))
            },
            Command::SetCommentaryIgnore(ignored) => {
                if !new_state.ignored_commentators.eq(ignored) {
                    modifications.push(ModifiedState::Commentary);
                    new_state.ignored_commentators = ignored.clone();
                }

                Ok((new_state, modifications))
            },
            Command::SetCommentary(comms) => {
                if !new_state.active_commentators.eq(comms) {
                    modifications.push(ModifiedState::Commentary);
                    new_state.active_commentators = comms.clone();
                }
        
                Ok((new_state, modifications))
            },
            Command::GetState => panic!("Should not get here"),
        }
    }

    /// Updates the provided player's stream in this state, returning if the stream changed.
    pub fn update_stream(&mut self, player: &'a Player) -> Result<bool, Error> {
        let new_stream = find_stream(player)?;
        let old_stream = self.streams.get(player);
        match !old_stream.is_some() || old_stream.unwrap().eq(&new_stream) {
            true => {
                self.streams.insert(player, new_stream);
                Ok(true)
            }
            false => Ok(false),
        }
    }

    pub fn to_save_state(&self) -> ProjectStateSerializer {
        ProjectStateSerializer {
            active_players: self
                .active_players
                .iter()
                .map(|a| a.name.to_owned())
                .collect(),
            active_commentators: self.active_commentators.clone(),
            ignored_commentators: self.ignored_commentators.clone(),
            player_fields: self.player_fields.to_save_state(),
            layouts_by_count: self
                .layouts_by_count
                .iter()
                .map(|(k, v)| (k.to_owned(), v.name.to_owned()))
                .collect(),
        }
    }

    pub fn from_save_state(
        save: &ProjectStateSerializer,
        project: &'a Project,
        obs_cfg: &'a LayoutFile,
    ) -> Result<ProjectState<'a>, Error> {
        Ok(ProjectState {
            running: false,
            active_players: save
                .active_players
                .iter()
                .map(|p| {
                    project
                        .find_by_nick(p)
                        .ok_or(Error::ProjectLoadError(format!(
                            "Failed to find player {} when loading file",
                            p
                        )))
                })
                .collect::<Result<Vec<&Player>, Error>>()?,
            active_commentators: save.active_commentators.clone(),
            ignored_commentators: save.ignored_commentators.clone(),
            streams: HashMap::new(),
            player_fields: FieldState::from_save_state(&save.player_fields, project)?,
            layouts_by_count: save
                .layouts_by_count
                .iter()
                .map(|(k, v)| {
                    obs_cfg
                        .layouts
                        .iter()
                        .find(|l| &l.name == v)
                        .ok_or(Error::ProjectLoadError(format!(
                            "Failed to find layout {}",
                            k
                        )))
                        .map(|l| (k.to_owned(), l))
                })
                .collect::<Result<HashMap<u32, &Layout>, Error>>()?,
        })
    }
}

impl<'a> FieldState<'a> {
    pub fn to_save_state(&self) -> FieldStateSerializer {
        FieldStateSerializer {
            player_fields: self
                .player_fields
                .iter()
                .map(|record| (record.0.name.to_owned(), record.1.to_owned()))
                .collect(),
        }
    }

    pub fn from_save_state(
        save: &FieldStateSerializer,
        project: &'a Project,
    ) -> Result<FieldState<'a>, Error> {
        save.player_fields
            .iter()
            .map(|(name, time)| {
                project
                    .find_by_nick(name)
                    .ok_or(Error::ProjectLoadError(format!(
                        "Failed to find player {} when loading file",
                        name
                    )))
                    .map(|usr| (usr, time.to_owned()))
            })
            .collect::<Result<Vec<(&Player, HashMap<String, String>)>, Error>>()
            .map(|fields| FieldState {
                player_fields: fields.into_iter().collect(),
            })
    }
}
