use std::collections::HashMap;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json;
use tokio::sync::mpsc::UnboundedSender;
use tokio_tungstenite;
use std::{thread, time};
use url::Url;

use crate::{error::Error, CommandMessage};

#[allow(non_snake_case)]
#[derive(Debug, Serialize, Deserialize)]
struct RunnerStats {
    user: String,
    run: Run,
}

#[allow(non_snake_case)]
#[derive(Debug, Serialize, Deserialize)]
struct Run {
    sob: f64,
    bestPossible: f64,
    delta: f64,
    startedAt: String,
    currentComparison: String,
    pb: f64,
    currentSplitName: String,
    currentSplitIndex: usize,
    splits: Vec<Split>,
}

#[allow(non_snake_case)]
#[derive(Debug, Serialize, Deserialize)]
struct Split {
    name: String,
    pbSplitTime: f64,
    splitTime: Option<f64>,
}

pub async fn create_player_websocket(
    player: String,
    twitch: String,
    tx: UnboundedSender<CommandMessage>,
) -> Result<(), Error> {
    loop {
        let (mut stream, response) = tokio_tungstenite::connect_async(
            Url::parse(&format!("wss://ws.therun.gg/?username={}", twitch)).unwrap(),
        )
        .await?;

        log::info!("TheRun.gg WebSocket open for {} ({})", player, twitch);

        while let Some(res) = stream.next().await {
            match res {
                Ok(msg) => {
                    let stats = serde_json::from_str::<RunnerStats>(msg.to_text()?)?;

                    let mut stats_map = HashMap::<String, String>::new();
                    stats_map.insert("stats_active".to_string(), "true".to_string());
                    stats_map.insert("delta".to_string(), stats.run.delta.to_string());
                    stats_map.insert(
                        "best_possible".to_string(),
                        stats.run.bestPossible.to_string(),
                    );


                    if stats.run.currentSplitIndex > 0 {
                        if let Some(latest_time) = stats.run.splits[stats.run.currentSplitIndex - 1].splitTime {
                            stats_map.insert("last_split".to_string(), latest_time.to_string());
                        }
                    }

                    let command = format!(
                        "set_fields {} {}",
                        player,
                        serde_json::to_string(&stats_map).unwrap()
                    );

                    let _ = tx.send((command, None));
                }
                Err(err) => log::error!("{}", err.to_string().replace("\\\"", "\"")),
            }
        }

        log::warn!("TheRun.gg WebSocket closed for {} ({}), reattempting in 30 seconds...", player, twitch);

        let _ = tx.send((format!("set_field {} stats_active false", player), None));

        thread::sleep(time::Duration::from_secs(30));
    }
}
