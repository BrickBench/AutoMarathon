use std::{convert::Infallible, collections::HashMap};

use tokio::sync::{mpsc::UnboundedSender, oneshot};
use warp::{http::Method, Filter};

use crate::{error::Error, CommandMessage, Response, cmd::CommandSource};

async fn process_req(tx: UnboundedSender<CommandMessage>) -> Result<impl warp::Reply, Infallible> {
    let (rtx, rrx) = oneshot::channel();
    let _ = tx.send((CommandSource::GetState, Some(rtx)));

    if let Ok(resp) = rrx.await {
        match resp {
            Response::Ok => Ok(warp::reply::with_status("Gaming".to_string(), warp::http::StatusCode::OK)),
            Response::Error(err) => Ok(warp::reply::with_status(err.to_string(), warp::http::StatusCode::INTERNAL_SERVER_ERROR)),
            Response::CurrentState(mut resp) => {
                let fields = &mut resp.player_fields.player_fields;
                let fields_list = fields.iter_mut()
                    .map(|(k, v)| {
                        v.insert("name".to_string(), k.to_string());
                        v.clone() 
                    }).collect::<Vec<HashMap<String, String>>>();

                Ok(warp::reply::with_status(serde_json::to_string(&fields_list).unwrap(), warp::http::StatusCode::OK))
            }
        }
    } else {
        Ok(warp::reply::with_status("Failed to request value".to_string(), warp::http::StatusCode::INTERNAL_SERVER_ERROR))
    }
}

pub async fn run_http_server(tx: UnboundedSender<CommandMessage>) -> Result<(), Error> {
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

    let user_endpoint = warp::path("users")
        .and(warp::path::end())
        .and_then(move || {
            let tx = tx.clone();
            
            async move { 
                process_req(tx).await 
            }
        });

    let dashboard = warp::path("dashboard")
        .and(warp::path::end())
        .and(warp::fs::file("web/dashboard.html"));

   warp::serve(user_endpoint.or(dashboard).with(cors))
        .run(([0, 0, 0, 0], 28010))
        .await;

    Ok(())
}
