use serde::{Deserialize, Deserializer, Serializer};
use sqlx::types::time;

pub fn serialize_datetime<S>(x: &Option<time::OffsetDateTime>, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Some(x) = x {
        s.serialize_u64((x.unix_timestamp_nanos() / 1_000_000) as u64)
    } else {
        s.serialize_none()
    }
}

pub fn deserialize_datetime<'de, D>(d: D) -> Result<Option<time::OffsetDateTime>, D::Error>
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
