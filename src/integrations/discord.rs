use songbird::SerenityInit;
use std::{collections::HashSet, sync::Arc};

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
use songbird::{driver::DecodeMode, Config};
use sqlx::types::time::OffsetDateTime;

use crate::{
    core::{
        db::ProjectDb,
        event::EventRequest,
        runner::RunnerRequest,
        settings::Settings,
        stream::{validate_streamed_event_id, StreamRequest},
    },
    error::Error,
    integrations::{discord_voice::connect_to_voice, obs::HostCommand},
    send_message, Directory, Rto,
};

use super::obs::HostActor;

struct Data {
    name: String,
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
    bot_name: &str,
    context: &serenity::Context,
    actor: &HostActor,
    voice_state: &VoiceState,
    settings: &Settings,
) {
    if let Some(channel) = get_voice_guild_channel(voice_state, context).await {
        if let Some(host) = settings
            .obs_hosts
            .iter()
            .find(|h| {
                h.1.discord_voice_channel_id
                    .as_ref()
                    .is_some_and(|id| *id == u64::from(channel.id))
            })
            .map(|m| m.0.to_owned())
        {
            let users = channel.members(context).unwrap();
            let user_list: Vec<String> = users
                .iter()
                .map(|u| u.user.name.clone())
                .filter(|u| u != bot_name)
                .collect();
            let _ = send_message!(actor, HostCommand, SetStreamCommentators, host, user_list);
        }
    }
}

async fn handle_voice_state_event(
    context: &serenity::Context,
    _old_state: &Option<VoiceState>,
    new_state: &VoiceState,
    data: &Data,
) {
    let settings = &data.settings;
    let actor = &data.directory.obs_actor;

    update_voice_list(&data.name, context, actor, new_state, settings).await;
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
        .map(|p| p.participant_data.name.clone())
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
    let runner = context.data().db.get_participant_by_name(&runner).await?;

    match stream.get_runner_slot(runner.id) {
        Some(pos) => {
            stream.stream_runners.remove(&pos);
        }
        None => {
            stream
                .stream_runners
                .insert(stream.get_first_empty_slot(), runner.id);
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
    let runner = context.data().db.get_participant_by_name(&runner).await?;
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

    let runner1 = context.data().db.get_participant_by_name(&runner1).await?;
    let runner2 = context.data().db.get_participant_by_name(&runner2).await?;

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
                res.push(context.data().db.get_participant_by_name(&runner).await?.id);
            }

            res
        }
        None => {
            vec![]
        }
    };

    let stream_id = get_stream_id(event.clone(), &context.data().db).await?;
    let mut stream = context.data().db.get_stream(stream_id).await?;
    stream.stream_runners = runner_ids
        .iter()
        .enumerate()
        .map(|(i, r)| ((i as i64), *r))
        .collect();

    send_message!(
        &context.data().directory.stream_actor,
        StreamRequest,
        Update,
        stream
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
        HostCommand,
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
        HostCommand,
        EndStream,
        host
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

    let runner = context.data().db.get_participant_by_name(&name).await?;
    stream.audible_runner = Some(runner.id);

    send_message!(
        &context.data().directory.stream_actor,
        StreamRequest,
        Update,
        stream
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
            owners.insert(info.owner.unwrap().id);
            owners
        }
        Err(why) => {
            return Err(Error::Unknown(format!(
                "Could not access app info: {:?}",
                why
            )))?
        }
    };

    let songbird_config = Config::default().decode_mode(DecodeMode::Decode);

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
        start_stream(),
        stop_stream(),
        set_start_time(),
        set_end_time(),
        start_timer(),
        stop_timer(),
        set_audible_runner(),
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
                if let serenity::FullEvent::VoiceStateUpdate { old, new } = event {
                    handle_voice_state_event(ctx, old, new, data).await;
                }

                Ok(())
            })
        },
        ..Default::default()
    };

    let token = settings.discord_token.clone().unwrap();
    let framework = poise::Framework::<Data, anyhow::Error>::builder()
        .options(options)
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                log::info!("Logged in as {}", ready.user.name);
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                for (host_name, host) in &settings.obs_hosts {
                    if host.enable_voice.unwrap_or(false) {
                        connect_to_voice(ctx, directory.clone(), settings.clone(), host_name)
                            .await?;
                    }
                }
                Ok(Data {
                    name: ready.user.name.clone(),
                    db,
                    settings,
                    directory,
                })
            })
        })
        .build();

    let mut client = serenity::Client::builder(token, intents)
        .framework(framework)
        .register_songbird_from_config(songbird_config)
        .await?;

    client.start().await?;

    Ok(())
}
