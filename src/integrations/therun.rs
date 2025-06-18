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
    core::{db::ProjectDb, runner::Runner},
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

    if let Ok(resp) = reqwest::get(format!("https://therun.gg/api/live/{}", therun)).await {
        match resp.json::<Run>().await {
            Ok(run) => {
                match db.set_runner_run_data(runner, &run).await {
                    Ok(_) => {}
                    Err(e) => log::error!("Failed to update runner {}'s run data: {}", runner, e),
                };
            }
            Err(e) => {
                log::warn!(
                    "Failed to parse initial state for {} ({}) from TheRun.gg: {}",
                    runner,
                    therun,
                    e
                );
            }
        }
    } else {
        log::warn!(
            "Failed to fetch initial state for {} ({}) from TheRun.gg, will retry later",
            runner,
            therun
        );
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

                                match db.set_runner_run_data(runner, &stats.run).await {
                                    Ok(_) => {}
                                    Err(e) => log::error!("Failed to update runner {}'s run data: {}", runner, e),
                                };

                                directory.web_actor.send(WebCommand::SendLiveSplitUpdate(runner));
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

#[derive(Serialize, Debug, Clone)]
pub struct HeadToHead {
    pub run1: i64,
    pub run2: i64,
    pub probability: f64, // positive for run1 to win, negative for run2
}

pub struct Probabilities {
    pub probabilities: Vec<f64>,
    pub h2h: Vec<HeadToHead>,
}

/// Calculates the live win probability for a set of runs.
///
/// Returns a vector of probabilities for each run to win.
pub fn determine_live_win_probability(runs: &[&Run]) -> Option<Probabilities> {
    let gold_to_delta_diffs = [
        6, 5, 3, 15, 11, 13, 6, 12, 7, 16, 4, 4, 6, 14, 15, 11, 20, 6, 10, 14, 4, 20, 3, 22, 3, 17,
        12, 12, 5, 5, 17, 8, 16, 10, 12, 20,
    ];

    let choke_splits = [32, 34, 35];

    let good_runs = runs
        .iter()
        .map(|run| SingleSplit::from_run(run))
        .collect::<Vec<_>>();

    if good_runs.len() < 2 {
        return None;
    }

    if good_runs.iter().any(|run| run.len() < 36) {
        return None;
    }

    // optimize having this out here
    let mut final_times = vec![0.0; good_runs.len()];

    let mut winners = vec![0.0; good_runs.len()];
    let mut h2h = HashMap::new();

    const SIMULATIONS: usize = 40000;

    let mut r = rand::rngs::OsRng;
    for _i in 0..SIMULATIONS {
        final_times.fill(0.0);
        for split in 0..36 {
            let exp = Exp::new(1.0 / gold_to_delta_diffs[split] as f64).unwrap();
            let sim_splits = good_runs
                .iter()
                .map(|run| &run[split])
                .map(|split| {
                    split
                        .split_time
                        .unwrap_or_else(|| split.gold_split + exp.sample(&mut r))
                })
                .collect::<Vec<_>>();

            for (j, sim_split) in sim_splits.iter().enumerate() {
                final_times[j] += sim_split;
            }

            if choke_splits.contains(&split) {
                // calculate choke
                let leader = final_times
                    .iter()
                    .enumerate()
                    .min_by(|a, b| float_cmp(*a.1, *b.1))
                    .unwrap()
                    .0;

                if good_runs[leader][split].split_time.is_none() {
                    let max_from_gold =
                        (good_runs[leader][split].gold_split + 50.0 - sim_splits[leader]).max(0.0);
                    let other_runners_times = sim_splits
                        .iter()
                        .enumerate()
                        .filter(|(idx, _)| *idx != leader)
                        .map(|(_, time)| time);
                    let delta_from_worst = other_runners_times
                        .max_by(|a, b| float_cmp(**a, **b))
                        .unwrap()
                        - sim_splits[leader];
                    let anomaly_point = max_from_gold.min(delta_from_worst);
                    if anomaly_point > 0.0 {
                        // add choke addon
                        final_times[leader] += Exp::new(
                            (4.0_f64.ln() / anomaly_point) + (1.5 * 3.0_f64.ln() / anomaly_point),
                        )
                        .unwrap()
                        .sample(&mut r);
                    }
                }
            }
        }

        let winner = final_times
            .iter()
            .enumerate()
            .min_by(|a, b| float_cmp(*a.1, *b.1))
            .unwrap()
            .0;
        winners[winner] += 1.0;

        for r1 in 0..good_runs.len() {
            for r2 in r1 + 1..good_runs.len() {
                if r1 != r2 {
                    let r1_time = final_times[r1];
                    let r2_time = final_times[r2];

                    let entry = h2h.entry((r1, r2)).or_insert((0, 0));
                    if r1_time < r2_time {
                        entry.0 += 1;
                    } else if r2_time < r1_time {
                        entry.1 += 1;
                    }
                }
            }
        }
    }

    Some(Probabilities {
        probabilities: winners
            .iter()
            .map(|&w| w / SIMULATIONS as f64)
            .collect::<Vec<_>>(),
        h2h: h2h
            .iter()
            .map(|((r1, r2), (r1_wins, r2_wins))| HeadToHead {
                run1: *r1 as i64,
                run2: *r2 as i64,
                probability: (*r1_wins as f64 - *r2_wins as f64) / SIMULATIONS as f64,
            })
            .collect(),
    })
}
