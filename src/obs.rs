use std::collections::HashMap;

use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ObsConfiguration {
    pub ip: Option<String>,
    pub port: Option<u16>,
    pub password: Option<String>,
    pub scene: String,
    pub layouts: HashMap<String, Layout>,
}

#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub struct Layout {
    pub name: String,
    pub default: Option<bool>,
    pub displays: Vec<UserDisplay>
}

#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub struct UserDisplay {
    pub stream: PosSize,
    pub name: PosSize,
    pub webcam: Option<PosSize>,
    pub timer: Option<PosSize>,
    pub description: Option<PosSize>,
}

#[derive(PartialEq, Eq, Debug, Copy, Clone, Serialize, Deserialize)]
pub struct PosSize {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32
}

