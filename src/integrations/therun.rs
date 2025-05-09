use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;
use tokio::{
    sync::broadcast,
    time::{self, sleep},
};
use tokio_stream::StreamExt;
use url::Url;

use crate::{
    core::{db::ProjectDb, runner::Runner},
    ActorRef, Rto,
};

/// TheRun websocket return type
#[derive(Serialize, Deserialize)]
pub struct TheRunReturnJson {
    pub user: String,
    pub run: Run,
}

/// Data for an active LiveSplit run
#[allow(non_snake_case)]
#[derive(Debug, Serialize, Deserialize, Clone, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub pb: Option<f64>,
    pub sob: Option<f64>,
    pub best_possible: Option<f64>,
    pub delta: Option<f64>,
    pub started_at: String,
    pub current_comparison: String,
    pub current_split_name: String,
    pub current_split_index: i64,

    #[sqlx(skip)]
    pub splits: Vec<Split>,
}

/// A single LiveSplit split
#[allow(non_snake_case)]
#[derive(Debug, Serialize, Deserialize, Clone, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Split {
    pub name: String,
    pub pb_split_time: Option<f64>,
    pub split_time: Option<f64>,
}

/// Notifies the TheRun.gg poller of a change in runner TheRun.gg status
pub enum TheRunCommand {
    AddRunner(Runner, Rto<()>),
    RemoveRunner(Runner, Rto<()>),
}

pub type TheRunActor = ActorRef<TheRunCommand>;

/// A list of TheRun.gg users who are to be polled
type LiveRunners = Arc<tokio::sync::Mutex<Vec<String>>>;

/// Worker to manage TheRun.gg connections
pub async fn run_therun_actor(
    db: Arc<ProjectDb>,
    mut therun_rx: tokio::sync::mpsc::UnboundedReceiver<TheRunCommand>,
) -> anyhow::Result<()> {
    let live_runners = LiveRunners::default();
    let (death_tx, _) = broadcast::channel(16);
    while let Some(alert) = therun_rx.recv().await {
        match alert {
            TheRunCommand::AddRunner(runner, rto) => {
                live_runners
                    .lock()
                    .await
                    .push(runner.get_therun_username().expect("No username"));

                tokio::spawn(create_therun_websocket_monitor(
                    db.clone(),
                    runner.participant,
                    runner.get_therun_username().expect("No username"),
                    live_runners.clone(),
                    death_tx.clone(),
                ));

                rto.reply(Ok(()));
            }
            TheRunCommand::RemoveRunner(runner, rto) => {
                live_runners
                    .lock()
                    .await
                    .retain(|r| r != &runner.get_therun_username().expect("No username"));

                death_tx
                    .send(runner.get_therun_username().expect("No username"))
                    .ok();

                rto.reply(Ok(()));
            }
        }
    }

    Ok(())
}

/// Creates a player info websocket, restarting it on failure.
async fn create_therun_websocket_monitor(
    db: Arc<ProjectDb>,
    runner: i64,
    therun: String,
    runners: LiveRunners,
    death_monitor: broadcast::Sender<String>,
) -> Result<(), anyhow::Error> {
    loop {
        let res = tokio::spawn(run_runner_websocket(
            db.clone(),
            runner,
            therun.clone(),
            death_monitor.subscribe(),
        ))
        .await;

        if runners.lock().await.contains(&therun) {
            match res {
                Ok(_) => log::warn!(
                    "TheRun.gg WebSocket closed for {} ({}), reattempting in 30 seconds...",
                    runner,
                    therun
                ),
                Err(error) => log::warn!(
                    "TheRun.gg WebSocket closed for {} ({}, {}), reattempting in 30 seconds...",
                    runner,
                    therun,
                    error.to_string()
                ),
            }
            sleep(time::Duration::from_secs(30)).await;
        } else {
            log::info!("TheRun.gg WebSocket closed for {} ({})", runner, therun);
            return Ok(());
        }
    }
}

/// Creates a websocket for the provided runner.
///
/// This function will occasionally completely bypass the return or panic when failing,
/// so it is restarted by ```create_player_websocket```.
async fn run_runner_websocket(
    db: Arc<ProjectDb>,
    runner: i64,
    therun: String,
    mut death_monitor: broadcast::Receiver<String>,
) -> Result<(), anyhow::Error> {
    let (mut stream, _) = tokio_tungstenite::connect_async(
        Url::parse(&format!("wss://ws.therun.gg/?username={}", therun)).unwrap(),
    )
    .await?;

    log::info!("TheRun.gg WebSocket open for {} ({})", runner, therun);

    loop {
        tokio::select! {
            kill_name = death_monitor.recv() => {
                if let Ok(kill_name) = kill_name {
                    if kill_name == therun {
                        log::debug!("Killing TheRun.gg websocket for {} ({})", runner, therun);
                        return Ok(());
                    }
                }
            }
            message = stream.next() => {
                match message {
                    Some(Ok(msg)) => {
                        match serde_json::from_str::<TheRunReturnJson>(
                            msg.to_text().expect("Failed to get text"),
                        ) {
                            Ok(stats) => {
                                log::debug!("Received TheRun.gg data for {}", runner);

                                match db.set_runner_run_data(runner, &stats.run).await {
                                    Ok(_) => {}
                                    Err(e) => log::error!("Failed to update runner {}'s run data: {}", runner, e),
                                };
                            }
                            Err(err) => {
                                log::warn!("Failed to parse {} endpoint: {}", runner, err.to_string());
                            }
                        };
                    }
                    Some(Err(err)) => log::error!("{}", err.to_string().replace("\\\"", "\"")),
                    None => return Ok(()),
                }
            }
        }
    }
}

/*
pub struct TheRunRaceStats {
    pub leading_runner: i64,
    
    /// Deltas between each runner and the leading runner, at the last split shared
    /// by those two runners. If a runner has not completed the first split, their delta
    /// will not be present.
    pub runner_deltas: HashMap<i64, f64>
}

pub fn compare_runners(runners: HashMap<i64, Run>) -> Option<TheRunRaceStats> {
    // find which split and at what time each runner

}
*/
