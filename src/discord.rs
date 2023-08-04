use std::{collections::HashSet, sync::Arc};

use serenity::{
    async_trait,
    framework::{
        standard::{
            macros::{command, group},
            CommandResult,
        },
        StandardFramework,
    },
    http::Http,
    model::{prelude::Message, user::CurrentUser, voice::VoiceState},
    prelude::{Context, EventHandler, GatewayIntents, TypeMapKey},
};
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use crate::{error::Error, settings::Settings, CommandMessage, Response};

struct MessageStore;
impl TypeMapKey for MessageStore {
    type Value = UnboundedSender<CommandMessage>;
}

struct UserStore;
impl TypeMapKey for UserStore {
    type Value = Arc<CurrentUser>;
}

struct SettingsStore;
impl TypeMapKey for SettingsStore {
    type Value = Settings;
}

#[group]
#[commands(set, toggle)]
struct General;

pub struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn voice_state_update(
        &self,
        context: Context,
        _old_state: Option<VoiceState>,
        new_state: VoiceState,
    ) {
        log::info!("Ee");
        let channel = match new_state.channel_id.unwrap().to_channel(&context).await {
            Ok(channel) => channel.guild().unwrap(),
            Err(why) => {
                log::error!("Err w/ channel {}", why);

                return;
            }
        };

        if channel.name().contains("CoolChannel") {
            let users = channel.members(&context).await.unwrap();

            let user_list: Vec<String> =
                users.iter().map(|u| u.display_name().to_string()).collect();
            let message = format!("commentary {}", user_list.join(" "));

            let mut data = context.data.write().await;
            let channel = data.get_mut::<MessageStore>().unwrap();

            let (rtx, rrx) = oneshot::channel();
            let _res = channel.send((message, Some(rtx)));

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
    }
}

pub async fn command_general(context: &Context, msg: &Message) -> CommandResult {
    let channel = match msg.channel_id.to_channel(&context).await {
        Ok(channel) => channel.guild().unwrap(),
        Err(why) => {
            println!("Err w/ channel {}", why);

            return Err(Box::new(Error::GeneralError(why.to_string())));
        }
    };

    let mut data = context.data.write().await;
    let user = data.get::<UserStore>().unwrap();

    let channel = data.get_mut::<MessageStore>().unwrap();

    let (rtx, rrx) = oneshot::channel();
    let _res = channel.send((msg.content.to_owned(), Some(rtx)));

    if let Ok(resp) = rrx.await {
        match resp {
            Response::Ok => {
                if let Err(why) = msg.react(&context, '\u{1F44D}').await {
                    println!("Failed to react: {}", why);
                    Err(Box::new(Error::GeneralError(why.to_string())))
                } else {
                    Ok(())
                }
            }
            Response::Error(message) => {
                if let Err(why) = msg.reply(&context, message).await {
                    Err(Box::new(Error::GeneralError(why.to_string())))
                } else {
                    Ok(())
                }
            }
            Response::CurrentState(message) => {
                if let Err(why) = msg
                    .reply(
                        &context,
                        format!(
                            "```\n{}\n```",
                            serde_json::to_string_pretty(&message).unwrap()
                        ),
                    )
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

#[command]
async fn set(context: &Context, msg: &Message) -> CommandResult {
    command_general(context, msg).await
}

#[command]
async fn toggle(context: &Context, msg: &Message) -> CommandResult {
    command_general(context, msg).await
}

pub async fn init_discord(
    settings: Settings,
    tx: UnboundedSender<CommandMessage>,
) -> Result<(), Error> {
    println!("Initializing Discord bot");
    let http = Http::new(&settings.discord_token.clone().unwrap());
    let owners = match http.get_current_application_info().await {
        Ok(info) => {
            let mut owners = HashSet::new();
            owners.insert(info.owner.id);
            owners
        }
        Err(why) => return Err(format!("Could not access app info: {:?}", why))?,
    };

    let bot_user = http
        .get_current_user()
        .await
        .map_err(|why| format!("Could not access user info: {:?}", why))?;

    log::info!("Found user {}", bot_user.name);

    let intents = GatewayIntents::non_privileged()
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILD_VOICE_STATES;

    let framework = StandardFramework::new().configure(|c| c.owners(owners).prefix("!")).group(&GENERAL_GROUP);

    let mut client =
        serenity::Client::builder(settings.discord_token.clone().unwrap().trim(), intents)
            .framework(framework)
            .type_map_insert::<MessageStore>(tx)
            .type_map_insert::<UserStore>(Arc::new(bot_user))
            .event_handler(Handler)
            .await
            .expect("Err creating client");

    client.start().await?;
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
