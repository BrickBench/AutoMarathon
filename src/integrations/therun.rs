use std::collections::HashMap;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json;
use std::{thread, time};
use tokio::sync::mpsc::UnboundedSender;
use tokio_tungstenite;
use url::Url;

use crate::{cmd::CommandSource, error::Error, CommandMessage};

///Response type for therun.gg player websocket
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
    currentSplitIndex: isize,
    splits: Vec<Split>,
}

#[allow(non_snake_case)]
#[derive(Debug, Serialize, Deserialize)]
struct Split {
    name: String,
    pbSplitTime: f64,
    splitTime: Option<f64>,
}

/// Creates a player info websocket, restarting it on failure.
pub async fn create_player_websocket_monitor(
    player: String,
    twitch: String,
    tx: UnboundedSender<CommandMessage>,
) -> Result<(), Error> {
    loop {
        let res = tokio::spawn(create_player_websocket(
            player.clone(),
            twitch.clone(),
            tx.clone(),
        ))
        .await;

        match res {
            Ok(_) => log::warn!(
                "TheRun.gg WebSocket closed for {} ({}), reattempting in 30 seconds...",
                player,
                twitch
            ),
            Err(error) => log::warn!(
                "TheRun.gg WebSocket closed for {} ({}, {}), reattempting in 30 seconds...",
                player,
                twitch,
                error.to_string()
            ),
        }

        //let _ = tx.send((
        //    CommandSource::SetPlayerField(
        //        player.clone(),
        //        "stats_active".to_string(),
        //        Some("false".to_string()),
        //    ),
        //    None,
        //));

        thread::sleep(time::Duration::from_secs(30));
    }
}

/// Creates a websocket for the provided player.
///
/// This function will occasionally completely bypass the return or panic when failing,
/// so use the ```create_player_websocket_monitor``` function instead for error management.
async fn create_player_websocket(
    player: String,
    twitch: String,
    tx: UnboundedSender<CommandMessage>,
) -> Result<(), Error> {
    let (stream, _) = tokio_tungstenite::connect_async(
        Url::parse(&format!("wss://ws.therun.gg/?username={}", twitch)).unwrap(),
    )
    .await?;

    log::info!("TheRun.gg WebSocket open for {} ({})", player, twitch);

    stream
        .for_each(|res| async {
            match res {
                Ok(msg) => {
                    match serde_json::from_str::<RunnerStats>(
                        msg.to_text().expect("Failed to get text"),
                    )   {
                        Ok(stats) => {
                            let mut stats_map = HashMap::<String, Option<String>>::new();
                            stats_map.insert(
                                "stats_active".to_string(),
                                Some((stats.run.currentSplitIndex >= 0).to_string()),
                            );
                            stats_map
                                .insert("delta".to_string(), Some(stats.run.delta.to_string()));
                            stats_map.insert(
                                "best_possible".to_string(),
                                Some(stats.run.bestPossible.to_string()),
                            );

                            if stats.run.currentSplitIndex > 0 {
                                if let Some(latest_time) = stats.run.splits
                                    [stats.run.currentSplitIndex as usize - 1]
                                    .splitTime
                                {
                                    stats_map.insert(
                                        "last_split".to_string(),
                                        Some(latest_time.to_string()),
                                    );
                                }
                            }

                            let _ = tx.send((
                                CommandSource::SetPlayerFields(player.clone(), stats_map),
                                None,
                            ));
                        }
                        Err(err) => {
                            log::warn!("Failed to parse {} endpoint: {}", player, err.to_string());
                        },
                    };
                }
                Err(err) => log::error!("{}", err.to_string().replace("\\\"", "\"")),
            }
        })
        .await;

    Ok(())
}
