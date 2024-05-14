use std::process;

use crate::{db::ProjectDb, error::Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;

#[derive(PartialEq, Eq, Debug, FromRow, Clone)]
pub struct Runner {
    /// Player's display name
    pub name: String,

    /// Player's stream link
    /// If the link is an https:// address,
    /// it is used as-is, otherwise it is treated
    /// as a Twitch handle.
    ///
    /// This is assumed to be the same as the name if None
    pub stream: Option<String>,
    /// Player's TheRun.gg username.
    ///
    /// This is assumed to be the same as the name if None
    pub therun: Option<String>,

    /// A cache of this runner's latest valid m3u8 link
    pub cached_stream_url: Option<String>,

    /// User volume in percent
    pub volume_percent: u32
}

#[derive(PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct FieldDefault {
    pub field: String,
    pub value: String,
}

impl Runner {
    pub fn get_therun_username(&self) -> String {
        self.therun.clone().unwrap_or(self.name.clone())
    }

    pub fn get_stream(&self) -> String {
        match &self.stream {
            Some(stream) => {
                if stream.starts_with("https://") {
                    stream.to_string()
                } else {
                    format!("https://twitch.tv/{}", stream)
                }
            }
            None => format!("https://twitch.tv/{}", self.name),
        }
    }

    /// Returns a .m3u8 link corresponding to the current players' stream.
    pub fn find_stream(&mut self) -> anyhow::Result<bool> {
        let output = process::Command::new("streamlink")
            .arg("-Q")
            .arg("-j")
            .arg(&self.get_stream())
            .output()
            .map_err(|_| {
                Error::FailedStreamAcq(self.name.clone(), "Failed to aquire stream.".to_owned())
            })?;

        let json = std::str::from_utf8(output.stdout.as_slice())?;

        let parsed_json: Value =
            serde_json::from_str(json).expect("Unable to parse streamlink output");

        if parsed_json.get("error").is_some() {
            Err(Error::FailedStreamAcq(
                self.name.clone(),
                parsed_json["error"].to_string(),
            ))?
        } else {
            let new_url = parsed_json["streams"]["720p"]["url"]
                .to_string()
                .replace('\"', "");
            if let Some(old_url) = &self.cached_stream_url {
                if old_url != &new_url {
                    self.cached_stream_url = Some(new_url);
                    Ok(true)
                } else {
                    Ok(false)
                }
            } else {
                self.cached_stream_url = Some(new_url);
                Ok(true)
            }
        }
    }

    pub async fn find_and_save_stream(&mut self, db: &ProjectDb) -> anyhow::Result<bool> {
        if self.find_stream()? {
            db.update_runner_stream_url(self).await?;
            return Ok(true);
        }
        Ok(false)
    }
}
