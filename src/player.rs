use serde::{Serialize, Deserialize};

#[derive(PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Player {
    /// Player's display name
    pub name: String,
    /// Nicknames for commands
    pub nicks: Vec<String>,
    /// Link to Twitch channel
    pub stream: String
}

impl Player {

    /// Return if the provided name matches this player's name or nicknames.
    pub fn name_match(&self, name: &str) -> bool {
        if self.name.to_lowercase() == name.to_lowercase() {
            return true;
        }

        self.nicks.iter().any(|x| x.to_lowercase() == name.to_lowercase())
    }
}
