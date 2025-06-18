use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Json struct for project-independent settings
#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub obs_hosts: HashMap<String, ObsHost>,
    /// The OBS transition to use when transitioning between scenes.
    pub obs_short_transition: Option<String>,
    /// The OBS transition to use when transitioning within the same scene.
    pub obs_long_transition: Option<String>,
    /// Whether or not to delete VLC sources that are no longer in use.
    pub keep_unused_streams: Option<bool>,
    /// The Discord token for this bot.
    pub discord_token: Option<String>,
    /// The Discord channel to use for bot commands. If this is empty, the bot will not accept
    /// text commands.
    pub discord_command_channel: Option<String>,
    /// The port to use for the web server, or 28010 by default.
    pub web_port: Option<u16>,
    /// The default delay for the `therun` integration, in seconds, for synchronizing
    /// live data with runner streams.
    pub therun_delay: Option<f64>,
    /// Whether to calculate the discrete Fourier transform per-user for display purposes.
    pub transmit_voice_dft: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ObsHost {
    pub obs_ip: String,
    pub obs_port: u16,
    pub obs_password: Option<String>,
    /// Whether or not to enable Discord voice integration for this OBS host.
    pub enable_voice: Option<bool>,
    /// The Discord guild (server) ID used for voice integration.
    pub discord_voice_channel_guild_id: Option<u64>,
    /// The Discord channel ID that the bot will join for voice integration.
    pub discord_voice_channel_id: Option<u64>,
}
