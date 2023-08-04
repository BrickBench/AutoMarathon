
#[derive(Debug)]
pub enum Error {
    ProjectLoadError(String),
    StreamAcqError(String, String),
    ProjectStateError(String),
    PlayerError(String),
    CommandError(String),
    ParseError(String),
    GeneralError(String),
    LayoutError(String),
    DiscordError(serenity::Error),
    ObsError(obws::Error),
    IOError(std::io::Error),
    JsonError(serde_json::Error),
    HTTPError(reqwest::Error),
    WebsocketError(tokio_tungstenite::tungstenite::Error)
}

impl From<&str> for Error {
    fn from(str: &str) -> Self {
        Self::GeneralError(str.to_owned())
    }
}

impl From<String> for Error {
    fn from(str: std::string::String) -> Self {
        Self::GeneralError(str.to_owned())
    }
}

impl From<serenity::Error> for Error {
    fn from(err: serenity::Error) -> Self {
        Error::DiscordError(err)
    }
}

impl From<obws::Error> for Error {
    fn from(err: obws::Error) -> Self {
        Error::ObsError(err)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::IOError(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::JsonError(err)
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Error::HTTPError(err)
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for Error {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        Error::WebsocketError(err)
    }
}

impl std::error::Error for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::StreamAcqError(name, msg) => write!(f, "Failed to acquire stream for {}: {}", name, msg),
            Error::ProjectStateError(msg) => write!(f, "Invalid state: {}", msg),    
            Error::ProjectLoadError(msg) => write!(f, "Failed to load project: {}", msg),
            Error::PlayerError(err) => write!(f, "Unknown player: {}", err),
            Error::CommandError(err) => write!(f, "Error with command: {}", err),
            Error::ParseError(_) => todo!(),
            Error::GeneralError(err) => write!(f, "{}", err),
            Error::LayoutError(err) => write!(f, "Error with layout: {}", err),
            Error::DiscordError(err) => write!(f, "Error with discord: {}", err.to_string()),
            Error::ObsError(err) => write!(f, "Error with OBS: {}", err.to_string()),
            Error::IOError(err) => write!(f, "IO Error: {}", err.to_string()),
            Error::JsonError(err) => write!(f, "Json Error: {}", err.to_string()),
            Error::HTTPError(err) => write!(f, "HTTP Error: {}", err.to_string()),
            Error::WebsocketError(err) => write!(f, "Websocket Error: {}", err.to_string()),
        }
    }
}
