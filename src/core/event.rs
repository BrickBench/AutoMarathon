use serde::{Serialize, Serializer};
use sqlx::{prelude::FromRow, types::time};

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
    pub therun_race_id: Option<String>,
    #[serde(serialize_with = "serialize_datetime")]
    pub start_time: Option<time::OffsetDateTime>,
    #[serde(serialize_with = "serialize_datetime")]
    pub end_time: Option<time::OffsetDateTime>,
    pub is_relay: bool,
    pub is_marathon: bool,
}
