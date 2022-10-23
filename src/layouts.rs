use serde::{Serialize, Deserialize};

use crate::state::ProjectState;

#[derive(Debug, Serialize, Deserialize)]
pub struct ObsConfiguration {
    ip: String,
    port: u16,
    password: Option<String>,
    layouts: Vec<Layout>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Layout {
    name: String,
    displays: Vec<UserDisplay>
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserDisplay {
    stream: PosSize,
    name: PosSize,
    webcam: Option<PosSize>,
    timer: Option<PosSize>,
    description: Option<PosSize>,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub struct PosSize {
    x: u32,
    y: u32,
    width: u32,
    height: u32
}

impl Layout {
    pub async fn apply_layout(&self, _state: &ProjectState<'_>, _obs: &obws::Client) -> Result<(), String> {
        Ok(())
    }
}

