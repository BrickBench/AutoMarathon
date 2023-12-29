use std::{collections::HashMap, convert::Infallible, sync::Arc};

use serde::{Deserialize, Serialize};
use warp::{http::Method, Filter};

use crate::{
    project::Project,
    state::{StateActor, StateRequest},
    Rto,
};

use super::therun::{Run, TheRunActor, TheRunRequest};

#[derive(Serialize, Deserialize)]
struct StateResponse {
    #[serde(flatten)]
    type_response: ProjectTypeResponse,
    runner_state: HashMap<String, PlayerResponse>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
enum ProjectTypeResponse {
    Marathon,
    Relay {
        start_time: u64,
        end_time: Option<u64>,
        team_behind_by: HashMap<String, u64>
    },
}

#[derive(Serialize, Deserialize)]
struct PlayerResponse {
    runner: String,
    is_visible: bool,
    live_stats: Option<Run>,

    #[serde(flatten)]
    type_response: ProjectTypePlayerResponse,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum ProjectTypePlayerResponse {
    Marathon { event_pb: Option<f64> },
    Relay { run_time: Option<String> },
}

async fn project_endpoint(project: &Project) -> Result<impl warp::Reply, Infallible> {
    Ok(warp::reply::with_status(
        serde_json::to_string(project).unwrap(),
        warp::http::StatusCode::OK,
    ))
}

async fn state_endpoint(
    project: &Project,
    state_actor: StateActor,
    therun_actor: Option<TheRunActor>,
) -> Result<impl warp::Reply, Infallible> {
    let (rtx, rrx) = Rto::new();
    state_actor.send(StateRequest::GetState(rtx));

    if let Ok(resp) = rrx.await {
        match resp {
            Ok(state) => {
                let mut player_responses = HashMap::<String, PlayerResponse>::new();

                for player in &project.players {
                    let player_type_response = match &state.event_state {
                        crate::state::ProjectTypeState::Marathon { event_pbs } => {
                            ProjectTypePlayerResponse::Marathon {
                                event_pb: event_pbs.get(&player.name).copied(),
                            }
                        }
                        crate::state::ProjectTypeState::Relay(relay_data) => {
                            ProjectTypePlayerResponse::Relay {
                                run_time: relay_data.runner_end_time.get(&player.name).cloned() 
                            }
                        }
                    };

                    let mut player_resp = PlayerResponse {
                        runner: player.name.clone(),
                        live_stats: None,
                        is_visible: state.is_active(player),
                        type_response: player_type_response,
                    };

                    if let Some(actor) = &therun_actor {
                        let (rtx2, rrx2) = Rto::new();
                        actor.send(TheRunRequest::GetLivePlayerStats(player.name.clone(), rtx2));

                        if let Ok(Ok(stats)) = rrx2.await {
                            player_resp.live_stats = stats;
                        }
                    }

                    player_responses.insert(player.name.clone(), player_resp);
                }

                let response_event_state = match &state.event_state {
                    crate::state::ProjectTypeState::Marathon { event_pbs: _ } => {
                        ProjectTypeResponse::Marathon
                    }
                    crate::state::ProjectTypeState::Relay(relay_state) => {
                        ProjectTypeResponse::Relay {
                            start_time: relay_state.start_date_time,
                            team_behind_by: relay_state.calculate_deltas_to_leader(project).clone(),
                            end_time: relay_state.end_date_time,
                        }
                    }
                };

                let response = StateResponse {
                    type_response: response_event_state,
                    runner_state: player_responses,
                };

                Ok(warp::reply::with_status(
                    serde_json::to_string(&response).unwrap(),
                    warp::http::StatusCode::OK,
                ))
            }
            Err(err) => Ok(warp::reply::with_status(
                err.to_string(),
                warp::http::StatusCode::INTERNAL_SERVER_ERROR,
            )),
        }
    } else {
        Ok(warp::reply::with_status(
            "Failed to request value".to_string(),
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        ))
    }
}

async fn commentary_endpoint(state_actor: StateActor) -> Result<impl warp::Reply, Infallible> {
    let (rtx, rrx) = Rto::new();
    state_actor.send(StateRequest::GetState(rtx));

    if let Ok(resp) = rrx.await {
        match resp {
            Ok(state) => Ok(warp::reply::with_status(
                serde_json::to_string(&state.get_commentators()).unwrap(),
                warp::http::StatusCode::OK,
            )),
            Err(err) => Ok(warp::reply::with_status(
                err.to_string(),
                warp::http::StatusCode::INTERNAL_SERVER_ERROR,
            )),
        }
    } else {
        Ok(warp::reply::with_status(
            "Failed to request value".to_string(),
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        ))
    }
}

pub async fn run_http_server(
    project: Arc<Project>,
    state_actor: StateActor,
    therun_actor: Option<TheRunActor>,
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

    let proj2 = project.clone();
    let project_endpoint = warp::path("project")
        .and(warp::path::end())
        .and_then(move || {
            let proj = proj2.clone();
            async move { project_endpoint(&proj).await }
        });

    let state_actor_2 = state_actor.clone();
    let commentary_endpoint = warp::path("commentators")
        .and(warp::path::end())
        .and_then(move || {
            let state_actor = state_actor_2.clone();
            async move { commentary_endpoint(state_actor).await }
        });

    let state_endpoint = warp::path("event_state")
        .and(warp::path::end())
        .and_then(move || {
            let state_actor = state_actor.clone();
            let therun_actor = therun_actor.clone();
            let proj = project.clone();

            async move { state_endpoint(&proj, state_actor, therun_actor).await }
        });

    let dashboard = warp::path("dashboard")
        .and(warp::path::end())
        .and(warp::fs::file("web/dashboard.html"));

    warp::serve(state_endpoint.or(project_endpoint).or(commentary_endpoint).or(dashboard).with(cors))
        .run(([0, 0, 0, 0], 28010))
        .await;

    Ok(())
}
