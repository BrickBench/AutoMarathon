use std::collections::HashMap;

use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ObsConfiguration {
    pub ip: Option<String>,
    pub port: Option<u16>,
    pub password: Option<String>,
    pub layouts: HashMap<String, Layout>,
}

#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub struct Layout {
    pub name: String,
    displays: Vec<UserDisplay>
}

#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub struct UserDisplay {
    stream: PosSize,
    name: PosSize,
    webcam: Option<PosSize>,
    timer: Option<PosSize>,
    description: Option<PosSize>,
}

#[derive(PartialEq, Eq, Debug, Copy, Clone, Serialize, Deserialize)]
pub struct PosSize {
    x: u32,
    y: u32,
    width: u32,
    height: u32
}

