use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc::UnboundedReceiver, watch, Mutex};
use warp::{reject::Rejection, Filter};

use crate::{
    core::{
        db::ProjectDb, event::Event, participant::Participant, runner::Runner, stream::StreamState,
    },
    integrations::{
        obs::{HostCommand, ObsHostState},
        therun::Run,
    },
    send_message, ActorRef, Directory, Rto,
};

use super::filters::{with_db, with_directory};

pub enum WebCommand {
    SendStateUpdate,
    SendVoiceUpdate(VoiceUpdate),
}

pub type WebActor = ActorRef<WebCommand>;

/// Struct for the elements project state that are
/// sent over the main state endpoint
#[derive(Serialize, Clone, Debug)]
struct StateUpdate {
    streams: Vec<StreamState>,
    events: Vec<Event>,
    people: HashMap<i64, Participant>,
    runners: HashMap<i64, Runner>,
    active_runs: HashMap<i64, Run>,
    hosts: HashMap<String, ObsHostState>,
    custom_fields: HashMap<String, Option<String>>,
}

/// Data for a single user's voice state
#[derive(Serialize, Debug, Clone)]
pub struct ParticipantVoiceState {
    /// Whether the user was actively speaking during this update
    pub active: bool,
    /// The user's peak volume during this 20ms period
    pub peak_db: f32,
    /// If `transmit_voice_dft` is enabled, this is the discrete fourier transform
    /// for the last 20ms of this user's voice, quantized into 16 bins
    pub voice_dft: [f32; 16],
}

/// Data for voice data in the last 20ms.
#[derive(Serialize, Debug, Clone)]
pub struct VoiceUpdate {
    pub host: String,
    pub voice_users: HashMap<String, ParticipantVoiceState>,
}

/// Struct denoting the current dashboard layout editor.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EditorClaim {
    unix_time: u64,
    editor: Option<String>,
}

/// Run a voice websocket for a single client
async fn run_voice_websocket(
    socket: warp::ws::WebSocket,
    mut voice_rx: broadcast::Receiver<VoiceUpdate>,
) {
    log::info!("New voice websocket connection opened");
    let (mut tx, _) = socket.split();
    while let Ok(update) = voice_rx.recv().await {
        if let Ok(update) = serde_json::to_string(&update) {
            if let Err(e) = tx.send(warp::ws::Message::text(update)).await {
                log::error!("Failed to send voice update: {}", e);
                break;
            }
        } else {
            log::error!("Failed to voice state update");
            break;
        }
    }
}

/// Run a dashboard websocket for a single client
async fn run_state_websocket(
    db: Arc<ProjectDb>,
    directory: Directory,
    socket: warp::ws::WebSocket,
    mut state_rx: broadcast::Receiver<StateUpdate>,
    address: Option<SocketAddr>,
) {
    match address {
        Some(addr) => log::debug!("New state websocket connection opened from {}", addr.ip()),
        None => log::debug!("New state websocket connection opened"),
    };

    let (mut tx, _) = socket.split();

    match assemble_state_update(db, &directory).await {
        Ok(update) => {
            if let Err(e) = tx
                .send(warp::ws::Message::text(
                    serde_json::to_string(&update).unwrap(),
                ))
                .await
            {
                log::error!("Failed to send initial state: {}", e);
            }
        }
        Err(e) => {
            log::error!("Failed to assemble initial state: {}", e);
        }
    }

    while let Ok(update) = state_rx.recv().await {
        if let Ok(update) = serde_json::to_string(&update) {
            if let Err(e) = tx.send(warp::ws::Message::text(update)).await {
                log::error!("Failed to send state update: {}", e);
                break;
            }
        } else {
            log::error!("Failed to serialize state update");
            break;
        }
    }
}

async fn run_dash_editor_websocket(
    socket: warp::ws::WebSocket,
    state_tx: watch::Sender<EditorClaim>,
    mut state_rx: watch::Receiver<EditorClaim>,
) {
    log::debug!("New dashboard websocket connection opened");
    let (mut tx, mut rx) = socket.split();

    let this_reader_id = Arc::new(Mutex::new(None));
    let last_reader_id = Arc::new(Mutex::new(None));

    let this_reader_id_clone = this_reader_id.clone();
    let state_tx_clone = state_tx.clone();
    let reader = tokio::spawn(async move {
        while let Some(Ok(msg)) = rx.next().await {
            if let Ok(msg_str) = msg.to_str() {
                match serde_json::from_str::<EditorClaim>(msg_str) {
                    Ok(claim) => {
                        if let Some(ref claim_id) = claim.editor {
                            log::info!("Claiming dashboard editor for {}", claim_id);
                            let mut this_reader_id = this_reader_id_clone.lock().await;
                            *this_reader_id = Some(claim_id.clone());
                        }

                        let _ = state_tx_clone.send(claim);
                    }
                    Err(e) => {
                        log::error!("Failed to deserialize editor claim: {}", e);
                    }
                }
            } else {
                log::error!("Failed to convert message to string, socket may be in binary mode?");
            }
        }
    });

    let last_reader_id_clone = last_reader_id.clone();
    let writer = tokio::spawn(async move {
        loop {
            let update = state_rx.borrow_and_update().clone();

            *last_reader_id_clone.lock().await = update.editor.clone();
            if let Ok(update) = serde_json::to_string(&update) {
                let _ = tx.send(warp::ws::Message::text(update)).await;
            } else {
                log::error!("Failed to serialize editor owner update");
                break;
            }

            if state_rx.changed().await.is_err() {
                log::error!("Editor receiver died");
                break;
            }
        }
    });

    tokio::select! {
        _ = reader => log::debug!("Dashboard editor websocket reader closed"),
        _ = writer => log::warn!("Dashboard editor websocket writer closed"),
    }

    // If the current editor is this function's editor, unlock the dashboard.
    if let Some(this_reader_id) = this_reader_id.lock().await.take() {
        if let Some(last_reader_id) = last_reader_id.lock().await.take() {
            if this_reader_id == last_reader_id {
                log::info!(
                    "{} was locking the dashboard, unlocking dashboard",
                    this_reader_id
                );
                let _ = state_tx.send(EditorClaim {
                    unix_time: 0,
                    editor: None,
                });
            }
        }
    };
}

async fn assemble_state_update(
    db: Arc<ProjectDb>,
    directory: &Directory,
) -> anyhow::Result<StateUpdate> {
    let event_names = db.get_event_ids().await?;
    let mut events = vec![];
    for event in event_names {
        events.push(db.get_event(event).await?);
    }

    let mut runs = HashMap::new();
    let people = db
        .get_participants()
        .await?
        .into_iter()
        .map(|p| (p.id, p))
        .collect();

    let runners: HashMap<i64, Runner> = db
        .get_runners()
        .await?
        .into_iter()
        .map(|r| (r.participant, r))
        .collect();

    for runner in &runners {
        if let Ok(run) = db.get_runner_run_data(*runner.0).await {
            runs.insert(*runner.0, run);
        }
    }

    let stream_names = db.get_streamed_events().await?;
    let mut streams = vec![];
    for stream in stream_names {
        streams.push(db.get_stream(stream).await?);
    }

    let hosts = send_message!(directory.obs_actor, HostCommand, GetState)?;
    let custom_fields = db.get_custom_fields().await?;

    Ok(StateUpdate {
        events,
        people,
        runners,
        streams,
        active_runs: runs,
        hosts,
        custom_fields,
    })
}

pub fn websocket_filters(
    db: Arc<ProjectDb>,
    directory: Directory,
    mut rx: UnboundedReceiver<WebCommand>,
) -> impl Filter<Extract = (impl warp::Reply,), Error = Rejection> + Clone {
    let (state_update_tx, _) = tokio::sync::broadcast::channel::<StateUpdate>(256);
    let state_reader_tx = state_update_tx.clone();
    let state_socket = warp::path!("ws")
        .and(warp::ws())
        .and(with_db(db.clone()))
        .and(with_directory(directory.clone()))
        .and(warp::any().map(move || state_update_tx.subscribe()))
        .and(warp::filters::addr::remote())
        .map(
            |ws: warp::ws::Ws,
             db: Arc<ProjectDb>,
             directory: Directory,
             state_rx: broadcast::Receiver<StateUpdate>,
             address: Option<SocketAddr>| {
                ws.on_upgrade(move |socket| {
                    run_state_websocket(db, directory, socket, state_rx, address)
                })
            },
        );

    let (voice_update_tx, _) = tokio::sync::broadcast::channel::<VoiceUpdate>(256);
    let voice_reader_tx = voice_update_tx.clone();
    let voice_socket = warp::path!("ws" / "voice")
        .and(warp::ws())
        .and(warp::any().map(move || voice_update_tx.subscribe()))
        .map(
            |ws: warp::ws::Ws, state_rx: broadcast::Receiver<VoiceUpdate>| {
                ws.on_upgrade(move |socket| run_voice_websocket(socket, state_rx))
            },
        );

    let (editor_update_tx, _) = tokio::sync::watch::channel::<EditorClaim>(EditorClaim {
        unix_time: 0,
        editor: None,
    });

    let editor_update_tx_clone = editor_update_tx.clone();
    let editor_socket = warp::path!("ws" / "dashboard-editor")
        .and(warp::ws())
        .and(warp::any().map(move || editor_update_tx_clone.subscribe()))
        .and(warp::any().map(move || editor_update_tx.clone()))
        .map(
            |ws: warp::ws::Ws,
             editor_rx: watch::Receiver<EditorClaim>,
             editor_tx: watch::Sender<EditorClaim>| {
                ws.on_upgrade(move |socket| run_dash_editor_websocket(socket, editor_tx, editor_rx))
            },
        );

    tokio::spawn(async move {
        loop {
            match rx.recv().await.unwrap() {
                WebCommand::SendStateUpdate => {
                    match assemble_state_update(db.clone(), &directory).await {
                        Ok(update) => {
                            let _ = state_reader_tx.send(update);
                        }
                        Err(e) => {
                            log::error!("Failed to assemble state update: {}", e);
                        }
                    }
                }
                WebCommand::SendVoiceUpdate(voice_update) => {
                    let _ = voice_reader_tx.send(voice_update);
                }
            }
        }
    });

    editor_socket.or(voice_socket).or(state_socket)
}
