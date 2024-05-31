use std::sync::Arc;

use serde::{Serialize, Serializer};
use sqlx::{prelude::FromRow, types::time};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{ActorRef, Rto};

use super::db::ProjectDb;

fn serialize_datetime<S>(x: &Option<time::OffsetDateTime>, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Some(x) = x {
        return s.serialize_u64((x.unix_timestamp_nanos() / 1_000_000) as u64);
    } else {
        return s.serialize_none();
    }
}

/// Single event (game/race/relay)
#[derive(PartialEq, Debug, FromRow, Serialize)]
pub struct Event {
    pub name: String,

    /// The tournament this event is associated with
    pub tournament_id: Option<String>,

    /// The four-character therun.gg race ID
    pub therun_race_id: Option<String>,

    #[serde(serialize_with = "serialize_datetime")]
    pub start_time: Option<time::OffsetDateTime>,

    #[serde(serialize_with = "serialize_datetime")]
    pub end_time: Option<time::OffsetDateTime>,

    pub is_relay: bool,
    pub is_marathon: bool,
}

pub enum EventRequest {
    SetStartTime(String, Option<time::OffsetDateTime>, Rto<()>),
    SetEndTime(String, Option<time::OffsetDateTime>, Rto<()>),
    AddRunner(String, String, Rto<()>),
    RemoveRunner(String, String, Rto<()>),
}

pub type EventActor = ActorRef<EventRequest>;

pub async fn run_event_actor(db: Arc<ProjectDb>, mut rx: UnboundedReceiver<EventRequest>) {
    while let Some(msg) = rx.recv().await {
        match msg {
            EventRequest::SetStartTime(name, time, rto) => 
                rto.reply(db.update_event_start_time(&name, time).await),
            EventRequest::SetEndTime(name, time, rto) => 
                rto.reply(db.update_event_end_time(&name, time).await),
            EventRequest::AddRunner(name, runner, rto) => 
                rto.reply(db.add_runner_to_event(&name, &runner).await),
            EventRequest::RemoveRunner(name, runner, rto) =>
                rto.reply(db.delete_runner_from_event(&name, &runner).await)
        }
    }
}
