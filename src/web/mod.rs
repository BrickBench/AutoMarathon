use crate::core::settings::Settings;
use std::{convert::Infallible, sync::Arc};

use dashboard::{websocket_filters, WebCommand};
use filters::api_filters;
use tokio::sync::mpsc::UnboundedReceiver;
use warp::{http::Method, reject::Rejection, Filter};

use crate::{core::db::ProjectDb, Directory};

pub mod dashboard;
pub mod filters;
pub mod handlers;

async fn handle_rejection(err: Rejection) -> Result<impl warp::Reply, Infallible> {
    let (code, msg) = if let Some(err) = err.find::<warp::filters::body::BodyDeserializeError>() {
        log::error!("{}", err);
        (warp::http::StatusCode::BAD_REQUEST, err.to_string())
    } else if let Some(err) = err.find::<warp::reject::MethodNotAllowed>() {
        log::error!("Method Not Allowed: {}", err);
        (warp::http::StatusCode::METHOD_NOT_ALLOWED, err.to_string())
    } else if let Some(err) = err.find::<warp::reject::InvalidQuery>() {
        log::error!("Invalid Query: {}", err);
        (warp::http::StatusCode::BAD_REQUEST, err.to_string())
    } else {
        log::error!("Unhandled Rejection: {:?}", err);
        (
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error".to_string(),
        )
    };

    Ok(warp::reply::with_status(warp::reply::json(&msg), code))
}

pub async fn run_http_server(
    db: Arc<ProjectDb>,
    directory: Directory,
    settings: Arc<Settings>,
    rx: UnboundedReceiver<WebCommand>,
) -> anyhow::Result<()> {
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
        .allow_methods(&[
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ]);

    let routes = api_filters(db.clone(), directory.clone())
        .or(websocket_filters(db, directory, rx))
        .recover(handle_rejection);

    warp::serve(routes.with(cors))
        .run(([0, 0, 0, 0], settings.web_port.unwrap_or(28010)))
        .await;

    Ok(())
}
