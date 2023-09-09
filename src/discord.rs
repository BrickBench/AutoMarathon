use std::{collections::{HashSet, HashMap}, sync::Arc};

use poise::serenity_prelude as serenity;

use futures::{Stream, StreamExt};
use serenity::{
    http::Http,
    model::{
        prelude::{ChannelId, GuildChannel},
        voice::VoiceState,
    },
};
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use crate::{
    cmd::CommandSource, error::Error, obs::LayoutFile, settings::Settings, state::Project,
    CommandMessage, Response,
};

struct Data {
    project: Arc<Project>,
    layouts: Arc<LayoutFile>,
    settings: Arc<Settings>,
    tx: UnboundedSender<CommandMessage>,
}
type CError = Box<dyn std::error::Error + Sync + Send>;
type Context<'a> = poise::Context<'a, Data, CError>;

/// Returns the GuildChannel for the provided voice
async fn get_voice_guild_channel(state: &VoiceState, context: &serenity::Context) -> Option<GuildChannel> {
    let channel_id = state.channel_id?;

    to_guild_channel(channel_id, context).await
}

/// Convert a ChannelID to a GuildChannel
async fn to_guild_channel(channel_id: ChannelId, context: &serenity::Context) -> Option<GuildChannel> {
    match channel_id.to_channel(&context).await {
        Ok(channel) => channel.guild(),
        Err(why) => {
            log::error!("Err w/ channel {}", why);

            None
        }
    }
}

async fn handle_voice_state_event(
    context: &serenity::Context,
    old_state: &Option<VoiceState>,
    new_state: &VoiceState,
    data: &Data
) {
    let settings = &data.settings;
    let tx = data.tx.clone();

    if settings.discord_voice_channel.is_none() {
        return;
    }

    let target = settings.discord_voice_channel.as_ref().unwrap().clone();

    let old_channel = match old_state {
        Some(state) => get_voice_guild_channel(state, &context).await,
        None => None,
    };

    let new_channel = get_voice_guild_channel(new_state, &context).await;

    let target_channel = old_channel
        .filter(|c| c.name() == target)
        .or(new_channel.filter(|c| c.name() == target));

    match target_channel {
        Some(channel) => {
            let users = channel.members(&context).await.unwrap();

            let user_list: Vec<String> =
                users.iter().map(|u| u.display_name().to_string()).collect();

            let (rtx, rrx) = oneshot::channel();
            let _res = tx.send((CommandSource::Commentary(user_list), Some(rtx)));

            if let Ok(resp) = rrx.await {
                match resp {
                    Response::Ok => {}
                    Response::Error(message) => {
                        log::error!("Failed to set voice state: {}", message);
                    }
                    Response::CurrentState(_) => {}
                }
            }
        }
        None => {}
    }
}

/// General command processor
async fn command_general(context: &Context<'_>, cmd: CommandSource) -> Result<(), CError> {
    let channel = context.data().tx.clone();
    let _ = context.defer().await;

    let _msg_channel = to_guild_channel(context.channel_id(), context.serenity_context()).await;

    let (rtx, rrx) = oneshot::channel();
    let _res = channel.send((cmd, Some(rtx)));

    if let Ok(resp) = rrx.await {
        match resp {
            Response::Ok => {
                if let Err(why) = context.say("\u{1F44D}").await {
                    println!("Failed to react: {}", why);
                    Err(Box::new(Error::GeneralError(why.to_string())))
                } else {
                    Ok(())
                }
            }
            Response::Error(message) => {
                if let Err(why) = context.say(message).await {
                    Err(Box::new(Error::GeneralError(why.to_string())))
                } else {
                    Ok(())
                }
            }
            Response::CurrentState(message) => {
                if let Err(why) = context
                    .say(format!(
                        "```\n{}\n```",
                        serde_json::to_string_pretty(&message).unwrap()
                    ))
                    .await
                {
                    Err(Box::new(Error::GeneralError(why.to_string())))
                } else {
                    Ok(())
                }
            }
        }
    } else {
        Ok(())
    }
}

/// Create an autocomplete stream that matches runner names
async fn autocomplete_runner_name<'a>(
    ctx: Context<'_>,
    partial: &'a str,
) -> impl Stream<Item = String> + 'a {
    let runners: Vec<String> = ctx
        .data()
        .project
        .players
        .iter()
        .map(|p| p.name.clone())
        .collect();

    futures::stream::iter(runners)
        .filter(move |name| futures::future::ready(name.to_lowercase().starts_with(&partial.to_lowercase())))
        .map(|name| name.to_string())
}

/// Create an autocomplete stream that matches layout names
async fn autocomplete_layout_name<'a>(
    ctx: Context<'_>,
    partial: &'a str,
) -> impl Stream<Item = String> + 'a {
    let runners: Vec<String> = ctx
        .data()
        .layouts
        .layouts
        .iter()
        .map(|p| p.name.clone())
        .collect();

    futures::stream::iter(runners)
        .filter(move |name| futures::future::ready(name.starts_with(partial)))
        .map(|name| name.to_string())
}

/// Create an autocomplete stream that matches layout names
async fn autocomplete_field_name<'a>(
    ctx: Context<'_>,
    partial: &'a str,
) -> impl Stream<Item = String> + 'a {
    futures::stream::iter(ctx.data().project.get_allowed_fields())
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
) -> Result<(), CError> {
    command_general(&context, CommandSource::Toggle(runner)).await?;
    Ok(())
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
) -> Result<(), CError> {
    command_general(
        &context,
        CommandSource::Refresh(
            runners
                .map(|r| r.split(",").map(|s| s.to_owned()).collect())
                .unwrap_or(Vec::new()),
        ),
    )
    .await?;
    Ok(())
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
) -> Result<(), CError> {
    command_general(&context, CommandSource::Swap(runner1, runner2)).await?;
    Ok(())
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
) -> Result<(), CError> {
    command_general(&context, CommandSource::Layout(layout)).await?;
    Ok(())
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
) -> Result<(), CError> {
    command_general(
        &context,
        CommandSource::SetPlayers(
            runners
                .map(|r| r.split(",").map(|s| s.to_owned()).collect())
                .unwrap_or(Vec::new()),
        ),
    )
    .await?;
    Ok(())
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
) -> Result<(), CError> {
    command_general(
        &context,
        CommandSource::CommentaryIgnore(
            ignored
                .map(|r| r.split(",").map(|s| s.to_owned()).collect())
                .unwrap_or(Vec::new()),
        ),
    )
    .await?;
    Ok(())
}

/// Set a player field.
///
/// This command can be used to stop 'listener' users from appearing
/// as commentators. The provided names should be the user's nicknames
/// (those that appear in the voice channel).
/// ```
/// /ignore streamer_bot
/// ```
#[poise::command(prefix_command, slash_command)]
async fn set_field(
    context: Context<'_>,
    #[description = "Runner"]
    #[autocomplete = "autocomplete_runner_name"]
    runner: String,
    #[autocomplete = "autocomplete_field_name"]
    #[description = "Field"]
    field: String,
    #[description = "Value"]
    value: Option<String>
) -> Result<(), CError> {
    let mut data = HashMap::new();
    data.insert(field, value);

    command_general(
        &context,
        CommandSource::SetPlayerFields(
            runner, data
        ),
    )
    .await?;
    Ok(())
}

pub async fn init_discord(
    tx: UnboundedSender<CommandMessage>,
    settings: Arc<Settings>,
    project: Arc<Project>,
    layouts: Arc<LayoutFile>,
) -> Result<(), Error> {
    log::info!("Initializing Discord bot");
    let http = Http::new(&settings.discord_token.clone().unwrap());
    let _owners = match http.get_current_application_info().await {
        Ok(info) => {
            let mut owners = HashSet::new();
            owners.insert(info.owner.id);
            owners
        }
        Err(why) => return Err(format!("Could not access app info: {:?}", why))?,
    };

    let intents = serenity::GatewayIntents::non_privileged()
        | serenity::GatewayIntents::GUILD_MESSAGES
        | serenity::GatewayIntents::MESSAGE_CONTENT
        | serenity::GatewayIntents::GUILD_VOICE_STATES;
    let options = poise::FrameworkOptions::<Data, CError> {
        commands: vec![toggle(), set(), swap(), layout(), refresh(), ignore(), set_field()],
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

    let _client = poise::Framework::<Data, CError>::builder()
        .token(settings.discord_token.clone().unwrap().trim())
        .intents(intents)
        .options(options)
        .setup(move |ctx, _ready, framework| {
            Box::pin(async move {
                log::info!("Logged in as {}", _ready.user.name);
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                Ok(Data {
                    settings,
                    project,
                    layouts,
                    tx,
                })
            })
        })
        .run()
        .await
        .expect("Err running client");

    Ok(())
}

pub async fn test_discord(token: &str) -> Result<(), String> {
    let http = Http::new(token);

    let bot_user = http
        .get_current_user()
        .await
        .map_err(|why| format!("Could not access user info: {:?}", why))?;

    log::info!("Found user {}", bot_user.name);

    Ok(())
}
