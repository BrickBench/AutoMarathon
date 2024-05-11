use std::{collections::HashMap, sync::Arc};

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json;
use std::{thread, time};
use tokio::sync::{Mutex, mpsc::UnboundedReceiver};
use tokio_tungstenite;
use url::Url;

use crate::{
    db::ProjectDb, stream::{StreamActor, StreamCommand, StreamRequest}, ActorRef, Rto
};

/// TheRun websocket return type
#[derive(Serialize, Deserialize)]
pub struct TheRunReturnJson {
    user: String,
    run: Run
}

/// Data for an active LiveSplit run
#[allow(non_snake_case)]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Run {
    pub sob: Option<f64>,
    pub bestPossible: Option<f64>,
    pub delta: Option<f64>,
    pub startedAt: String,
    pub currentComparison: String,
    pub pb: Option<f64>,
    pub currentSplitName: String,
    pub currentSplitIndex: isize,
    pub splits: Vec<Split>,
}

/// A single LiveSplit split
#[allow(non_snake_case)]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Split {
    pub name: String,
    pub pbSplitTime: Option<f64>,
    pub splitTime: Option<f64>,
}

/// Requests that can be sent to TheRunActor
pub enum TheRunRequest {
    GetLivePlayerStats(String, Rto<Option<Run>>),
}

pub type TheRunActor = ActorRef<TheRunRequest>;

type StatsStore = Arc<Mutex<Option<(Run, bool)>>>;

pub async fn run_therun(
    db: Arc<ProjectDb>,
    state_actor: StreamActor,
    mut rx: UnboundedReceiver<TheRunRequest>
) -> Result<(), anyhow::Error> {
    let mut stats_map: HashMap<String, StatsStore> = HashMap::new();

    for player in &db.get_runners().await? {
        let store = Arc::new(Mutex::new(None));

        tokio::spawn(create_player_websocket_monitor(
            player.name.clone(),
            player.get_therun_username().to_string(),
            state_actor.clone(),
            store.clone(),
        ));

        stats_map.insert(player.name.clone(), store);
    }
    
    loop {
        match rx.recv().await.unwrap() {
            TheRunRequest::GetLivePlayerStats(player, rto) => {
                let stats: Option<Run>;

                {
                    let store = stats_map.get(&player).unwrap();
                    let stats_guard = store.lock().await;

                    stats = (*stats_guard).clone().map(|s| s.0);
                }

                rto.reply(Ok(stats))
            },
        }
    }
}

/// Creates a player info websocket, restarting it on failure.
pub async fn create_player_websocket_monitor(
    player: String,
    therun: String,
    state_actor: StreamActor,
    dest: StatsStore,
) -> Result<(), anyhow::Error> {
    loop {
        let res = tokio::spawn(create_player_websocket(
            player.clone(),
            therun.clone(),
            state_actor.clone(),
            dest.clone(),
        ))
        .await;

        match res {
            Ok(_) => log::warn!(
                "TheRun.gg WebSocket closed for {} ({}), reattempting in 30 seconds...",
                player,
                therun
            ),
            Err(error) => log::warn!(
                "TheRun.gg WebSocket closed for {} ({}, {}), reattempting in 30 seconds...",
                player,
                therun,
                error.to_string()
            ),
        }

        thread::sleep(time::Duration::from_secs(30));
    }
}

/// Creates a websocket for the provided player.
///
/// This function will occasionally completely bypass the return or panic when failing,
/// so use the ```create_player_websocket_monitor``` function instead for error management.
async fn create_player_websocket(
    player: String,
    therun: String,
    state_actor: StreamActor,
    dest: StatsStore,
) -> Result<(), anyhow::Error> {
    let (stream, _) = tokio_tungstenite::connect_async(
        Url::parse(&format!("wss://ws.therun.gg/?username={}", therun)).unwrap(),
    )
    .await?;

    log::info!("TheRun.gg WebSocket open for {} ({})", player, therun);

    stream
        .for_each(|res| async {
            match res {
                Ok(msg) => {
                    match serde_json::from_str::<TheRunReturnJson>(
                        msg.to_text().expect("Failed to get text"),
                    ) {
                        Ok(stats) => {
                            //let (rtx, rrx) = Rto::new();
                            //state_actor.send(StreamRequest::UpdateStream(StreamCommand::UpdateRunnerLiveData(player.clone(), stats.run.clone()), rtx));
                           // let _ = rrx.await;

                            let mut stats_store = dest.lock().await;
                            *stats_store = Some((stats.run, true));
                        }
                        Err(err) => {
                            log::warn!("Failed to parse {} endpoint: {}", player, err.to_string());
                        }
                    };
                }
                Err(err) => log::error!("{}", err.to_string().replace("\\\"", "\"")),
            }
        })
        .await;

    Ok(())
}
