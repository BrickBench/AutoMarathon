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
use sqlx::types::time::OffsetDateTime;

use crate::{core::{
    db::ProjectDb,
    event::Event,
    runner::Runner,
    settings::Settings,
    stream::{validate_streamed_event_name, StreamActor, StreamCommand, StreamRequest},
}, error::Error, integrations::obs::{ObsCommand, ObsLayout}, send_message, Rto};

use super::obs::ObsActor;

struct Data {
    db: Arc<ProjectDb>,
    settings: Arc<Settings>,
    state_actor: StreamActor,
    obs_actor: ObsActor,
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

                let resp = send_message!(
                    actor,
                    StreamRequest,
                    UpdateStream,
                    Some(stream.to_string()),
                    StreamCommand::Commentary(user_list)
                );

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
    let actor = &data.state_actor;

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
            if !channel.name().eq_ignore_ascii_case(&desired_channel) {
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

    return Ok(true);
}

async fn wrap_fallible<T>(context: &Context<'_>, result: anyhow::Result<T>) -> anyhow::Result<T> {
    if let Err(err) = &result {
        if let Err(why) = context.say(format!("Failed to run command: {}", err)).await {
            log::warn!("Failed to react: {}", why);
        }
    }

    result
}

async fn send_success_reply(context: &Context<'_>) -> Result<(), anyhow::Error> {
    if let Err(why) = context.say("\u{1F44D}").await {
        log::warn!("Failed to react: {}", why);
        Err(Box::new(Error::Unknown(why.to_string())).into())
    } else {
        Ok(())
    }
}

/// General command processor
async fn command_general(
    context: &Context<'_>,
    cmd: StreamCommand,
    event: Option<String>,
) -> Result<(), anyhow::Error> {
    log::debug!("Discord received command {:?}", cmd);
    let _ = context.defer().await;
    send_message!(
        &context.data().state_actor,
        StreamRequest,
        UpdateStream,
        event,
        cmd
    )?;
    send_success_reply(&context).await
}

/// Create an autocomplete stream that matches streamed events
async fn autocomplete_streamed_event_name<'a>(
    ctx: Context<'_>,
    partial: &'a str,
) -> impl Stream<Item = String> + 'a {
    let runners: Vec<String> = ctx.data().db.get_streamed_events().await.unwrap();

    futures::stream::iter(runners)
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
async fn autocomplete_layout_name<'a>(
    ctx: Context<'_>,
    partial: &'a str,
) -> impl Stream<Item = String> + 'a {
    let layouts: Vec<String> = ctx
        .data()
        .db
        .get_layouts()
        .await
        .unwrap()
        .iter()
        .map(|p| p.name.clone())
        .collect();

    futures::stream::iter(layouts)
        .filter(move |name| futures::future::ready(name.starts_with(partial)))
        .map(|name| name.to_string())
}

/// Create an autocomplete stream that matches layout names
async fn autocomplete_obs_name<'a>(
    ctx: Context<'_>,
    partial: &'a str,
) -> impl Stream<Item = String> + 'a {
    let hosts: Vec<String> = ctx
        .data()
        .settings
        .obs_hosts
        .keys()
        .map(|k| k.clone())
        .collect();

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
    wrap_fallible(
        &context,
        command_general(&context, StreamCommand::Toggle(runner), event).await,
    )
    .await
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
    #[description = "Runners to refresh"] runners: Option<String>,
    #[description = "Event for this command"]
    #[autocomplete = "autocomplete_streamed_event_name"]
    event: Option<String>,
) -> Result<(), anyhow::Error> {
    wrap_fallible(
        &context,
        command_general(
            &context,
            StreamCommand::Refresh(
                runners
                    .map(|r| r.split(',').map(|s| s.trim().to_owned()).collect())
                    .unwrap_or_default(),
            ),
            event,
        )
        .await,
    )
    .await
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
    wrap_fallible(
        &context,
        command_general(&context, StreamCommand::Swap(runner1, runner2), event).await,
    )
    .await
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
    #[autocomplete = "autocomplete_layout_name"]
    layout: String,
    #[description = "Event for this command"]
    #[autocomplete = "autocomplete_streamed_event_name"]
    event: Option<String>,
) -> Result<(), anyhow::Error> {
    wrap_fallible(
        &context,
        command_general(&context, StreamCommand::Layout(layout), event).await,
    )
    .await
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
    wrap_fallible(
        &context,
        command_general(
            &context,
            StreamCommand::SetPlayers(
                runners
                    .map(|r| r.split(',').map(|s| s.trim().to_owned()).collect())
                    .unwrap_or_default(),
            ),
            event,
        )
        .await,
    )
    .await
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
    wrap_fallible(
        &context,
        command_general(
            &context,
            StreamCommand::CommentaryIgnore(
                ignored
                    .map(|r| r.split(',').map(|s| s.to_owned()).collect())
                    .unwrap_or_default(),
            ),
            event,
        )
        .await,
    )
    .await
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
    send_message!(
        &context.data().state_actor,
        StreamRequest,
        CreateStream,
        event,
        host
    );
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
    let event = wrap_fallible(
        &context,
        validate_streamed_event_name(&context.data().db, Some(event)).await,
    )
    .await?;
    wrap_fallible(
        &context,
        send_message!(
            &context.data().state_actor,
            StreamRequest,
            DeleteStream,
            event
        ),
    )
    .await?;
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
    event: Option<String>,
) -> Result<(), anyhow::Error> {
    let db = &context.data().db;
    let time = OffsetDateTime::now_utc();
    let event = wrap_fallible(&context, validate_streamed_event_name(&db, event).await).await?;
    wrap_fallible(&context, db.set_event_start_time(&event, Some(time)).await).await?;
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
    event: Option<String>,
) -> Result<(), anyhow::Error> {
    let db = &context.data().db;
    let time = OffsetDateTime::now_utc();
    let event = wrap_fallible(&context, validate_streamed_event_name(&db, event).await).await?;
    wrap_fallible(&context, db.set_event_end_time(&event, Some(time)).await).await?;
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
    event: Option<String>,
) -> Result<(), anyhow::Error> {
    let db = &context.data().db;
    let event = wrap_fallible(&context, validate_streamed_event_name(&db, event).await).await?;
    wrap_fallible(
        &context,
        db.set_event_start_time(
            &event,
            time.map(|t| OffsetDateTime::from_unix_timestamp(t.try_into().unwrap()).ok())
                .flatten(),
        )
        .await,
    )
    .await?;
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
    event: Option<String>,
) -> Result<(), anyhow::Error> {
    let db = &context.data().db;
    let event = wrap_fallible(&context, validate_streamed_event_name(&db, event).await).await?;
    wrap_fallible(
        &context,
        db.set_event_end_time(
            &event,
            time.map(|t| OffsetDateTime::from_unix_timestamp(t.try_into().unwrap()).ok())
                .flatten(),
        )
        .await,
    )
    .await?;
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
    wrap_fallible(
        &context,
        send_message!(&context.data().obs_actor, ObsCommand, StartStream, host),
    )
    .await?;
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
    wrap_fallible(
        &context,
        send_message!(&context.data().obs_actor, ObsCommand, EndStream, host),
    )
    .await?;
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
    let new_event = Event {
        name: event_name,
        therun_race_id: None,
        start_time: None,
        end_time: None,
        is_relay: false,
        is_marathon: false,
    };

    let db = &context.data().db;
    let runners_names_list: Vec<_> = runners
        .map(|r| r.split(',').map(|s| s.trim().to_owned()).collect())
        .unwrap_or_default();

    let mut runners = vec![];
    for name in runners_names_list {
        runners.push(wrap_fallible(&context, db.find_runner(&name).await).await?);
    }

    wrap_fallible(&context, db.add_event(&new_event).await).await?;

    for runner in runners {
        wrap_fallible(
            &context,
            db.add_runner_to_event(&new_event.name, &runner.name).await,
        )
        .await?;
    }

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

    let db = &context.data().db;
    if wrap_fallible(&context, db.get_streamed_events().await)
        .await?
        .contains(&event_name)
    {
        return Err(anyhow!(
            "Cannot delete event {} as it currently has an associated stream.
             Please end the stream first.",
            event_name
        )
        .into());
    }

    wrap_fallible(&context, db.delete_event(&event_name).await).await?;

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
    let runner = Runner {
        name,
        stream,
        therun,
        cached_stream_url: None,
        volume_percent: 50,
    };

    let db = &context.data().db;
    let nicknames: Vec<_> = nicknames
        .map(|r| r.split(',').map(|s| s.trim().to_owned()).collect())
        .unwrap_or_default();

    wrap_fallible(&context, db.add_runner(&runner, &nicknames).await).await?;

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
    log::debug!("Deleting runner {}", name);
    let db = &context.data().db;
    let runner_events = wrap_fallible(&context, db.get_events_for_runner(&name).await).await?;
    if !runner_events.is_empty() {
        return Err(anyhow!(
            "Cannot delete runner {}, as they are used in the following events: {:?}",
            name,
            runner_events
        )
        .into());
    }

    wrap_fallible(&context, db.delete_runner(&name).await).await?;

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
    command_general(&context, StreamCommand::SetAudibleRunner(name), event).await?;
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

    if volume > 100 || volume < 0 {
        return Err(anyhow!("Volume must be between 0 and 100").into());
    }

    let mut runner = db.find_runner(&name).await?;
    runner.volume_percent = volume;
    db.update_runner_volume(&runner).await?;
    send_success_reply(&context).await
}

/// Create a layout
#[poise::command(prefix_command, slash_command)]
async fn create_layout(
    context: Context<'_>,
    #[description = "OBS scene name"] scene: String,
    #[description = "Supported runners"] size: u32,
    #[description = "Default for this runner count"] default: bool,
) -> Result<(), anyhow::Error> {
    log::debug!("Creating layout {}", scene);

    let layout = ObsLayout {
        name: scene,
        runner_count: size,
        default_layout: default,
    };

    let db = &context.data().db;
    wrap_fallible(&context, db.create_layout(&layout).await).await?;
    send_success_reply(&context).await
}

pub async fn init_discord(
    settings: Arc<Settings>,
    db: Arc<ProjectDb>,
    state_actor: StreamActor,
    obs_actor: ObsActor,
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
        create_layout(),
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
                    state_actor,
                    obs_actor,
                })
            })
        })
        .run()
        .await
        .expect("Err running client");

    Ok(())
}

pub async fn test_discord(token: &str) -> Result<(), Error> {
    let http = Http::new(token);

    let bot_user = http
        .get_current_user()
        .await
        .map_err(|why| format!("Could not access user info: {:?}", why))?;

    log::info!("Found user {}", bot_user.name);

    Ok(())
}
