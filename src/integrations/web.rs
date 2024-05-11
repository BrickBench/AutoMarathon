use std::{collections::HashMap, convert::Infallible, sync::Arc};

use serde::{Deserialize, Serialize};
use warp::{http::Method, Filter};

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

async fn project_endpoint(event: String, db: &ProjectDb) -> Result<impl warp::Reply, Infallible> {
    if let Ok(event) = db.get_event(&event).await {
        Ok(warp::reply::with_status(
            serde_json::to_string::<Event>(&event).unwrap(),
            warp::http::StatusCode::OK,
        ))
    } else {
        Ok(warp::reply::with_status(
            "Failed to find event by name".to_string(),
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        ))
    }
}

async fn commentary_endpoint(
    event: String,
    db: &ProjectDb,
) -> Result<impl warp::Reply, Infallible> {
    if let Ok(state) = db.get_stream(&event).await {
        Ok(warp::reply::with_status(
            serde_json::to_string(&state.get_commentators()).unwrap(),
            warp::http::StatusCode::OK,
        ))
    } else {
        Ok(warp::reply::with_status(
            "Failed to request value".to_string(),
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        ))
    }
}

pub async fn run_http_server(
    db: Arc<ProjectDb>,
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

    let db2 = db.clone();
    let project_endpoint = warp::path("project")
        .and(warp::query::<String>())
        .and(warp::path::end())
        .and_then(move |event| {
            let db2 = db2.clone();
            async move { project_endpoint(event, &db2).await }
        });

    let commentary_endpoint = warp::path("commentators")
        .and(warp::query::<String>())
        .and(warp::path::end())
        .and_then(move |event| {
            let db = db.clone();
            async move { commentary_endpoint(event, &db).await }
        });

    let dashboard = warp::path("dashboard")
        .and(warp::path::end())
        .and(warp::fs::file("web/dashboard.html"));

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
