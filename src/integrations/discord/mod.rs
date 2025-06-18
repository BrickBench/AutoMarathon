use commands::define_command_options;
use discord_voice::connect_to_voice;
use poise::serenity_prelude as serenity;
use songbird::{driver::DecodeMode, Config, SerenityInit};
use std::{collections::HashSet, sync::Arc};

use crate::{
    core::{db::ProjectDb, settings::Settings},
    web::streams::WebCommand,
    integrations::obs::HostCommand,
    send_message, Directory, Rto,
};

mod commands;
mod discord_voice;

struct Data {
    name: String,
    db: Arc<ProjectDb>,
    settings: Arc<Settings>,
    directory: Directory,
}

/// Convert a ChannelID to a GuildChannel
async fn to_guild_channel(
    channel_id: serenity::ChannelId,
    context: &serenity::Context,
) -> Option<serenity::GuildChannel> {
    match channel_id.to_channel(&context).await {
        Ok(channel) => channel.guild(),
        Err(why) => {
            log::error!("Err w/ channel {}", why);

            None
        }
    }
}

/// Returns the GuildChannel for the provided voice channel
async fn get_voice_guild_channel(
    state: &serenity::VoiceState,
    context: &serenity::Context,
) -> Option<serenity::GuildChannel> {
    let channel_id = state.channel_id?;

    to_guild_channel(channel_id, context).await
}

/// Determine the Discord users present in the provided voice channel.
async fn update_voice_list(
    bot_name: &str,
    context: &serenity::Context,
    directory: &Directory,
    voice_state: &serenity::VoiceState,
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
            let user_list: Vec<(u64, String)> = users
                .iter()
                .map(|u| (u.user.id.get(), u.user.name.clone()))
                .filter(|u| u.1 != bot_name)
                .collect();

            let _ = send_message!(
                directory.obs_actor,
                HostCommand,
                SetStreamCommentators,
                host,
                user_list
            );

            directory.web_actor.send(WebCommand::TriggerStateUpdate);
        }
    }
}

async fn handle_voice_state_event(
    context: &serenity::Context,
    old_state: &Option<serenity::VoiceState>,
    new_state: &serenity::VoiceState,
    data: &Data,
) {
    let settings = &data.settings;

    if let Some(old_state) = old_state {
        update_voice_list(&data.name, context, &data.directory, old_state, settings).await;
    }

    update_voice_list(&data.name, context, &data.directory, new_state, settings).await;
}

/// Initialize the discord bot, enabling voice and command integration if defined in the settings
/// file.
pub async fn init_discord(
    token: String,
    settings: Arc<Settings>,
    db: Arc<ProjectDb>,
    directory: Directory,
) -> anyhow::Result<()> {
    log::info!("Initializing Discord bot");

    let http = serenity::Http::new(&settings.discord_token.clone().unwrap());
    let _owners = match http.get_current_application_info().await {
        Ok(info) => {
            let mut owners = HashSet::new();
            owners.insert(info.owner.unwrap().id);
            owners
        }
        Err(why) => {
            anyhow::bail!("Could not access app info: {:?}", why)
        }
    };

    let songbird_config = Config::default().decode_mode(DecodeMode::Decode);

    let intents = serenity::GatewayIntents::non_privileged()
        | serenity::GatewayIntents::GUILD_MESSAGES
        | serenity::GatewayIntents::MESSAGE_CONTENT
        | serenity::GatewayIntents::GUILD_VOICE_STATES;


    let mut options = poise::FrameworkOptions::<Data, anyhow::Error> {
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

    if let Some(channel) = &settings.discord_command_channel {
        log::info!("Receiving Discord commands on '{}'", channel);
        define_command_options(&mut options);
    } else {
        log::info!("Discord command integration disabled.")
    }

    let framework = poise::Framework::<Data, anyhow::Error>::builder()
        .options(options)
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                log::info!("Logged in as {}", ready.user.name);
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                for (host_name, host) in &settings.obs_hosts {
                    if host.enable_voice.unwrap_or(false) {
                        connect_to_voice(
                            ctx,
                            directory.clone(),
                            db.clone(),
                            settings.clone(),
                            host_name,
                        )
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
