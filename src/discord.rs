use std::{sync::Arc, collections::HashSet};

use futures::FutureExt;
use serde::{Serialize, Deserialize};
use serenity::{async_trait, prelude::{EventHandler, Context, TypeMapKey, GatewayIntents}, model::{voice::VoiceState, prelude::Message, user::CurrentUser}, http::Http, framework::StandardFramework};
use tokio::sync::mpsc::UnboundedSender;

use crate::CommandMessage;

#[derive(Serialize, Deserialize)]
pub struct DiscordConfig {
    pub token: String,
    pub channel: String,
}

struct MessageStore;
impl TypeMapKey for MessageStore {
    type Value = UnboundedSender<CommandMessage>;
} 

struct UserStore;
impl TypeMapKey for UserStore {
    type Value = Arc<CurrentUser>;
}

struct DiscordStore;
impl TypeMapKey for DiscordStore {
    type Value = DiscordConfig;
}

pub struct Handler; 

#[async_trait]
impl EventHandler for Handler {
    async fn voice_state_update(&self, _context: Context, _old_state: Option<VoiceState>, _new_state: VoiceState) {
    }
    
    async fn message(&self, context: Context, msg: Message) {
       let channel = match msg.channel_id.to_channel(&context).await {
            Ok(channel) => channel.guild().unwrap(),
            Err(why) => {
                println!("Err w/ channel {}", why);
                
                return;
            }
        };


        let mut data = context.data.write().await;
        let user = data.get::<UserStore>().unwrap();

        if channel.name().contains("nsfw") && msg.author.id != user.id {
            let channel = data.get_mut::<MessageStore>().unwrap();

            let context = context.clone();
            let _res = channel.send((
                msg.content.to_owned(),
                Box::new(move |e: Option<String>| async move {
                    match e {
                        None => if let Err(why) = msg.react(&context, '\u{1F44D}').await {
                            println!("Failed to react: {}", why);
                        }
                        Some(err) => if let Err(why) = msg.reply(&context, err).await {
                            println!("Failed to reply: {}", why);
                        }
                    } 
                }.boxed())
            ));
        }
    }
}

pub async fn test_discord(token: &str) -> Result<(), String> {
    let http = Http::new(token);

    let bot_user = http.get_current_user().await
        .map_err(|why| format!("Could not access user info: {:?}", why))?;

    println!("Found user {}", bot_user.name);

    Ok(())
}

pub async fn init_discord(config: DiscordConfig, tx: UnboundedSender<CommandMessage>) -> Result<(), String> {
    println!("Initializing Discord bot");
    let http = Http::new(&config.token);
    let owners = match http.get_current_application_info().await {
        Ok(info) => {
            let mut owners = HashSet::new();
            owners.insert(info.owner.id);
            owners
        }
        Err(why) => return Err(format!("Could not access app info: {:?}", why))
    };

    let bot_user = http.get_current_user().await
        .map_err(|why| format!("Could not access user info: {:?}", why))?;

    println!("Found user {}", bot_user.name);


    let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT;

    let framework = StandardFramework::new()
        .configure(|c| c.owners(owners).prefix("!"));

    let mut client = serenity::Client::builder(config.token.trim(), intents)
        .framework(framework)
        .type_map_insert::<MessageStore>(tx)
        .type_map_insert::<UserStore>(Arc::new(bot_user))
        .event_handler(Handler)
        .await.expect("Err creating client");


    client.start().await.map_err(|why| format!("Discord failed: {}", why))
}

