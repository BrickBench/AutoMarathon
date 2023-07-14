use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub obs_ip: Option<String>,
    pub obs_port: Option<u16>,
    pub obs_password: Option<String>,
    pub discord_token: Option<String>,
    pub discord_channel: Option<String>,
}


