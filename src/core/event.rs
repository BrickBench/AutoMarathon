use std::sync::Arc;

use anyhow::anyhow;
use serde::{Deserialize, Serialize, Serializer};
use sqlx::{prelude::FromRow, types::time};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{send_message, ActorRef, Directory, Rto};

use super::{db::ProjectDb, stream::StreamRequest};

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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum EventResult {
    SingleTime { time: f64 },
    SplitTimes { split_times: Vec<f64> },
    SingleScore { score: f64 },
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

    /// The tournament this event is associated with
    pub tournament: Option<String>,

    /// The four-character therun.gg race ID
    pub therun_race_id: Option<String>,

    #[serde(serialize_with = "serialize_datetime")]
    pub start_time: Option<time::OffsetDateTime>,

    #[serde(serialize_with = "serialize_datetime")]
    pub end_time: Option<time::OffsetDateTime>,

    pub is_relay: bool,
    pub is_marathon: bool,

    #[sqlx(skip)]
    pub runner_state: Vec<RunnerEventState>,
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
            },
            EventRequest::Update(event, rto) => rto.reply(db.update_event(&event).await),
            EventRequest::SetStartTime(id, time, rto) => {
                rto.reply(db.update_event_start_time(id, time).await);
            }
            EventRequest::SetEndTime(id, time, rto) => {
                rto.reply(db.update_event_end_time(id, time).await);
            }
            EventRequest::AddRunner(id, runner, rto) => match db.get_event(id).await {
                Ok(mut event) => {
                    if event.runner_state.iter().any(|r| r.runner == runner) {
                        rto.reply(Ok(()));
                    } else {
                        event.runner_state.push(RunnerEventState {
                            runner: runner.clone(),
                            result: None,
                        });
                        rto.reply(db.update_event(&event).await);
                    }
                }
                Err(e) => rto.reply(Err(e)),
            },
            EventRequest::RemoveRunner(id, runner, rto) => match db.get_event(id).await {
                Ok(mut event) => {
                    if event.runner_state.iter().all(|r| r.runner != runner) {
                        rto.reply(Ok(()));
                    } else {
                        event.runner_state.remove(
                            event
                                .runner_state
                                .iter()
                                .position(|r| r.runner == runner)
                                .unwrap(),
                        );

                        rto.reply(db.update_event(&event).await);
                    }
                }
                Err(e) => rto.reply(Err(e)),
            },
            EventRequest::Delete(id, rto) => match db.get_streamed_events().await {
                Ok(ev) => {
                    if ev.iter().any(|e| *e == id) {
                        match send_message!(
                            directory.stream_actor,
                            StreamRequest,
                            DeleteStream,
                            id.clone()
                        ) {
                            Ok(_) => {
                                log::info!("Deleting event with ID {}", id);
                                rto.reply(db.delete_event(id).await)
                            },
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
