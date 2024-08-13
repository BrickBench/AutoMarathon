use std::{collections::HashMap, sync::Arc};

use anyhow::anyhow;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sqlx::{prelude::FromRow, types::time};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{send_message, ActorRef, Directory, Rto};

use super::{db::ProjectDb, stream::StreamRequest};

fn serialize_datetime<S>(x: &Option<time::OffsetDateTime>, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Some(x) = x {
        s.serialize_u64((x.unix_timestamp_nanos() / 1_000_000) as u64)
    } else {
        s.serialize_none()
    }
}

fn deserialize_datetime<'de, D>(d: D) -> Result<Option<time::OffsetDateTime>, D::Error>
where
    D: Deserializer<'de>,
{
    let x = Option::<u64>::deserialize(d)?;
    let time = x.map(|x| time::OffsetDateTime::from_unix_timestamp_nanos(x as i128 * 1_000_000));
    match time {
        Some(Ok(x)) => Ok(Some(x)),
        _ => Ok(None),
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum EventResult {
    SingleScore { score: String },
    MultiScores { scores: HashMap<String, String> },
}

/// State of a runner in an event
#[derive(Debug, Serialize, FromRow, Clone, Deserialize)]
pub struct RunnerEventState {
    pub runner: i64,
    pub result: Option<sqlx::types::Json<EventResult>>,
}

/// Single event (game/race/relay)
#[derive(Debug, FromRow, Serialize, Clone, Deserialize)]
pub struct Event {
    /// Unique event ID
    pub id: i64,

    /// Name of the event
    pub name: String,

    /// Game for this event
    pub game: Option<String>,

    /// Category for this event
    pub category: Option<String>,

    /// Game console for this event
    pub console: Option<String>,

    /// If the event is complete 
    pub complete: Option<bool>,

    /// Time estimate for the event in seconds
    pub estimate: Option<i64>,

    /// The tournament this event is associated with
    pub tournament: Option<i64>,

    /// The four-character therun.gg race ID
    pub therun_race_id: Option<String>,

    /// The scheduled start time for this event
    #[serde(serialize_with = "serialize_datetime")]
    #[serde(deserialize_with = "deserialize_datetime")]
    pub event_start_time: Option<time::OffsetDateTime>,

    /// The start time of the event timer
    #[serde(serialize_with = "serialize_datetime")]
    #[serde(deserialize_with = "deserialize_datetime")]
    pub timer_start_time: Option<time::OffsetDateTime>,

    /// The end time of the event timer
    #[serde(serialize_with = "serialize_datetime")]
    #[serde(deserialize_with = "deserialize_datetime")]
    pub timer_end_time: Option<time::OffsetDateTime>,

    /// The default layouts to be used for this event.
    /// Note that these layouts are not checked for validity,
    /// as they may or may not exist in OBS when the event is made.
    #[sqlx(json)]
    pub preferred_layouts: Vec<String>,

    pub is_relay: bool,
    pub is_marathon: bool,

    #[sqlx(skip)]
    pub runner_state: HashMap<i64, RunnerEventState>,
}

pub enum EventRequest {
    Create(Event, Rto<()>),
    SetStartTime(i64, Option<time::OffsetDateTime>, Rto<()>),
    SetEndTime(i64, Option<time::OffsetDateTime>, Rto<()>),
    AddRunner(i64, i64, Rto<()>),
    RemoveRunner(i64, i64, Rto<()>),
    Update(Event, Rto<()>),
    Delete(i64, Rto<()>),
}

pub type EventActor = ActorRef<EventRequest>;

pub async fn run_event_actor(
    db: Arc<ProjectDb>,
    mut rx: UnboundedReceiver<EventRequest>,
    directory: Directory,
) -> Result<(), anyhow::Error> {
    while let Some(msg) = rx.recv().await {
        match msg {
            EventRequest::Create(mut event, rto) => {
                log::info!("Creating event {}", event.name);
                rto.reply(db.add_event(&mut event).await)
            }
            EventRequest::Update(event, rto) => {
                let update = db.update_event(&event).await;
                if let Err(e) = &update {
                    log::error!("Error updating event: {:?}", e);
                }
                rto.reply(update)
            }
            EventRequest::SetStartTime(id, time, rto) => {
                rto.reply(db.update_event_start_time(id, time).await);
            }
            EventRequest::SetEndTime(id, time, rto) => {
                rto.reply(db.update_event_end_time(id, time).await);
            }
            EventRequest::AddRunner(id, runner, rto) => match db.get_event(id).await {
                Ok(mut event) => {
                    if event.runner_state.iter().any(|(r, _)| *r == runner) {
                        rto.reply(Ok(()));
                    } else {
                        event.runner_state.insert(
                            runner,
                            RunnerEventState {
                                runner,
                                result: None,
                            },
                        );
                        rto.reply(db.update_event(&event).await);
                    }
                }
                Err(e) => rto.reply(Err(e)),
            },
            EventRequest::RemoveRunner(id, runner, rto) => match db.get_event(id).await {
                Ok(mut event) => {
                    if event.runner_state.iter().all(|(r, _)| *r != runner) {
                        rto.reply(Ok(()));
                    } else {
                        event.runner_state.remove(&id);
                        rto.reply(db.update_event(&event).await);
                    }
                }
                Err(e) => rto.reply(Err(e)),
            },
            EventRequest::Delete(id, rto) => match db.get_streamed_events().await {
                Ok(ev) => {
                    if ev.iter().any(|e| *e == id) {
                        match send_message!(directory.stream_actor, StreamRequest, Delete, id) {
                            Ok(_) => {
                                log::info!("Deleting event with ID {}", id);
                                rto.reply(db.delete_event(id).await)
                            }
                            Err(e) => rto.reply(Err(anyhow!(
                                "Error in deleting stream while deleting event {}: {:?}",
                                id,
                                e
                            ))),
                        }
                    } else {
                        rto.reply(db.delete_event(id).await);
                    }
                }
                Err(e) => rto.reply(Err(e)),
            },
        }
    }

    Ok(())
}
