use std::{collections::HashMap, convert::Infallible, sync::Arc};

use serde::{Deserialize, Serialize};
use warp::reply::WithStatus;

use crate::{
    core::{
        db::ProjectDb,
        event::Event,
        runner::RunnerRequest,
        settings::Settings,
        stream::{StreamRequest, StreamState},
    },
    integrations::obs::HostCommand,
    send_message, Directory, Rto,
};

/// A Json struct to store an event/runner ID
#[derive(Serialize, Deserialize, Debug)]
pub struct Id {
    pub id: i64,
}

/// A Json struct to store values for a new custom field
#[derive(Serialize, Deserialize, Debug)]
pub struct NewField {
    pub key: String,
}

/// A Json struct to store values for a new custom field
#[derive(Serialize, Deserialize, Debug)]
pub struct UpdateField {
    pub key: String,
    pub value: Option<String>,
}

/// A Json struct to set the streaming state of an OBS host
#[derive(Serialize, Deserialize, Debug)]
pub struct SetStreamingState {
    pub host: String,
    pub streaming: bool,
}

/// A Json struct to set a Discord user's volume
#[derive(Serialize, Deserialize, Debug)]
pub struct SetDiscordUserVolume {
    pub user: String,
    pub volume: u32,
}

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

pub fn to_http_none_or_error(result: anyhow::Result<()>) -> Result<impl warp::Reply, Infallible> {
    match result {
        Ok(_) => Ok(warp::reply::with_status(
            "Success".to_string(),
            warp::http::StatusCode::OK,
        )),
        Err(e) => {
            log::warn!("{}", e);
            Ok(warp::reply::with_status(
                e.to_string(),
                warp::http::StatusCode::INTERNAL_SERVER_ERROR,
            ))
        }
    }
}

pub fn to_http_output<T: Serialize>(
    result: anyhow::Result<T>,
) -> Result<impl warp::Reply, Infallible> {
    match result {
        Ok(data) => Ok(warp::reply::with_status(
            serde_json::to_string::<T>(&data).unwrap(),
            warp::http::StatusCode::OK,
        )),
        Err(e) => {
            log::warn!("{}", e);
            Ok(warp::reply::with_status(
                e.to_string(),
                warp::http::StatusCode::INTERNAL_SERVER_ERROR,
            ))
        }
    }
}

pub async fn get_event(
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

pub async fn refresh_runner(
    runner: Id,
    directory: Directory,
) -> Result<impl warp::Reply, Infallible> {
    let refreshed = send_message!(
        directory.runner_actor,
        RunnerRequest,
        RefreshStream,
        runner.id
    );

    match refreshed {
        Ok(refreshed) => Ok(warp::reply::with_status(
            serde_json::to_string::<bool>(&refreshed).unwrap(),
            warp::http::StatusCode::OK,
        )),
        Err(e) => Ok(warp::reply::with_status(
            e.to_string(),
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        )),
    }
}
pub async fn create_stream(
    stream: StreamState,
    do_transition: bool,
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
            stream,
            do_transition
        ))
    }
}

pub async fn set_streaming_state(
    streaming: SetStreamingState,
    directory: Directory,
) -> Result<impl warp::Reply, Infallible> {
    if streaming.streaming {
        to_http_none_or_error(send_message!(
            directory.obs_actor,
            HostCommand,
            StartStream,
            streaming.host
        ))
    } else {
        to_http_none_or_error(send_message!(
            directory.obs_actor,
            HostCommand,
            EndStream,
            streaming.host
        ))
    }
}

pub enum RedirectOrString {
    Redirect(Box<dyn warp::Reply>),
    String(WithStatus<String>),
}

impl warp::Reply for RedirectOrString {
    fn into_response(self) -> warp::reply::Response {
        match self {
            RedirectOrString::Redirect(r) => r.into_response(),
            RedirectOrString::String(s) => s.into_response(),
        }
    }
}

pub async fn get_stream_redirect(
    host: Id,
    slot: Id,
    db: Arc<ProjectDb>,
    settings: Arc<Settings>,
) -> Result<impl warp::Reply, Infallible> {
    let stream_id = db.get_event_by_obs_host(&host.id.to_string()).await;
    if stream_id.is_err() {
        return Ok(RedirectOrString::String(warp::reply::with_status(
            "Failed to find stream for host".to_string(),
            warp::http::StatusCode::BAD_REQUEST,
        )));
    }
    let stream = db.get_stream(stream_id.unwrap()).await;
    if stream.is_err() {
        return Ok(RedirectOrString::String(warp::reply::with_status(
            "Failed to find stream".to_string(),
            warp::http::StatusCode::BAD_REQUEST,
        )));
    }

    let stream = stream.unwrap();
    let slot = stream.get_runner_in_slot(slot.id);
    if slot.is_none() {
        return Ok(RedirectOrString::String(warp::reply::with_status(
            "Failed to find runner in specified slot".to_string(),
            warp::http::StatusCode::BAD_REQUEST,
        )));
    }

    let runner = db.get_runner(slot.unwrap()).await.unwrap();
    let best_url = runner.calculate_best_url(1080, 30);
    match best_url {
        Some((url, _)) => Ok(RedirectOrString::Redirect(Box::new(
            warp::redirect::temporary(warp::http::Uri::from_maybe_shared(url).unwrap()),
        ))),
        None => Ok(RedirectOrString::String(warp::reply::with_status(
            "No stream URL".to_owned(),
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        ))),
    }
}
