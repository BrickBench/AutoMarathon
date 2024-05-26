use std::{collections::HashMap, convert::Infallible, sync::Arc};

use serde::{Deserialize, Serialize};
use warp::{http::Method, reply::WithStatus, Filter};

use crate::{db::ProjectDb, event::Event};

use super::therun::Run;

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
    db: &ProjectDb,
) -> Result<impl warp::Reply, Infallible> {
    match get_event_by_args(args, db).await {
        Ok(event) => Ok(warp::reply::with_status(
            serde_json::to_string::<Event>(&event).unwrap(),
            warp::http::StatusCode::OK,
        )),
        Err(reply) => Ok(reply),
    }
}

async fn commentary_endpoint(
    args: HashMap<String, String>,
    db: &ProjectDb,
) -> Result<impl warp::Reply, Infallible> {
    match get_event_by_args(args, db).await {
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

    let db2 = db.clone();
    let project_endpoint = warp::path("event")
        .and(warp::query::<HashMap<String, String>>())
        .and(warp::path::end())
        .and_then(move |args| {
            let db2 = db2.clone();
            async move { event_endpoint(args, &db2).await }
        });

    let commentary_endpoint = warp::path("commentators")
        .and(warp::query::<HashMap<String, String>>())
        .and(warp::path::end())
        .and_then(move |args| {
            let db = db.clone();
            async move { commentary_endpoint(args, &db).await }
        });

    let dashboard = warp::path("timer")
        .and(warp::path::end())
        .and(warp::fs::file("web/timer.html"));

    warp::serve(
        project_endpoint
            .or(commentary_endpoint)
            .or(dashboard)
            .with(cors),
    )
    .run(([0, 0, 0, 0], 28010))
    .await;

    Ok(())
}
