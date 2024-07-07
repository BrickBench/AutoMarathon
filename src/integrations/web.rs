use crate::core::{runner::RunnerRequest, stream::StreamRequest};
use crate::Rto;
use std::{collections::HashMap, convert::Infallible, sync::Arc};

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast::Receiver, mpsc::UnboundedReceiver};
use warp::{http::Method, reply::WithStatus, Filter};

use crate::{
    core::{
        db::ProjectDb,
        event::{Event, EventRequest},
        runner::Runner,
        stream::StreamState,
    },
    send_message, ActorRef, Directory,
};

use super::{
    obs::{ObsCommand, ObsLayout, ObsHostState},
    therun::Run,
};

#[derive(Serialize, Clone, Debug)]
struct StateUpdate {
    streams: Vec<StreamState>,
    events: Vec<Event>,
    runners: Vec<Runner>,
    layouts: Vec<ObsLayout>,
    active_runs: HashMap<String, Run>,
    hosts: HashMap<String, ObsHostState>,
}

/// A Json struct to store an event/runner ID
#[derive(Serialize, Deserialize, Debug)]
struct Id {
    id: i64,
}

/// A Json struct to store values for a new stream
#[derive(Serialize, Deserialize, Debug)]
struct NewStream {
    event: i64,
    host: String,
}

/// A Json struct to set the streaming state of an OBS host
#[derive(Serialize, Deserialize, Debug)]
struct SetStreamingState {
    host: String,
    streaming: bool,
}

pub enum WebCommand {
    SendStateUpdate,
}

pub type WebActor = ActorRef<WebCommand>;

async fn get_event_by_args(
    args: HashMap<String, String>,
    db: &ProjectDb,
) -> Result<Event, WithStatus<String>> {
    if let Some(event) = args.get("id") {
        match i64::from_str_radix(event, 10) {
            Ok(id) => match db.get_event(id).await {
                Ok(event) => return Ok(event),
                Err(_) => Err(warp::reply::with_status(
                    "Failed to find event by ID".to_string(),
                    warp::http::StatusCode::BAD_REQUEST,
                )),
            },
            Err(_) => Err(warp::reply::with_status(
                "Failed to parse event ID".to_string(),
                warp::http::StatusCode::BAD_REQUEST,
            )),
        }
    } else if let Some(host) = args.get("host") {
        match db.get_event_by_obs_host(&host).await {
            Ok(event) => match db.get_event(event).await {
                Ok(event) => Ok(event),
                Err(_) => Err(warp::reply::with_status(
                    "Failed to find event by name".to_string(),
                    warp::http::StatusCode::BAD_REQUEST,
                )),
            },
            Err(_) => Err(warp::reply::with_status(
                "Provided host is not currently running an event".to_string(),
                warp::http::StatusCode::BAD_REQUEST,
            )),
        }
    } else {
        Err(warp::reply::with_status(
            "Missing 'id' or 'host' field".to_string(),
            warp::http::StatusCode::BAD_REQUEST,
        ))
    }
}

fn to_http_none_or_error(result: anyhow::Result<()>) -> Result<impl warp::Reply, Infallible> {
    match result {
        Ok(_) => Ok(warp::reply::with_status(
            "Success".to_string(),
            warp::http::StatusCode::OK,
        )),
        Err(e) => Ok(warp::reply::with_status(
            e.to_string(),
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        )),
    }
}

fn to_http_output<T: Serialize>(result: anyhow::Result<T>) -> Result<impl warp::Reply, Infallible> {
    match result {
        Ok(data) => Ok(warp::reply::with_status(
            serde_json::to_string::<T>(&data).unwrap(),
            warp::http::StatusCode::OK,
        )),
        Err(e) => Ok(warp::reply::with_status(
            e.to_string(),
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        )),
    }
}

async fn get_event_endpoint(
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

async fn create_event(event: Event, directory: Directory) -> Result<impl warp::Reply, Infallible> {
    to_http_none_or_error(send_message!(
        directory.event_actor,
        EventRequest,
        Create,
        event
    ))
}

async fn update_event(event: Event, directory: Directory) -> Result<impl warp::Reply, Infallible> {
    to_http_none_or_error(send_message!(
        directory.event_actor,
        EventRequest,
        Update,
        event
    ))
}

async fn delete_event(
    event_name: Id,
    directory: Directory,
) -> Result<impl warp::Reply, Infallible> {
    to_http_none_or_error(send_message!(
        directory.event_actor,
        EventRequest,
        Delete,
        event_name.id
    ))
}

async fn create_runner(
    runner: Runner,
    directory: Directory,
) -> Result<impl warp::Reply, Infallible> {
    to_http_none_or_error(send_message!(
        directory.runner_actor,
        RunnerRequest,
        Create,
        runner
    ))
}

async fn update_runner(
    runner: Runner,
    directory: Directory,
) -> Result<impl warp::Reply, Infallible> {
    to_http_none_or_error(send_message!(
        directory.runner_actor,
        RunnerRequest,
        Update,
        runner
    ))
}

async fn delete_runner(
    runner_name: Id,
    directory: Directory,
) -> Result<impl warp::Reply, Infallible> {
    to_http_none_or_error(send_message!(
        directory.runner_actor,
        RunnerRequest,
        Delete,
        runner_name.id
    ))
}

async fn create_stream(
    stream: NewStream,
    directory: Directory,
) -> Result<impl warp::Reply, Infallible> {
    to_http_none_or_error(send_message!(
        directory.stream_actor,
        StreamRequest,
        CreateStream,
        stream.event,
        stream.host
    ))
}

async fn update_stream(
    stream: StreamState,
    directory: Directory,
) -> Result<impl warp::Reply, Infallible> {
    to_http_none_or_error(send_message!(
        directory.stream_actor,
        StreamRequest,
        UpdateStream,
        stream
    ))
}

async fn delete_stream(event: Id, directory: Directory) -> Result<impl warp::Reply, Infallible> {
    to_http_none_or_error(send_message!(
        directory.stream_actor,
        StreamRequest,
        DeleteStream,
        event.id
    ))
}

async fn get_hosts(directory: Directory) -> Result<impl warp::Reply, Infallible> {
    to_http_output(send_message!(directory.obs_actor, ObsCommand, GetState))
}

async fn set_streaming_state(
    streaming: SetStreamingState,
    directory: Directory,
) -> Result<impl warp::Reply, Infallible> {
    if streaming.streaming {
        to_http_none_or_error(send_message!(
            directory.obs_actor,
            ObsCommand,
            StartStream,
            streaming.host
        ))
    } else {
        to_http_none_or_error(send_message!(
            directory.obs_actor,
            ObsCommand,
            EndStream,
            streaming.host
        ))
    }
}

async fn commentary_endpoint(
    args: HashMap<String, String>,
    db: Arc<ProjectDb>,
) -> Result<impl warp::Reply, Infallible> {
    match get_event_by_args(args, &db).await {
        Ok(event) => match db.get_stream(event.id).await {
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

async fn run_dashboard_websocket(
    db: Arc<ProjectDb>,
    directory: Directory,
    socket: warp::ws::WebSocket,
    mut state_rx: Receiver<StateUpdate>,
) {
    log::info!("New dashboard websocket connection opened");
    let (mut tx, _) = socket.split();

    tokio::spawn(async move {});

    match assemble_state_update(db, &directory).await {
        Ok(update) => {
            if let Err(e) = tx
                .send(warp::ws::Message::text(
                    &serde_json::to_string(&update).unwrap(),
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
    let runners = db.get_runners().await?;
    for runner in &runners {
        if let Ok(run) = db.get_runner_run_data(runner.id).await {
            runs.insert(runner.name.clone(), run);
        }
    }

    let stream_names = db.get_streamed_events().await?;
    let mut streams = vec![];
    for stream in stream_names {
        streams.push(db.get_stream(stream).await?);
    }

    let layouts = db.get_layouts().await?;
    let hosts = send_message!(directory.obs_actor, ObsCommand, GetState)?;

    Ok(StateUpdate {
        events,
        runners,
        streams,
        layouts,
        active_runs: runs,
        hosts,
    })
}

pub async fn run_http_server(
    db: Arc<ProjectDb>,
    directory: Directory,
    mut rx: UnboundedReceiver<WebCommand>,
) -> Result<(), anyhow::Error> {
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

    let reader_tx = update_tx.clone();
    let socket = warp::path("ws")
        .and(warp::ws())
        .and(with_db(db.clone()))
        .and(with_directory(directory.clone()))
        .and(warp::any().map(move || update_tx.subscribe()))
        .and(warp::path::end())
        .map(
            |ws: warp::ws::Ws,
             db: Arc<ProjectDb>,
             directory: Directory,
             state_rx: Receiver<StateUpdate>| {
                ws.on_upgrade(move |socket| {
                    run_dashboard_websocket(db, directory, socket, state_rx)
                })
            },
        );

    let project_endpoint = warp::path("event")
        .and(warp::get())
        .and(warp::query::<HashMap<String, String>>())
        .and(with_db(db.clone()))
        .and(warp::path::end())
        .and_then(get_event_endpoint);

    let commentary_endpoint = warp::path("commentators")
        .and(warp::get())
        .and(warp::query::<HashMap<String, String>>())
        .and(with_db(db.clone()))
        .and(warp::path::end())
        .and_then(commentary_endpoint);

    let create_runner = warp::path("runner")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(create_runner);

    let update_runner = warp::path("runner")
        .and(warp::put())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(update_runner);

    let delete_runner = warp::path("runner")
        .and(warp::delete())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(delete_runner);

    let create_event = warp::path("event")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(create_event);

    let update_event = warp::path("event")
        .and(warp::put())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(update_event);

    let delete_event = warp::path("event")
        .and(warp::delete())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(delete_event);

    let create_stream = warp::path("stream")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(create_stream);

    let update_stream = warp::path("stream")
        .and(warp::put())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(update_stream);

    let delete_stream = warp::path("stream")
        .and(warp::delete())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(delete_stream);

    let get_hosts = warp::path("hosts")
        .and(warp::get())
        .and(with_directory(directory.clone()))
        .and_then(get_hosts);

    let set_streaming_state = warp::path("hosts")
        .and(warp::put())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(set_streaming_state);

    let dashboard = warp::path("static")
        .and(warp::get())
        .and(warp::fs::dir("web/static/timer.html"));

    tokio::spawn(async move {
        warp::serve(
            project_endpoint
                .or(commentary_endpoint)
                .or(dashboard)
                .or(socket)
                .or(create_runner)
                .or(update_runner)
                .or(delete_runner)
                .or(create_event)
                .or(update_event)
                .or(delete_event)
                .or(create_stream)
                .or(update_stream)
                .or(delete_stream)
                .or(get_hosts)
                .or(set_streaming_state)
                .with(cors),
        )
        .run(([0, 0, 0, 0], 28010))
        .await;
    });

    loop {
        match rx.recv().await.unwrap() {
            WebCommand::SendStateUpdate => {
                match assemble_state_update(db.clone(), &directory).await {
                    Ok(update) => {
                        let _ = reader_tx.send(update);
                    }
                    Err(e) => {
                        log::error!("Failed to assemble state update: {}", e);
                    }
                }
            }
        }
    }
}

fn with_db(
    db: Arc<ProjectDb>,
) -> impl Filter<Extract = (Arc<ProjectDb>,), Error = Infallible> + Clone {
    warp::any().map(move || db.clone())
}

fn with_directory(
    directory: Directory,
) -> impl Filter<Extract = (Directory,), Error = Infallible> + Clone {
    warp::any().map(move || directory.clone())
}
