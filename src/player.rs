use serde::{Serialize, Deserialize};

#[derive(PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Player {
    /// Player's display name
    pub name: String,
    /// Nicknames for commands
    pub nicks: Option<Vec<String>>,
    /// Link to Twitch channel
    pub stream: Option<String>,
    /// TheRun.gg username (if it differs from Twitch)
    pub therun: Option<String>,
}

#[derive(PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct FieldDefault {
    pub field: String,
    pub value: String
}

impl Player {
    /// Return if the provided name matches this player's name or nicknames.
    pub fn name_match(&self, name: &str) -> bool {
        if self.name.to_lowercase() == name.to_lowercase() {
            return true;
        }

        if self.therun.as_ref().map(|tr| name.to_lowercase() == tr.to_lowercase()).unwrap_or(false) {
            return true;
        }

        self.nicks.clone().map(|n| n.iter().any(|x| x.to_lowercase() == name.to_lowercase())).unwrap_or(false)
    }
    
    pub fn get_therun_username(&self) -> String {
        self.therun.clone().unwrap_or(self.name.clone())
    }

    pub fn get_stream(&self) -> String {
        self.stream.clone().unwrap_or(format!("https://twitch.tv/{}", self.name))
    }
}
