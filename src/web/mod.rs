use crate::core::settings::Settings;
use std::sync::Arc;

use dashboard::{websocket_filters, WebCommand};
use filters::api_filters;
use tokio::sync::mpsc::UnboundedReceiver;
use warp::{http::Method, Filter};

use crate::{core::db::ProjectDb, Directory};

pub mod dashboard;
pub mod filters;
pub mod handlers;

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
        .allow_methods(&[Method::GET, Method::POST, Method::PUT, Method::DELETE]);

    warp::serve(
        api_filters(db.clone(), directory.clone())
            .or(websocket_filters(db, directory, rx))
            .with(cors),
    )
    .run(([0, 0, 0, 0], settings.web_port.unwrap_or(28010)))
    .await;

    Ok(())
}
