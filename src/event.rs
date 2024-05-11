use serde::Serialize;
use sqlx::{prelude::FromRow, types::chrono};

#[derive(PartialEq, Debug, FromRow, Serialize)]
pub struct Event {
    pub name: String,
    pub therun_race_id: Option<String>,
    pub start_time: Option<chrono::DateTime<chrono::Utc>>,
    pub end_time: Option<chrono::DateTime<chrono::Utc>>,
    pub is_relay: bool,
    pub is_marathon: bool,
}

