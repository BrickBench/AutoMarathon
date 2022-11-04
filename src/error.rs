

#[derive(Debug)]
pub enum ProjectError {
    ProjectLoadError(String),
    StreamAcqError(String, String),
    ProjectStateError(String),
    PlayerError(String),
    CommandError(String),
    ParseError(String),
    GeneralError(String)
}

impl From<&str> for ProjectError {
    fn from(str: &str) -> Self {
        Self::GeneralError(str.to_owned())
    }
}

impl From<String> for ProjectError {
    fn from(str: String) -> Self {
        Self::GeneralError(str.to_owned())
    }
}
impl std::error::Error for ProjectError {}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectError::StreamAcqError(name, msg) => write!(f, "Failed to acquire stream for {}: {}", name, msg),
            ProjectError::ProjectStateError(msg) => write!(f, "Invalid state: {}", msg),    
            ProjectError::ProjectLoadError(msg) => write!(f, "Failed to load project: {}", msg),
            ProjectError::PlayerError(err) => write!(f, "Error with player: {}", err),
            ProjectError::CommandError(err) => write!(f, "Error with command: {}", err),
            ProjectError::ParseError(_) => todo!(),
            ProjectError::GeneralError(_) => todo!(),
        }
    }
}
