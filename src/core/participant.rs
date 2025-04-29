use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

/// A struct representing a participant related to this event
#[derive(PartialEq, Eq, Debug, FromRow, Clone, Serialize, Deserialize, Default)]
pub struct Participant {
    /// Unique participant ID
    pub id: i64,

    /// The name of this participant
    pub name: String,

    /// This participant's pronouns
    pub pronouns: Option<String>,

    /// This participant's location
    pub location: Option<String>,

    /// This participant's Discord ID
    pub discord_id: Option<String>,

    /// Encoded player photo
    #[serde(skip)]
    pub photo: Option<Vec<u8>>,

    /// Whether this participant is an event host
    pub host: bool,
}
