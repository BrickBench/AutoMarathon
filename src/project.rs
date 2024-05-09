use std::{collections::HashMap, sync::Arc};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{player::Player, error::Error};

pub type ProjectStore = Arc<RwLock<Project>>;

/// Contains the immutable state of a project.
#[derive(PartialEq, Debug, Serialize, Deserialize, Clone)]
pub struct Project {
    pub features: Vec<Feature>,
    pub integrations: Vec<Integration>,
    pub timer_source: Option<TimerSource>,
    pub therun_race_id: Option<String>,
    pub relay_teams: Option<HashMap<String, Vec<String>>>,
    pub players: Vec<Player>,
}

/// Feature flags to enable certain AutoMarathon functionality
#[derive(PartialEq, Debug, Serialize, Deserialize, Clone, ValueEnum)]
pub enum Feature {
    StreamControl,
    Marathon,
    Timer,
    Relay,
}

/// Feature flags to enable certain AutoMarathon functionality
#[derive(PartialEq, Debug, Serialize, Deserialize, Clone, ValueEnum)]
pub enum TimerSource {
    TheRun,
    Manual
}

/// A type used to determine the project type
#[derive(PartialEq, Debug, Serialize, Deserialize, Clone)]
pub enum Integration {
    Discord,
    TheRun,
    LadderLeague
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
