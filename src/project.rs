use std::collections::HashMap;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::{player::Player, error::Error};

/// Contains the immutable state of a project.
#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub struct Project {
    #[serde(flatten)]
    pub project_type: ProjectTypeSettings,
    pub integrations: Vec<Integration>,
    pub players: Vec<Player>,
}

/// A type used to determine the project type
#[derive(PartialEq, Debug, Serialize, Deserialize, Clone, ValueEnum)]
pub enum ProjectType {
    Marathon,
    Relay
}

#[derive(PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProjectTypeSettings {
    Marathon,
    Relay {
        teams: HashMap<String, Vec<String>>
    },
}

/// A type used to determine the project type
#[derive(PartialEq, Debug, Serialize, Deserialize, Clone)]
pub enum Integration {
    Discord,
    TheRun,
}

impl Project {
    /// Return a player by name or nickname, or return error
    pub fn find_player(&self, name: &'_ str) -> Result<&Player, anyhow::Error> {
        self.players
            .iter()
            .find(|p| p.name_match(name))
            .ok_or(Error::UnknownPlayer(name.to_owned()).into())
    }
}
