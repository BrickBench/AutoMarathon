use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Json struct for project-independent settings
#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub obs_hosts: HashMap<String, ObsHost>,
    pub obs_short_transition: Option<String>,
    pub obs_long_transition: Option<String>,
    pub keep_unused_streams: Option<bool>,
    pub discord_token: Option<String>,
    pub discord_command_channel: Option<String>,
    pub web_port: Option<u16>,
    pub transmit_voice_dft: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ObsHost {
    pub obs_ip: String,
    pub obs_port: u16,
    pub obs_password: Option<String>,
    pub discord_voice_channel_guild_id: Option<u64>,
    pub discord_voice_channel_id: Option<u64>,
    pub enable_voice: Option<bool>,
}
