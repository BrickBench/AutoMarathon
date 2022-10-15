#[derive(PartialEq, Eq, Hash, Debug)]
pub struct Player {
    pub name: String,
    pub nicks: Vec<String>,
    pub twitch: String
}

impl Player {
    pub fn name_match(&self, name: &str) -> bool {
        if self.name.to_lowercase() == name.to_lowercase() {
            return true;
        }

        self.nicks.iter().any(|x| x.to_lowercase() == name.to_lowercase())
    }
}
