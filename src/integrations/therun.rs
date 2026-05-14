use std::{collections::HashMap, sync::Arc};

use rand::distributions::Distribution;
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;
use statrs::distribution::Exp;
use tokio::{
    sync::broadcast,
    time::{self, sleep},
};
use tokio_stream::StreamExt;
use url::Url;

use crate::{
    core::{db::ProjectDb, event::EventResult, runner::Runner},
    web::streams::WebCommand,
    ActorRef, Directory, Rto,
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
    pub best_possible: Option<f64>,
}

/// Notifies the TheRun.gg poller of a change in runner TheRun.gg status
pub enum TheRunCommand {
    AddRunner(Runner, Rto<()>),
    RemoveRunner(Runner, Rto<()>),
}

pub type TheRunActor = ActorRef<TheRunCommand>;

/// A list of TheRun.gg users who are to be polled
type LiveRunners = Arc<tokio::sync::Mutex<Vec<i64>>>;

pub async fn process_therun_data(
    db: &ProjectDb,
    directory: &Directory,
    runner_id: i64,
    data: &Run,
) -> anyhow::Result<()> {
    match db.set_runner_run_data(runner_id, data).await {
        Ok(_) => {}
        Err(e) => log::error!("Failed to update runner {}'s run data: {}", runner_id, e),
    };

    directory
        .web_actor
        .send(WebCommand::SendLiveSplitUpdate(runner_id));

    let runner = db.get_runner(runner_id).await?;
    if !runner.use_live_data {
        return Ok(());
    }

    let events = db.get_event_ids_for_runner(runner_id).await?;
    let streams = db.get_streamed_events().await?;

    for event in events {
        if streams.contains(&event) {
            let mut event = db.get_event(event).await?;

            if event.timer_start_time.is_none() {
                continue;
            }

            let result = event
                .runner_state
                .get(&runner_id)
                .and_then(|state| state.result.as_ref());
            if let Some(sqlx::types::Json(EventResult::SplitTimes {
                splits,
                final_result,
            })) = result
            {
                let mut split_updated = false;
                let mut new_splits = splits.clone();
                for (i, split) in data.splits.iter().enumerate() {
                    if i < new_splits.len() && new_splits[i].is_none() {
                        split_updated = true;
                        new_splits[i] = split.split_time;
                    }
                }

                if !split_updated {
                    return Ok(());
                }

                let new_result = EventResult::SplitTimes {
                    final_result: final_result.clone(),
                    splits: new_splits,
                };

                event.runner_state.insert(
                    runner_id,
                    crate::core::event::RunnerEventState {
                        runner: runner_id,
                        result: Some(sqlx::types::Json(new_result)),
                    },
                );

                db.update_event(&event, true).await?;
            }
        }
    }

    Ok(())
}

pub async fn query_therun_state_for_username(username: &str) -> anyhow::Result<Run> {
    let resp = reqwest::get(format!("https://therun.gg/api/live/{}", username)).await?;

    let run = resp.json::<Run>().await?;

    Ok(run)
}
/// Worker to manage TheRun.gg connections
pub async fn run_therun_actor(
    db: Arc<ProjectDb>,
    directory: Directory,
    mut therun_rx: tokio::sync::mpsc::UnboundedReceiver<TheRunCommand>,
) -> anyhow::Result<()> {
    let live_runners = LiveRunners::default();
    let (death_tx, _) = broadcast::channel(16);
    while let Some(alert) = therun_rx.recv().await {
        match alert {
            TheRunCommand::AddRunner(runner, rto) => {
                if let Some(username) = runner.get_therun_username() {
                    log::info!("Adding {} to TheRun.gg poller", username);
                    live_runners.lock().await.push(runner.participant);

                    tokio::spawn(create_therun_websocket_monitor(
                        db.clone(),
                        directory.clone(),
                        runner.participant,
                        username,
                        live_runners.clone(),
                        death_tx.clone(),
                    ));
                    rto.reply(Ok(()));
                } else {
                    rto.reply(Err(anyhow::anyhow!(
                        "Runner added to therun, but no TheRun.gg username"
                    )));
                }
            }
            TheRunCommand::RemoveRunner(runner, rto) => {
                live_runners
                    .lock()
                    .await
                    .retain(|r| r != &runner.participant);

                death_tx.send(runner.participant).ok();

                rto.reply(Ok(()));
            }
        }
    }

    Ok(())
}

/// Creates a player info websocket, restarting it on failure.
async fn create_therun_websocket_monitor(
    db: Arc<ProjectDb>,
    directory: Directory,
    runner: i64,
    therun: String,
    runners: LiveRunners,
    death_monitor: broadcast::Sender<i64>,
) -> Result<(), anyhow::Error> {
    log::info!(
        "Starting TheRun.gg WebSocket monitor for {} ({}), polling inital state",
        runner,
        therun
    );

    match query_therun_state_for_username(&therun).await {
        Ok(run) => {
            if let Err(e) = process_therun_data(&db, &directory, runner, &run).await {
                log::error!("Failed to process TheRun.gg data for {}: {}", runner, e);
            }
        }
        Err(e) => {
            log::warn!(
                "Failed to aquire initial state for {} ({}) from TheRun.gg: {}",
                runner,
                therun,
                e
            );
        }
    }

    loop {
        let res = tokio::spawn(run_runner_websocket(
            db.clone(),
            directory.clone(),
            runner,
            therun.clone(),
            death_monitor.subscribe(),
        ))
        .await;

        if runners.lock().await.contains(&runner) {
            match res {
                Ok(Ok(restart)) => {
                    if restart {
                        log::debug!(
                            "TheRun.gg WebSocket closed for {} ({}), reattempting in 30 seconds...",
                            runner,
                            therun
                        )
                    } else {
                        log::debug!(
                            "TheRun.gg WebSocket closed for {} ({})",
                            runner,
                            therun
                        );
                        return Ok(());
                    }
                },
                Ok(Err(error)) => {
                    log::warn!(
                        "TheRun.gg WebSocket had an error for {} ({}, {}), reattempting in 30 seconds...",
                        runner,
                        therun,
                        error.to_string()
                    );
                }
                Err(error) => log::warn!(
                    "TheRun.gg WebSocket task for {} ({}) failed with {}, reattempting in 30 seconds...",
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
///
/// Returns if it should restart.
async fn run_runner_websocket(
    db: Arc<ProjectDb>,
    directory: Directory,
    runner: i64,
    therun: String,
    mut death_monitor: broadcast::Receiver<i64>,
) -> Result<bool, anyhow::Error> {
    let (mut stream, _) = tokio_tungstenite::connect_async(
        Url::parse(&format!("wss://ws.therun.gg/?username={}", therun)).unwrap(),
    )
    .await?;

    log::debug!("TheRun.gg WebSocket open for {} ({})", runner, therun);

    loop {
        tokio::select! {
            kill_name = death_monitor.recv() => {
                if let Ok(kill_name) = kill_name {
                    if kill_name == runner{
                        log::debug!("Killing TheRun.gg websocket for {} ({})", runner, therun);
                        return Ok(false);
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
                                if let Err(e) = process_therun_data(&db, &directory, runner, &stats.run).await {
                                    log::error!("Failed to process TheRun.gg data for {}: {}", runner, e);
                                }

                            }
                            Err(err) => {
                                log::debug!("Failed to parse {} endpoint: {}", runner, err.to_string());
                            }
                        };
                    }
                    Some(Err(err)) => log::error!("{}", err.to_string().replace("\\\"", "\"")),
                    None => return Ok(true),
                }
            }
        }
    }
}

// 28
fn float_cmp(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SingleSplit {
    split_time: Option<f64>,
    gold_split: f64,
}

impl SingleSplit {
    fn from_run(run: &Run) -> Vec<Self> {
        let mut splits = Vec::with_capacity(run.splits.len());
        let mut last_time = 0.0;
        for split in &run.splits {
            splits.push(SingleSplit {
                split_time: split.split_time.map(|t| t - last_time).map(|t| t / 1000.0),
                gold_split: split.best_possible.unwrap_or(0.0) / 1000.0,
            });

            if let Some(split_time) = split.split_time {
                last_time = split_time;
            }
        }

        splits
    }
}

