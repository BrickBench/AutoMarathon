use std::{collections::HashMap, convert::Infallible, sync::Arc};

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::Receiver;
use warp::{http::Method, reply::WithStatus, Filter};

use crate::core::{db::ProjectDb, event::Event};

use super::therun::Run;

#[derive(Clone, Serialize, Deserialize)]
enum StateUpdate {
     
}

#[derive(Serialize, Deserialize)]
struct StateResponse {
    start_time: Option<u64>,
    end_time: Option<u64>,
    runner_state: HashMap<String, PlayerResponse>,
}

#[derive(Serialize, Deserialize)]
struct PlayerResponse {
    runner: String,
    is_visible: bool,
    live_stats: Option<Run>,
    event_pb: Option<f64>,
    relay_split_time: Option<f64>,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum ProjectTypePlayerResponse {
    Marathon { event_pb: Option<f64> },
    Relay { run_time: Option<String> },
}

async fn get_event_by_args(
    args: HashMap<String, String>,
    db: &ProjectDb,
) -> Result<Event, WithStatus<String>> {
    if let Some(event) = args.get("name") {
        if let Ok(event) = db.get_event(&event).await {
            Ok(event)
        } else {
            Err(warp::reply::with_status(
                "Failed to find event by name".to_string(),
                warp::http::StatusCode::BAD_REQUEST,
            ))
        }
    } else if let Some(host) = args.get("host") {
        if let Ok(event) = db.get_event_by_obs_host(&host).await {
            if let Ok(event) = db.get_event(&event).await {
                Ok(event)
            } else {
                Err(warp::reply::with_status(
                    "Failed to find event by name".to_string(),
                    warp::http::StatusCode::BAD_REQUEST,
                ))
            }
        } else {
            Err(warp::reply::with_status(
                "Provided host is not currently running an event".to_string(),
                warp::http::StatusCode::BAD_REQUEST,
            ))
        }
    } else {
        Err(warp::reply::with_status(
            "Missing 'event' or 'host' field".to_string(),
            warp::http::StatusCode::BAD_REQUEST,
        ))
    }
}

async fn event_endpoint(
    args: HashMap<String, String>,
    db: Arc<ProjectDb>,
) -> Result<impl warp::Reply, Infallible> {
    match get_event_by_args(args, &db).await {
        Ok(event) => Ok(warp::reply::with_status(
            serde_json::to_string::<Event>(&event).unwrap(),
            warp::http::StatusCode::OK,
        )),
        Err(reply) => Ok(reply),
    }
}

async fn commentary_endpoint(
    args: HashMap<String, String>,
    db: Arc<ProjectDb>,
) -> Result<impl warp::Reply, Infallible> {
    match get_event_by_args(args, &db).await {
        Ok(event) => match db.get_stream(&event.name).await {
            Ok(stream) => Ok(warp::reply::with_status(
                serde_json::to_string::<Vec<String>>(&stream.get_commentators()).unwrap(),
                warp::http::StatusCode::OK,
            )),
            Err(e) => Ok(warp::reply::with_status(
                e.to_string(),
                warp::http::StatusCode::INTERNAL_SERVER_ERROR,
            )),
        },
        Err(reply) => Ok(reply),
    }
}

async fn run_dashboard_websocket(socket: warp::ws::WebSocket, db: Arc<ProjectDb>, state_rx: Receiver<StateUpdate>) {
    let (mut tx, mut rx) = socket.split();

    tokio::spawn(async move {

    });

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

pub async fn run_http_server(db: Arc<ProjectDb>) -> Result<(), anyhow::Error> {
    let cors = warp::cors()
        .allow_any_origin()
        .allow_headers(vec![
            "User-Agent",
            "Sec-Fetch-Mode",
            "Referer",
            "Origin",
            "Access-Control-Allow-Origin",
            "Access-Control-Request-Method",
            "Access-Control-Request-Headers",
            "Access-Control-Allow-Headers",
        ])
        .allow_methods(&[Method::GET, Method::POST, Method::DELETE]);

    let (update_tx, _) = tokio::sync::broadcast::channel::<StateUpdate>(256);

    let socket = warp::path("ws")
        .and(warp::ws())
        .and(with_db(db.clone()))
        .and(warp::any().map(move || update_tx.subscribe()))
        .and(warp::path::end())
        .map(|ws: warp::ws::Ws, db: Arc<ProjectDb>, state_rx: Receiver<StateUpdate>| {
            ws.on_upgrade(move |socket| run_dashboard_websocket(socket, db, state_rx))
        });

    let project_endpoint = warp::path("event")
        .and(warp::get())
        .and(warp::query::<HashMap<String, String>>())
        .and(with_db(db.clone()))
        .and(warp::path::end())
        .and_then(event_endpoint);

    let commentary_endpoint = warp::path("commentators")
        .and(warp::get())
        .and(warp::query::<HashMap<String, String>>())
        .and(with_db(db.clone()))
        .and(warp::path::end())
        .and_then(commentary_endpoint);

    let dashboard = warp::path("static")
        .and(warp::get())
        .and(warp::fs::dir("web/static/timer.html"));

    warp::serve(
        project_endpoint
            .or(commentary_endpoint)
            .or(dashboard)
            .or(socket)
            .with(cors),
    )
    .run(([0, 0, 0, 0], 28010))
    .await;

    Ok(())
}

fn with_db(db: Arc<ProjectDb>) -> impl Filter<Extract = (Arc<ProjectDb>,), Error = Infallible> + Clone {
    warp::any().map(move || db.clone())
}
