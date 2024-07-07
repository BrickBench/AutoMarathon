use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to aquire stream for {0}: {1}")]
    FailedStreamAcq(String, String),
    #[error("Failed to load project: {0}")]
    Unknown(String),
    #[error("Unknown layout {0}")]
    UnknownLayout(String),
}

impl From<String> for Error {
    fn from(str: std::string::String) -> Self {
        Self::Unknown(str.to_owned())
    }
}
