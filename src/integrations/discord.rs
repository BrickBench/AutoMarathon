use std::{collections::{HashMap, HashSet}, sync::Arc};

use anyhow::anyhow;
use poise::serenity_prelude as serenity;

use futures::{Stream, StreamExt};
use serenity::{
    http::Http,
    model::{
        prelude::{ChannelId, GuildChannel},
        voice::VoiceState,
    },
};
use sqlx::types::time::OffsetDateTime;

use crate::{
    core::{
        db::ProjectDb,
        event::{Event, EventRequest, RunnerEventState},
        runner::{Runner, RunnerRequest},
        settings::Settings,
        stream::{validate_streamed_event_id, StreamActor, StreamRequest},
    },
    error::Error,
    integrations::obs::ObsCommand,
    send_message, Directory, Rto,
};

struct Data {
    db: Arc<ProjectDb>,
    settings: Arc<Settings>,
    directory: Directory,
}

type Context<'a> = poise::Context<'a, Data, anyhow::Error>;

/// Returns the GuildChannel for the provided voice
async fn get_voice_guild_channel(
    state: &VoiceState,
    context: &serenity::Context,
) -> Option<GuildChannel> {
    let channel_id = state.channel_id?;

    to_guild_channel(channel_id, context).await
}

/// Convert a ChannelID to a GuildChannel
async fn to_guild_channel(
    channel_id: ChannelId,
    context: &serenity::Context,
) -> Option<GuildChannel> {
    match channel_id.to_channel(&context).await {
        Ok(channel) => channel.guild(),
        Err(why) => {
            log::error!("Err w/ channel {}", why);

            None
        }
    }
}

/// Return the event ID corresponding to the given name, or None if None
async fn get_event_id(name: Option<String>, db: &ProjectDb) -> anyhow::Result<Option<i64>> {
    match name {
        Some(name) => Ok(Some(db.get_id_for_event(&name).await?)),
        None => Ok(None),
    }
}

/// Return the stream ID for the given name, or try retrieving the single active stream
async fn get_stream_id(name: Option<String>, db: &ProjectDb) -> anyhow::Result<i64> {
    match get_event_id(name, db).await {
        Ok(name) => match validate_streamed_event_id(db, name).await {
            Ok(id) => Ok(id),
            Err(e) => Err(e),
        },
        Err(e) => Err(e),
    }
}

async fn update_voice_list(
    db: &ProjectDb,
    context: &serenity::Context,
    actor: &StreamActor,
    voice_state: &VoiceState,
    settings: &Settings,
) {
    if let Some(channel) = get_voice_guild_channel(voice_state, context).await {
        if let Some(host) = settings
            .obs_hosts
            .iter()
            .find(|h| {
                h.1.discord_voice_channel
                    .as_ref()
                    .is_some_and(|name| name == channel.name())
            })
            .map(|m| m.0.to_owned())
        {
            if let Ok(stream) = db.get_event_by_obs_host(&host).await {
                let users = channel.members(&context).await.unwrap();

                let user_list: Vec<String> =
                    users.iter().map(|u| u.display_name().to_string()).collect();

                let mut stream_data = db.get_stream(stream).await.expect("Stream not found");
                stream_data.active_commentators = user_list.join(";");

                let resp = send_message!(actor, StreamRequest, Update, stream_data);

                match resp {
                    Ok(_) => {}
                    Err(message) => {
                        log::error!("Failed to set voice state: {}", message);
                    }
                }
            }
        }
    }
}

async fn handle_voice_state_event(
    context: &serenity::Context,
    old_state: &Option<VoiceState>,
    new_state: &VoiceState,
    data: &Data,
) {
    let db = &data.db;
    let settings = &data.settings;
    let actor = &data.directory.stream_actor;

    if let Some(old_state) = old_state {
        update_voice_list(db, context, actor, old_state, settings).await;
    }

    update_voice_list(db, context, actor, new_state, settings).await;
}

async fn check_channel(context: &Context<'_>) -> anyhow::Result<bool> {
    let settings = &context.data().settings;
    let _ = context.defer().await;

    let _msg_channel = to_guild_channel(context.channel_id(), context.serenity_context()).await;

    // Check for desired channel
    if let Some(channel) = _msg_channel {
        if let Some(desired_channel) = &settings.discord_command_channel {
            if !channel.name().eq_ignore_ascii_case(desired_channel) {
                log::debug!("Command ignored, incorrect channel '{}'", channel.name());
                return if let Err(why) = context.say("Commands are not read on this channel.").await
                {
                    Err(anyhow!(why.to_string()))
                } else {
                    Ok(false)
                };
            }
        }
    }

    Ok(true)
}

async fn send_success_reply(context: &Context<'_>) -> Result<(), anyhow::Error> {
    if let Err(why) = context.say("\u{1F44D}").await {
        log::warn!("Failed to react: {}", why);
        Err(Box::new(Error::Unknown(why.to_string())).into())
    } else {
        Ok(())
    }
}

/// Create an autocomplete stream that matches streamed events
async fn autocomplete_streamed_event_name<'a>(
    ctx: Context<'_>,
    partial: &'a str,
) -> impl Stream<Item = String> + 'a {
    let events: Vec<String> = ctx.data().db.get_streamed_event_names().await.unwrap();

    futures::stream::iter(events)
        .filter(move |name| {
            futures::future::ready(name.to_lowercase().starts_with(&partial.to_lowercase()))
        })
        .map(|name| name.to_string())
}

/// Create an autocomplete stream that matches events
async fn autocomplete_event_name<'a>(
    ctx: Context<'_>,
    partial: &'a str,
) -> impl Stream<Item = String> + 'a {
    let runners: Vec<String> = ctx.data().db.get_event_names().await.unwrap();

    futures::stream::iter(runners)
        .filter(move |name| {
            futures::future::ready(name.to_lowercase().starts_with(&partial.to_lowercase()))
        })
        .map(|name| name.to_string())
}

/// Create an autocomplete stream that matches runner names
async fn autocomplete_runner_name<'a>(
    ctx: Context<'_>,
    partial: &'a str,
) -> impl Stream<Item = String> + 'a {
    let runners: Vec<String> = ctx
        .data()
        .db
        .get_runners()
        .await
        .unwrap()
        .iter()
        .map(|p| p.name.clone())
        .collect();

    futures::stream::iter(runners)
        .filter(move |name| {
            futures::future::ready(name.to_lowercase().starts_with(&partial.to_lowercase()))
        })
        .map(|name| name.to_string())
}

/// Create an autocomplete stream that matches layout names
async fn autocomplete_obs_name<'a>(
    ctx: Context<'_>,
    partial: &'a str,
) -> impl Stream<Item = String> + 'a {
    let hosts: Vec<String> = ctx.data().settings.obs_hosts.keys().cloned().collect();

    futures::stream::iter(hosts)
        .filter(move |name| futures::future::ready(name.starts_with(partial)))
        .map(|name| name.to_string())
}

/// Toggle the visibility for a specific runner.
///
/// ```
/// /toggle javster101
/// ```
#[poise::command(prefix_command, slash_command)]
async fn toggle(
    context: Context<'_>,
    #[description = "Runner to toggle"]
    #[autocomplete = "autocomplete_runner_name"]
    runner: String,
    #[description = "Event for this command"]
    #[autocomplete = "autocomplete_streamed_event_name"]
    event: Option<String>,
) -> Result<(), anyhow::Error> {
    let stream_id = get_stream_id(event.clone(), &context.data().db).await?;
    let mut stream = context.data().db.get_stream(stream_id).await?;
    let runner = context.data().db.find_runner(&runner).await?;

    match stream.get_runner_slot(runner.id) {
        Some(pos) => {
            stream.stream_runners.remove(&pos);
        }
        None => {
            stream.stream_runners.insert(stream.get_first_empty_slot(), runner.id);
        }
    }
       
    send_message!(
        &context.data().directory.stream_actor,
        StreamRequest,
        Update,
        stream
    )?;

    send_success_reply(&context).await
}

/// Refresh the stream of an active runner, or all runners if no names are provided.
///
/// This reacquires the stream for a certain Twitch user. This should
/// automatically happen, but this command can be used if AutoMarathon
/// fails to or if a stream fails while a runner is active.
/// ```
/// /refresh
/// /refresh javster101
/// ```
#[poise::command(prefix_command, slash_command)]
async fn refresh(
    context: Context<'_>,
    #[autocomplete = "autocomplete_runner_name"]
    #[description = "Runner to refresh"]
    runner: String,
    #[description = "Event for this command"]
    #[autocomplete = "autocomplete_streamed_event_name"]
    event: Option<String>,
) -> Result<(), anyhow::Error> {
    let stream_id = get_stream_id(event.clone(), &context.data().db).await?;
    let runner = context.data().db.find_runner(&runner).await?;
    send_message!(
        &context.data().directory.runner_actor,
        RunnerRequest,
        RefreshStream,
        runner.id
    )?;
    send_message!(
        &context.data().directory.stream_actor,
        StreamRequest,
        Reload,
        stream_id
    )?;
    send_success_reply(&context).await
}

/// Swap two runners.
///
/// This can be used to swap a runner with another in view, and to
/// replace an onscreen runner with an offscreen one.
/// ```
/// /swap javster101 p53
/// ```
#[poise::command(prefix_command, slash_command)]
async fn swap(
    context: Context<'_>,
    #[description = "First runner"]
    #[autocomplete = "autocomplete_runner_name"]
    runner1: String,
    #[description = "Second runner"]
    #[autocomplete = "autocomplete_runner_name"]
    runner2: String,
    #[description = "Event for this command"]
    #[autocomplete = "autocomplete_streamed_event_name"]
    event: Option<String>,
) -> Result<(), anyhow::Error> {
    let stream_id = get_stream_id(event.clone(), &context.data().db).await?;
    let mut stream = context.data().db.get_stream(stream_id).await?;

    let runner1 = context.data().db.find_runner(&runner1).await?;
    let runner2 = context.data().db.find_runner(&runner2).await?;

    let pos_p1 = stream.get_runner_slot(runner1.id);
    let pos_p2 = stream.get_runner_slot(runner2.id);

    if let (Some(pos_p1), Some(pos_p2)) = (pos_p1, pos_p2) {
        stream.stream_runners.insert(pos_p1, runner2.id);
        stream.stream_runners.insert(pos_p2, runner1.id);
    } else if let Some(pos_p1) = pos_p1 {
        stream.stream_runners.insert(pos_p1, runner2.id);
    } else if let Some(pos_p2) = pos_p2 {
        stream.stream_runners.insert(pos_p2, runner1.id);
    }

    send_message!(
        &context.data().directory.stream_actor,
        StreamRequest,
        Update,
        stream
    )?;

    send_success_reply(&context).await
}

/// Enable a certain layout.
///
/// If the provided layout does not support the current amount
/// of enabled runners, it will be enabled whenever the supported amount
/// of runners is enabled.
/// ```
/// /layout 4_runners
/// ```
#[poise::command(prefix_command, slash_command)]
async fn layout(
    context: Context<'_>,
    #[description = "Layout to use"]
    //  #[autocomplete = "autocomplete_layout_name"]
    layout: String,
    #[description = "Event for this command"]
    #[autocomplete = "autocomplete_streamed_event_name"]
    event: Option<String>,
) -> Result<(), anyhow::Error> {
    let stream_id = get_stream_id(event.clone(), &context.data().db).await?;
    let mut stream = context.data().db.get_stream(stream_id).await?;
    stream.requested_layout = Some(layout.clone());
    send_message!(
        &context.data().directory.stream_actor,
        StreamRequest,
        Update,
        stream
    )?;
    send_success_reply(&context).await
}

/// Set the active runners.
///
/// ```
/// /set
/// /set javster101 p53
/// ```
#[poise::command(prefix_command, slash_command)]
async fn set(
    context: Context<'_>,
    #[description = "Runners to enable"] runners: Option<String>,
    #[description = "Event for this command"]
    #[autocomplete = "autocomplete_streamed_event_name"]
    event: Option<String>,
) -> Result<(), anyhow::Error> {
    let runner_ids = match runners {
        Some(runners) => {
            let mut res = vec![];
            let runners: Vec<String> = runners.split(',').map(|s| s.trim().to_owned()).collect();
            for runner in runners {
                res.push(context.data().db.find_runner(&runner).await?.id);
            }

            res
        }
        None => {
            vec![]
        }
    };
    let stream_id = get_stream_id(event.clone(), &context.data().db).await?;
    let mut stream = context.data().db.get_stream(stream_id).await?;
    stream.stream_runners = runner_ids.iter().enumerate().map(|(i, r)| ((i as i64), *r)).collect();

    send_message!(
        &context.data().directory.stream_actor,
        StreamRequest,
        Update,
        stream
    )?;

    send_success_reply(&context).await
}

/// Set a list of commentator names to ignore.
///
/// This command can be used to stop 'listener' users from appearing
/// as commentators. The provided names should be the user's nicknames
/// (those that appear in the voice channel).
/// ```
/// /ignore streamer_bot
/// ```
#[poise::command(prefix_command, slash_command)]
async fn ignore(
    context: Context<'_>,
    #[description = "Names to ignore"] ignored: Option<String>,
    #[description = "Event for this command"]
    #[autocomplete = "autocomplete_streamed_event_name"]
    event: Option<String>,
) -> Result<(), anyhow::Error> {
    let stream_id = get_stream_id(event.clone(), &context.data().db).await?;
    let mut stream = context.data().db.get_stream(stream_id).await?;
    stream.ignored_commentators = ignored.unwrap_or_default();
    send_message!(
        &context.data().directory.stream_actor,
        StreamRequest,
        Update,
        stream
    )?;
    send_success_reply(&context).await
}

/// Create a stream for an event.
#[poise::command(prefix_command, slash_command)]
async fn create_stream(
    context: Context<'_>,
    #[description = "Event to stream"]
    #[autocomplete = "autocomplete_event_name"]
    event: String,
    #[description = "OBS host to use"]
    #[autocomplete = "autocomplete_obs_name"]
    host: String,
) -> Result<(), anyhow::Error> {
    let event = context.data().db.get_id_for_event(&event).await?;
    send_message!(
        &context.data().directory.stream_actor,
        StreamRequest,
        Create,
        event,
        host
    )?;
    send_success_reply(&context).await
}

/// Stop a stream related to an event.
///
/// This allows the OBS host this stream is associated with to be used again.
#[poise::command(prefix_command, slash_command)]
async fn delete_stream(
    context: Context<'_>,
    #[description = "Stream to delete"]
    #[autocomplete = "autocomplete_streamed_event_name"]
    event: String,
) -> Result<(), anyhow::Error> {
    let event = context.data().db.get_id_for_event(&event).await?;
    send_message!(
        &context.data().directory.stream_actor,
        StreamRequest,
        Delete,
        event
    )?;
    send_success_reply(&context).await
}

/// Start the timer now.
///
/// This may be a bit off due to input delay. For more accurate input, set a Unix timestamp with
/// set_start_time.
///
/// ```
/// /start_timer
/// ```
#[poise::command(prefix_command, slash_command)]
async fn start_timer(
    context: Context<'_>,
    #[description = "Event for this command"]
    #[autocomplete = "autocomplete_event_name"]
    event: String,
) -> Result<(), anyhow::Error> {
    let time = OffsetDateTime::now_utc();
    let event = context.data().db.get_id_for_event(&event).await?;
    send_message!(
        &context.data().directory.event_actor,
        EventRequest,
        SetStartTime,
        event,
        Some(time)
    )?;
    send_success_reply(&context).await
}

/// Stop the timer now.
///
/// This may be a bit off due to input delay. For more accurate input, set a Unix timestamp with
/// set_end_time.
///
/// ```
/// /stop_timer
/// ```
#[poise::command(prefix_command, slash_command)]
async fn stop_timer(
    context: Context<'_>,
    #[description = "Event for this command"]
    #[autocomplete = "autocomplete_event_name"]
    event: String,
) -> Result<(), anyhow::Error> {
    let time = OffsetDateTime::now_utc();
    let event = context.data().db.get_id_for_event(&event).await?;
    send_message!(
        &context.data().directory.event_actor,
        EventRequest,
        SetEndTime,
        event,
        Some(time)
    )?;
    send_success_reply(&context).await
}

/// Set the start time for a race as a Unix millis timestamp.
///
/// ```
/// /set_start_time 1701125546192
/// ```
#[poise::command(prefix_command, slash_command)]
async fn set_start_time(
    context: Context<'_>,
    #[description = "Event start time in Unix time"] time: Option<u64>,
    #[description = "Event for this command"]
    #[autocomplete = "autocomplete_event_name"]
    event: String,
) -> Result<(), anyhow::Error> {
    let event = context.data().db.get_id_for_event(&event).await?;
    let time = time.and_then(|t| OffsetDateTime::from_unix_timestamp(t.try_into().unwrap()).ok());

    send_message!(
        &context.data().directory.event_actor,
        EventRequest,
        SetStartTime,
        event,
        time
    )?;
    send_success_reply(&context).await
}

/// Set the end time for a race as a Unix millis timestamp.
///
/// ```
/// /set_relay_end 1701125546192
/// ```
#[poise::command(prefix_command, slash_command)]
async fn set_end_time(
    context: Context<'_>,
    #[description = "Event start time in Unix time"] time: Option<u64>,
    #[description = "Event for this command"]
    #[autocomplete = "autocomplete_event_name"]
    event: String,
) -> Result<(), anyhow::Error> {
    let event = context.data().db.get_id_for_event(&event).await?;
    let time = time.and_then(|t| OffsetDateTime::from_unix_timestamp(t.try_into().unwrap()).ok());

    send_message!(
        &context.data().directory.event_actor,
        EventRequest,
        SetEndTime,
        event,
        time
    )?;
    send_success_reply(&context).await
}

/// Start the OBS stream.
#[poise::command(prefix_command, slash_command)]
async fn start_stream(
    context: Context<'_>,
    #[description = "Event for this command"]
    #[autocomplete = "autocomplete_obs_name"]
    host: String,
) -> Result<(), anyhow::Error> {
    let _ = context.defer().await;
    send_message!(
        &context.data().directory.obs_actor,
        ObsCommand,
        StartStream,
        host
    )?;
    send_success_reply(&context).await
}

/// Stop the OBS stream.
#[poise::command(prefix_command, slash_command)]
async fn stop_stream(
    context: Context<'_>,
    #[description = "Event for this command"]
    #[autocomplete = "autocomplete_obs_name"]
    host: String,
) -> Result<(), anyhow::Error> {
    let _ = context.defer().await;

    send_message!(
        &context.data().directory.obs_actor,
        ObsCommand,
        EndStream,
        host
    )?;

    send_success_reply(&context).await
}

/// Create a new event.
#[poise::command(prefix_command, slash_command)]
async fn create_event(
    context: Context<'_>,
    #[description = "Name for this event"] event_name: String,
    #[description = "Runners to add to this event"] runners: Option<String>,
) -> Result<(), anyhow::Error> {
    log::debug!("Creating event {}", event_name);
    let mut new_event = Event {
        id: -1,
        name: event_name,
        game: None,
        category: None,
        estimate: None,
        therun_race_id: None,
        event_start_time: None,
        timer_start_time: None,
        timer_end_time: None,
        is_relay: false,
        is_marathon: false,
        preferred_layouts: vec![],
        tournament: None,
        runner_state: HashMap::new(),
    };

    let db = &context.data().db;
    let runners_names_list: Vec<_> = runners
        .map(|r| r.split(',').map(|s| s.trim().to_owned()).collect())
        .unwrap_or_default();

    for name in runners_names_list {
        let runner = db.find_runner(&name).await?.id;
        new_event.runner_state.insert(runner, RunnerEventState {
            runner,
            result: None,
        });
    }

    send_message!(
        &context.data().directory.event_actor,
        EventRequest,
        Create,
        new_event.clone()
    )?;

    send_success_reply(&context).await
}

/// Remove an event
#[poise::command(prefix_command, slash_command)]
async fn delete_event(
    context: Context<'_>,
    #[description = "Name for this event"]
    #[autocomplete = "autocomplete_event_name"]
    event_name: String,
) -> Result<(), anyhow::Error> {
    log::debug!("Deleting event {}", event_name);
    let event = context.data().db.get_id_for_event(&event_name).await?;

    send_message!(
        context.data().directory.event_actor,
        EventRequest,
        Delete,
        event
    )?;

    send_success_reply(&context).await
}

/// Create a new runner.
#[poise::command(prefix_command, slash_command)]
async fn create_runner(
    context: Context<'_>,
    #[description = "Name for this runner"] name: String,
    #[description = "TheRun.gg username"] therun: Option<String>,
    #[description = "Twitch username/stream link"] stream: Option<String>,
    #[description = "Nicknames for this runner"] nicknames: Option<String>,
) -> Result<(), anyhow::Error> {
    log::debug!("Creating runner {}", name);
    let nicknames: Vec<_> = nicknames
        .map(|r| r.split(',').map(|s| s.trim().to_owned()).collect())
        .unwrap_or_default();

    let runner = Runner {
        id: -1,
        name,
        stream,
        therun,
        cached_stream_url: None,
        volume_percent: 50,
        location: None,
        photo: None,
        nicks: nicknames,
    };

    send_message!(
        &context.data().directory.runner_actor,
        RunnerRequest,
        Create,
        runner.clone()
    )?;

    send_success_reply(&context).await
}

/// Delete a runner.
#[poise::command(prefix_command, slash_command)]
async fn delete_runner(
    context: Context<'_>,
    #[description = "Name for this runner"]
    #[autocomplete = "autocomplete_runner_name"]
    name: String,
) -> Result<(), anyhow::Error> {
    let runner = context.data().db.find_runner(&name).await?;
    send_message!(
        &context.data().directory.runner_actor,
        RunnerRequest,
        Delete,
        runner.id
    )?;

    send_success_reply(&context).await
}

/// Set the audible runner.
#[poise::command(prefix_command, slash_command)]
async fn set_audible_runner(
    context: Context<'_>,
    #[description = "Name for this runner"]
    #[autocomplete = "autocomplete_runner_name"]
    name: String,
    #[description = "Event for this command"]
    #[autocomplete = "autocomplete_streamed_event_name"]
    event: Option<String>,
) -> Result<(), anyhow::Error> {
    let stream_id = get_stream_id(event.clone(), &context.data().db).await?;
    let mut stream = context.data().db.get_stream(stream_id).await?;

    let runner = context.data().db.find_runner(&name).await?;
    stream.audible_runner = Some(runner.id);

    send_message!(
        &context.data().directory.stream_actor,
        StreamRequest,
        Update,
        stream
    )?;
    send_success_reply(&context).await
}

/// Set the volume for a runner.
#[poise::command(prefix_command, slash_command)]
async fn set_runner_volume(
    context: Context<'_>,
    #[description = "Name for this runner"]
    #[autocomplete = "autocomplete_runner_name"]
    name: String,
    #[description = "Volume for this runner in percent"] volume: u32,
) -> Result<(), anyhow::Error> {
    let db = &context.data().db;

    if volume > 100 {
        return Err(anyhow!("Volume must be between 0 and 100"));
    }

    let mut runner = db.find_runner(&name).await?;
    runner.volume_percent = volume;
    send_message!(
        &context.data().directory.runner_actor,
        RunnerRequest,
        Update,
        runner.clone()
    )?;
    send_success_reply(&context).await
}

pub async fn init_discord(
    settings: Arc<Settings>,
    db: Arc<ProjectDb>,
    directory: Directory,
) -> Result<(), anyhow::Error> {
    log::info!("Initializing Discord bot");

    if let Some(channel) = &settings.discord_command_channel {
        log::info!("Receiving Discord commands on '{}'", channel);
    } else {
        log::warn!("No channel specified for 'discord_command_channel' in the settings file, this bot will accept commands on any channel!");
    }

    let http = Http::new(&settings.discord_token.clone().unwrap());
    let _owners = match http.get_current_application_info().await {
        Ok(info) => {
            let mut owners = HashSet::new();
            owners.insert(info.owner.id);
            owners
        }
        Err(why) => {
            return Err(Error::Unknown(format!(
                "Could not access app info: {:?}",
                why
            )))?
        }
    };

    let intents = serenity::GatewayIntents::non_privileged()
        | serenity::GatewayIntents::GUILD_MESSAGES
        | serenity::GatewayIntents::MESSAGE_CONTENT
        | serenity::GatewayIntents::GUILD_VOICE_STATES;

    let commands = vec![
        toggle(),
        set(),
        swap(),
        layout(),
        refresh(),
        ignore(),
        start_stream(),
        stop_stream(),
        create_stream(),
        delete_stream(),
        set_start_time(),
        set_end_time(),
        start_timer(),
        stop_timer(),
        create_event(),
        delete_event(),
        create_runner(),
        delete_runner(),
        set_audible_runner(),
        set_runner_volume(),
    ];

    let options = poise::FrameworkOptions::<Data, anyhow::Error> {
        commands,
        command_check: Some(|ctx| {
            Box::pin(async move {
                if !check_channel(&ctx).await? {
                    Ok(false)
                } else {
                    Ok(true)
                }
            })
        }),
        prefix_options: poise::PrefixFrameworkOptions {
            prefix: Some("/".into()),
            edit_tracker: None,
            ..Default::default()
        },
        event_handler: |ctx, event, _framework, data| {
            Box::pin(async move {
                if let poise::event::Event::VoiceStateUpdate { old, new } = event {
                    handle_voice_state_event(ctx, old, new, data).await;
                }

                Ok(())
            })
        },
        ..Default::default()
    };

    poise::Framework::<Data, anyhow::Error>::builder()
        .token(settings.discord_token.clone().unwrap().trim())
        .intents(intents)
        .options(options)
        .setup(move |ctx, _ready, framework| {
            Box::pin(async move {
                log::info!("Logged in as {}", _ready.user.name);
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                Ok(Data {
                    db,
                    settings,
                    directory,
                })
            })
        })
        .run()
        .await
        .expect("Err running client");

    Ok(())
}
