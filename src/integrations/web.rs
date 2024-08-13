use crate::core::settings::Settings;
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
    obs::{ObsCommand, ObsHostState},
    therun::Run,
};

#[derive(Serialize, Clone, Debug)]
struct StateUpdate {
    streams: Vec<StreamState>,
    events: Vec<Event>,
    runners: HashMap<i64, Runner>,
    active_runs: HashMap<i64, Run>,
    hosts: HashMap<String, ObsHostState>,
    custom_fields: HashMap<String, Option<String>>,
}

/// A Json struct to store an event/runner ID
#[derive(Serialize, Deserialize, Debug)]
struct Id {
    id: i64,
}

/// A Json struct to store values for a new custom field
#[derive(Serialize, Deserialize, Debug)]
struct NewField {
    key: String,
}

/// A Json struct to store values for a new custom field
#[derive(Serialize, Deserialize, Debug)]
struct UpdateField {
    key: String,
    value: Option<String>,
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
        match event.parse::<i64>() {
            Ok(id) => match db.get_event(id).await {
                Ok(event) => Ok(event),
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
        match db.get_event_by_obs_host(host).await {
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

async fn create_event(event: Event, directory: Directory) -> Result<impl warp::Reply, Infallible> {
    to_http_none_or_error(send_message!(
        directory.event_actor,
        EventRequest,
        Create,
        event
    ))
}

async fn get_event(
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

async fn refresh_runner(runner: Id, directory: Directory) -> Result<impl warp::Reply, Infallible> {
    let refreshed = send_message!(
        directory.runner_actor,
        RunnerRequest,
        RefreshStream,
        runner.id
    );

    match refreshed {
        Ok(refreshed) => {
            Ok(warp::reply::with_status(
                serde_json::to_string::<bool>(&refreshed).unwrap(),
                warp::http::StatusCode::OK,
            ))
        }
        Err(e) => {
            Ok(warp::reply::with_status(
                e.to_string(),
                warp::http::StatusCode::INTERNAL_SERVER_ERROR,
            ))
        }
    }
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
    stream: StreamState,
    directory: Directory,
) -> Result<impl warp::Reply, Infallible> {
    let message = send_message!(
        directory.stream_actor,
        StreamRequest,
        Create,
        stream.event,
        stream.obs_host.clone()
    );

    if let Err(e) = message {
        to_http_none_or_error(Err(e))
    } else {
        to_http_none_or_error(send_message!(
            directory.stream_actor,
            StreamRequest,
            Update,
            stream
        ))
    }
}

async fn update_stream(
    stream: StreamState,
    directory: Directory,
) -> Result<impl warp::Reply, Infallible> {
    to_http_none_or_error(send_message!(
        directory.stream_actor,
        StreamRequest,
        Update,
        stream
    ))
}

async fn delete_stream(event: Id, directory: Directory) -> Result<impl warp::Reply, Infallible> {
    to_http_none_or_error(send_message!(
        directory.stream_actor,
        StreamRequest,
        Delete,
        event.id
    ))
}

async fn create_custom_field(
    field: NewField,
    db: Arc<ProjectDb>,
) -> Result<impl warp::Reply, Infallible> {
    to_http_none_or_error(db.add_custom_field(&field.key, None).await)
}

async fn update_custom_field(
    field: UpdateField,
    db: Arc<ProjectDb>,
) -> Result<impl warp::Reply, Infallible> {
    to_http_none_or_error(
        db.add_custom_field(&field.key, field.value.as_deref())
            .await,
    )
}

async fn delete_custom_field(
    field: NewField,
    db: Arc<ProjectDb>,
) -> Result<impl warp::Reply, Infallible> {
    to_http_none_or_error(db.clear_custom_field(&field.key).await)
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
    let runners: HashMap<i64, Runner> = db
        .get_runners()
        .await?
        .into_iter()
        .map(|r| (r.id, r))
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

    let hosts = send_message!(directory.obs_actor, ObsCommand, GetState)?;
    let custom_fields = db.get_custom_fields().await?;

    Ok(StateUpdate {
        events,
        runners,
        streams,
        active_runs: runs,
        hosts,
        custom_fields,
    })
}

pub async fn run_http_server(
    db: Arc<ProjectDb>,
    directory: Directory,
    settings: Arc<Settings>,
    mut rx: UnboundedReceiver<WebCommand>,
) -> Result<(), anyhow::Error> {
    let cors = warp::cors()
        .allow_any_origin()
        .allow_headers(vec![
            "User-Agent",
            "Sec-Fetch-Mode",
            "Referer",
            "Origin",
            "Content-Type",
            "Access-Control-Allow-Origin",
            "Access-Control-Request-Method",
            "Access-Control-Request-Headers",
            "Access-Control-Allow-Headers",
        ])
        .allow_methods(&[Method::GET, Method::POST, Method::PUT, Method::DELETE]);

    let (update_tx, _) = tokio::sync::broadcast::channel::<StateUpdate>(256);

    let reader_tx = update_tx.clone();
    let socket = warp::path("ws")
        .and(warp::path::end())
        .and(warp::ws())
        .and(with_db(db.clone()))
        .and(with_directory(directory.clone()))
        .and(warp::any().map(move || update_tx.subscribe()))
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

    let commentary_endpoint = warp::path("commentators")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<HashMap<String, String>>())
        .and(with_db(db.clone()))
        .and_then(commentary_endpoint);

    let create_runner = warp::path("runner")
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(create_runner);

    let update_runner = warp::path("runner")
        .and(warp::path::end())
        .and(warp::put())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(update_runner);

    let refresh_runner = warp::path("runner")
        .and(warp::path("refresh"))
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(refresh_runner);

    let delete_runner = warp::path("runner")
        .and(warp::path::end())
        .and(warp::delete())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(delete_runner);

    let create_event = warp::path("event")
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(create_event);

    let read_event = warp::path("event")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<HashMap<String, String>>())
        .and(with_db(db.clone()))
        .and(warp::path::end())
        .and_then(get_event);

    let update_event = warp::path("event")
        .and(warp::path::end())
        .and(warp::put())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(update_event);

    let delete_event = warp::path("event")
        .and(warp::path::end())
        .and(warp::delete())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(delete_event);

    let create_stream = warp::path("stream")
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(create_stream);

    let update_stream = warp::path("stream")
        .and(warp::path::end())
        .and(warp::put())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(update_stream);

    let delete_stream = warp::path("stream")
        .and(warp::path::end())
        .and(warp::delete())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(delete_stream);

    let get_hosts = warp::path("hosts")
        .and(warp::path::end())
        .and(warp::get())
        .and(with_directory(directory.clone()))
        .and_then(get_hosts);

    let set_streaming_state = warp::path("hosts")
        .and(warp::path::end())
        .and(warp::put())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(set_streaming_state);

    let create_field = warp::path("custom-field")
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::body::json())
        .and(with_db(db.clone()))
        .and_then(create_custom_field);

    let update_field = warp::path("custom-field")
        .and(warp::path::end())
        .and(warp::put())
        .and(warp::body::json())
        .and(with_db(db.clone()))
        .and_then(update_custom_field);

    let delete_field = warp::path("custom-field")
        .and(warp::path::end())
        .and(warp::delete())
        .and(warp::body::json())
        .and(with_db(db.clone()))
        .and_then(delete_custom_field);

    let dashboard = warp::path("static")
        .and(warp::get())
        .and(warp::fs::dir("web/static/timer.html"));

    tokio::spawn(async move {
        warp::serve(
            read_event
                .or(commentary_endpoint)
                .or(dashboard)
                .or(socket)
                .or(create_runner)
                .or(update_runner)
                .or(refresh_runner)
                .or(delete_runner)
                .or(create_event)
                .or(update_event)
                .or(delete_event)
                .or(create_stream)
                .or(update_stream)
                .or(delete_stream)
                .or(create_field)
                .or(update_field)
                .or(delete_field)
                .or(get_hosts)
                .or(set_streaming_state)
                .with(cors),
        )
        .run(([0, 0, 0, 0], settings.web_port.unwrap_or(28010)))
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
