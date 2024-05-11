use std::collections::HashMap;

use serde::{Serialize, Deserialize};

/// Json struct for project-independent settings
#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub obs_hosts: HashMap<String, ObsHost>,
    pub obs_transition: Option<String>,
    pub keep_unused_streams: Option<bool>,
    pub discord_token: Option<String>,
    pub discord_command_channel: Option<String>,
    pub discord_voice_channel: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ObsHost {
    pub obs_ip: String,
    pub obs_port: u16,
    pub obs_password: Option<String>,
}
