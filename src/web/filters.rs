use std::{collections::HashMap, convert::Infallible, sync::Arc};

use warp::{reject::Rejection, Filter};

use crate::{
    core::{
        db::ProjectDb,
        event::{Event, EventRequest},
        participant::Participant,
        runner::{Runner, RunnerRequest},
        stream::{StreamRequest, StreamState},
    },
    integrations::obs::HostCommand,
    send_message, Directory, Rto,
};

use super::handlers::{
    create_stream, get_event, refresh_runner, set_streaming_state, to_http_none_or_error,
    to_http_output, Id, NewField, SetDiscordUserVolume, UpdateField,
};

pub fn with_db(
    db: Arc<ProjectDb>,
) -> impl Filter<Extract = (Arc<ProjectDb>,), Error = Infallible> + Clone {
    warp::any().map(move || db.clone())
}

pub fn with_directory(
    directory: Directory,
) -> impl Filter<Extract = (Directory,), Error = Infallible> + Clone {
    warp::any().map(move || directory.clone())
}

fn participant_filters(
    db: Arc<ProjectDb>,
) -> impl Filter<Extract = (impl warp::Reply,), Error = Rejection> + Clone {
    let create_participant = warp::path!("participant")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_db(db.clone()))
        .and_then(async |participant: Participant, db: Arc<ProjectDb>| {
            to_http_none_or_error(db.add_participant(&mut participant.clone()).await)
        });

    let update_participant = warp::path!("participant")
        .and(warp::put())
        .and(warp::body::json())
        .and(with_db(db.clone()))
        .and_then(async |participant: Participant, db: Arc<ProjectDb>| {
            to_http_none_or_error(db.update_participant(&participant).await)
        });

    let delete_participant = warp::path!("participant")
        .and(warp::delete())
        .and(warp::body::json())
        .and(with_db(db.clone()))
        .and_then(async |participant: Id, db: Arc<ProjectDb>| {
            to_http_none_or_error(db.delete_participant(participant.id).await)
        });

    create_participant
        .or(update_participant)
        .or(delete_participant)
}

fn runner_filters(
    directory: Directory,
) -> impl Filter<Extract = (impl warp::Reply,), Error = Rejection> + Clone {
    let create_runner = warp::path!("runner")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(async |runner: Runner, directory: Directory| {
            to_http_none_or_error(send_message!(
                directory.runner_actor,
                RunnerRequest,
                Create,
                runner
            ))
        });

    let update_runner = warp::path!("runner")
        .and(warp::put())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(async |runner: Runner, directory: Directory| {
            to_http_none_or_error(send_message!(
                directory.runner_actor,
                RunnerRequest,
                Update,
                runner
            ))
        });

    let refresh_runner = warp::path!("runner" / "refresh")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(refresh_runner);

    let delete_runner = warp::path!("runner")
        .and(warp::delete())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(async |runner: Id, directory: Directory| {
            to_http_none_or_error(send_message!(
                directory.runner_actor,
                RunnerRequest,
                Delete,
                runner.id
            ))
        });

    create_runner
        .or(update_runner)
        .or(refresh_runner)
        .or(delete_runner)
}

fn event_filters(
    db: Arc<ProjectDb>,
    directory: Directory,
) -> impl Filter<Extract = (impl warp::Reply,), Error = Rejection> + Clone {
    let create_event = warp::path!("event")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(async |event: Event, directory: Directory| {
            to_http_none_or_error(send_message!(
                directory.event_actor,
                EventRequest,
                Create,
                event
            ))
        });

    let read_event = warp::path!("event")
        .and(warp::get())
        .and(warp::query::<HashMap<String, String>>())
        .and(with_db(db.clone()))
        .and(warp::path::end())
        .and_then(get_event);

    let update_event = warp::path!("event")
        .and(warp::put())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(async |event: Event, directory: Directory| {
            to_http_none_or_error(send_message!(
                directory.event_actor,
                EventRequest,
                Update,
                event
            ))
        });

    let delete_event = warp::path!("event")
        .and(warp::delete())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(async |event: Id, directory: Directory| {
            to_http_none_or_error(send_message!(
                directory.event_actor,
                EventRequest,
                Delete,
                event.id
            ))
        });

    create_event
        .or(read_event)
        .or(update_event)
        .or(delete_event)
}

fn stream_filters(
    directory: Directory,
) -> impl Filter<Extract = (impl warp::Reply,), Error = Rejection> + Clone {
    let create_stream = warp::path!("stream")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(create_stream);

    let update_stream = warp::path!("stream")
        .and(warp::put())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(async |stream: StreamState, directory: Directory| {
            to_http_none_or_error(send_message!(
                directory.stream_actor,
                StreamRequest,
                Update,
                stream
            ))
        });

    let delete_stream = warp::path!("stream")
        .and(warp::delete())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(async |event: Id, directory: Directory| {
            to_http_none_or_error(send_message!(
                directory.stream_actor,
                StreamRequest,
                Delete,
                event.id
            ))
        });

    create_stream.or(update_stream).or(delete_stream)
}

pub fn api_filters(
    db: Arc<ProjectDb>,
    directory: Directory,
) -> impl Filter<Extract = (impl warp::Reply,), Error = Rejection> + Clone {
    let get_hosts = warp::path!("hosts")
        .and(warp::get())
        .and(with_directory(directory.clone()))
        .and_then(async |dir: Directory| {
            to_http_output(send_message!(dir.obs_actor, HostCommand, GetState))
        });

    let set_streaming_state = warp::path!("hosts")
        .and(warp::put())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(set_streaming_state);

    let create_field = warp::path!("custom-field")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_db(db.clone()))
        .and_then(async |field: NewField, db: Arc<ProjectDb>| {
            to_http_none_or_error(db.add_custom_field(&field.key, None).await)
        });

    let update_field = warp::path!("custom-field")
        .and(warp::put())
        .and(warp::body::json())
        .and(with_db(db.clone()))
        .and_then(async |field: UpdateField, db: Arc<ProjectDb>| {
            to_http_none_or_error(
                db.add_custom_field(&field.key, field.value.as_deref())
                    .await,
            )
        });

    let delete_field = warp::path!("custom-field")
        .and(warp::delete())
        .and(warp::body::json())
        .and(with_db(db.clone()))
        .and_then(async |field: NewField, db: Arc<ProjectDb>| {
            to_http_none_or_error(db.clear_custom_field(&field.key).await)
        });

    let set_discord_volume = warp::path!("discord" / "volume")
        .and(warp::put())
        .and(warp::body::json())
        .and(with_directory(directory.clone()))
        .and_then(
            async |volume_req: SetDiscordUserVolume, directory: Directory| {
                to_http_none_or_error(send_message!(
                    directory.obs_actor,
                    HostCommand,
                    SetCommentatorVolume,
                    volume_req.user,
                    volume_req.volume
                ))
            },
        );

    let dashboard = warp::path::end().and(warp::fs::dir("web/"));

    get_hosts
        .or(set_streaming_state)
        .or(create_field)
        .or(update_field)
        .or(delete_field)
        .or(dashboard)
        .or(set_discord_volume)
        .or(participant_filters(db.clone()))
        .or(runner_filters(directory.clone()))
        .or(event_filters(db.clone(), directory.clone()))
        .or(stream_filters(directory))
}
