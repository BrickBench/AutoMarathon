use crate::core::settings::Settings;
use std::{convert::Infallible, sync::Arc};

use dashboard::{websocket_filters, WebCommand};
use filters::api_filters;
use tokio::sync::mpsc::UnboundedReceiver;
use warp::{
    filters::{cors::Builder, BoxedFilter},
    http::{Method, Response, Uri},
    reject::Rejection,
    Filter,
};

use crate::{core::db::ProjectDb, Directory};

pub mod dashboard;
pub mod filters;
pub mod handlers;

#[derive(vite_rs::Embed)]
#[root = "./web"]
struct Assets;

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

fn get_cors_settings() -> Builder {
    warp::cors()
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
        ])
}

#[cfg(not(debug_assertions))]
fn asset_to_filter(path: &str, asset: &vite_rs::ViteFile) -> BoxedFilter<(impl warp::Reply,)> {
    let asset = asset.clone();

    // hack, should have at most one slash
    let path_parts = path.split('/').collect::<Vec<_>>();
    assert!(path_parts.len() <= 2);

    if path_parts.len() == 2 {
        warp::path(path_parts[0].to_string())
            .and(warp::path(path_parts[1].to_string()))
            .and(warp::path::end())
            .map(move || {
                Response::builder()
                    .header("Content-Type", asset.content_type)
                    .header("Content-Length", asset.content_length)
                    .body(asset.bytes.clone())
                    .unwrap()
            })
            .boxed()
    } else {
        warp::path(path.to_string())
            .and(warp::path::end())
            .map(move || {
                Response::builder()
                    .header("Content-Type", asset.content_type)
                    .header("Content-Length", asset.content_length)
                    .body(asset.bytes.clone())
                    .unwrap()
            })
            .boxed()
    }
}

#[cfg(not(debug_assertions))]
pub async fn run_http_server(
    db: Arc<ProjectDb>,
    directory: Directory,
    settings: Arc<Settings>,
    rx: UnboundedReceiver<WebCommand>,
) -> anyhow::Result<()> {
    use vite_rs::ViteFile;

    let assets: Vec<(String, ViteFile)> = Assets::iter()
        .map(|asset| (asset.to_string(), Assets::get(&asset).unwrap()))
        .collect();
    /*
    let dashboard_routes = assets
        .into_iter()
        .map(|(path, asset)| asset_to_filter(&path, &asset))
        .fold(warp::any().map(warp::reply).boxed(), |routes, route| {
            routes.or(route).unify().boxed()
        }); */

    let dashboard_routes = asset_to_filter(assets[0].0.as_str(), &assets[0].1)
        .or(asset_to_filter(assets[1].0.as_str(), &assets[1].1))
        .or(asset_to_filter(assets[2].0.as_str(), &assets[2].1))
        .or(asset_to_filter(assets[3].0.as_str(), &assets[3].1));

    let routes = api_filters(db.clone(), directory.clone())
        .or(websocket_filters(db, directory, rx))
        .or(dashboard_routes)
        .or(warp::path::end().map(|| warp::redirect(Uri::from_static("/index.html"))))
        .recover(handle_rejection);

    warp::serve(routes.with(get_cors_settings()))
        .run(([0, 0, 0, 0], settings.web_port.unwrap_or(28010)))
        .await;

    Ok(())
}

#[cfg(debug_assertions)]
pub async fn run_http_server(
    db: Arc<ProjectDb>,
    directory: Directory,
    settings: Arc<Settings>,
    rx: UnboundedReceiver<WebCommand>,
) -> anyhow::Result<()> {
    let _guard = Assets::start_dev_server(true);

    let routes = api_filters(db.clone(), directory.clone())
        .or(websocket_filters(db, directory, rx))
        .recover(handle_rejection);

    warp::serve(routes.with(get_cors_settings()))
        .run(([0, 0, 0, 0], settings.web_port.unwrap_or(28010)))
        .await;

    Ok(())
}
