use serde::{Serialize, Deserialize};

/// Json struct for project-independent settings
#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub obs_ip: Option<String>,
    pub obs_port: Option<u16>,
    pub obs_password: Option<String>,
    pub obs_transition: Option<String>,
    pub keep_unused_streams: Option<bool>,
    pub discord_token: Option<String>,
    pub discord_command_channel: Option<String>,
    pub discord_voice_channel: Option<String>,
}

