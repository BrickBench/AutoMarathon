use std::{path::PathBuf, fs::{File, read_to_string, self}, collections::{HashMap, HashSet}, sync::Arc, future::Future, pin::Pin};

use clap::{Parser, Subcommand};

use futures::{FutureExt, future::BoxFuture};
use serenity::{async_trait, prelude::{EventHandler, Context, GatewayIntents, TypeMapKey}, model::{prelude::Message, user::{User, CurrentUser}}, Client, framework::StandardFramework, http::Http};
use state::{Project, ProjectType, ProjectState, ProjectStateSerializer};
use tokio::{sync::mpsc::{self, UnboundedSender, UnboundedReceiver}, runtime::Handle};
use user::Player;

use crate::{state::{ProjectTypeState, ProjectTypeStateSerializer}, cmd::parse_cmds};

mod user;
mod cmd;
mod state;

#[derive(Parser, Debug)]
#[command(name = "AutoMarathon")]
#[command(author = "javster101")]
#[command(version = "0.1")]
#[command(about = "An automation tool for speedrunning events.", long_about = None)]
struct Args {
    #[command(subcommand)]
    command: RunType,
}

#[derive(Subcommand, Debug)]
enum RunType {
    /// Create and initialize a new project. 
    /// The output .json file will need to be manually edited to fill in details for players.
    Create {
        #[arg(short = 't', long)]
        project_type: ProjectType,

        /// List of players to initialize.
        #[arg(short = 'p', long, use_value_delimiter = true, value_delimiter = ',')]
        players: Vec<String>,

        project_file: PathBuf,
    },

    /// Run a project file.
    Run {

        /// Location for project state save file.
        #[arg(short, long)]
        save_file: PathBuf,

        /// File containing the token to access the discord bot that will be used.
        #[arg(short, long)]
        token_file: PathBuf,

        project_file: PathBuf,
    }
}

struct Handler; 

#[async_trait]
impl EventHandler for Handler {
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
            let res = channel.send((
                msg.content.to_owned(),
                Box::new(move |e: String| async move {
                    if e.is_empty() {
                        if let Err(why) = msg.react(&context, '\u{1F44D}').await {
                            println!("Failed to react: {}", why);
                        }
                    } else {
                        println!("{}", e);
                        if let Err(why) = msg.reply(&context, e).await {
                            println!("Failed to reply: {}", why);
                        }
                    } 
                }.boxed())
            ));
        }
    }
}

type CommandMessage = (String, Box<dyn Send + Sync + FnOnce(String) -> BoxFuture<'static, ()>>);

struct MessageStore;
impl TypeMapKey for MessageStore {
    type Value = UnboundedSender<CommandMessage>;
} 

struct UserStore;
impl TypeMapKey for UserStore {
    type Value = Arc<CurrentUser>;
}

async fn run_update<'a>(project: Project, state_src: ProjectStateSerializer, state_file: PathBuf, mut rx: UnboundedReceiver<CommandMessage>) {
    let mut state = ProjectState::from_save_state(&state_src, &project).unwrap();

    loop {
        let (msg, resp) = rx.recv().await.unwrap();

        if msg.starts_with("exit") {
            break;
        }

        let cmd = parse_cmds(&msg, &project);

        match &cmd {
            Ok(cmd) => {
                let state_res = state.apply_cmds(cmd);
                match state_res {
                    Ok(new_state) => {
                        state = new_state;
                        resp("".to_owned()).await;

                        let state_save = serde_json::to_string_pretty(&state.to_save_state()).unwrap();
                        fs::write(&state_file, state_save).expect("Failed to save file.");
                    }
                    Err(err) => {
                        println!("State applying error: {}", err.to_string());
                        resp(err.to_string()).await;
                    }
                }
            },
            Err(err) => {
                println!("Command error: {}", err.to_string());
                resp(err.to_string()).await;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {

    let args = Args::parse();
    
    match &args.command {
        RunType::Create { project_type, players, project_file } => {
            let new_project = Project {
                kind: project_type.to_owned(),
                players: players.iter().map(|p| Player {
                    name: p.to_owned(),
                    nicks: vec!(),
                    twitch: "FILL_THIS".to_owned(),
                }).collect()
            };

            let new_json = serde_json::to_string_pretty(&new_project).unwrap();
            
            fs::write(project_file, new_json).map_err(|e| format!("Failed to write project file: {}", e))?;
    
            Ok(())
        },
        RunType::Run { save_file, token_file, project_file} => {

            // Load token
            let token: String = match read_to_string(&token_file) {
                Ok(token) => token.trim().into(),
                Err(why) => return Err(format!("Failed to read token file: {}", why))
            };

            // Load project
            let project: Project = match read_to_string(&project_file) {
                Ok(file) => {
                    serde_json::from_str::<Project>(&file).map_err(|e| format!("Failed to parse project file: {}", e))?
                },
                Err(why) => return Err(format!("Failed to open project file {}: {}", &project_file.to_str().unwrap(), why)),
            };

            println!("Loaded project");

            // Load or create project state
            let state: ProjectStateSerializer = match read_to_string(save_file) {
                Ok(file) => {
                    println!("Loading project state file...");
                    let save_res = serde_json::from_str::<ProjectStateSerializer>(&file).map_err(|e| format!("Failed to parse state file: {}", e));
                    match save_res {
                        Ok(save) => {
                            ProjectState::from_save_state(&save, &project).map_err(|e| e.to_string())?;
                            save
                        }
                        Err(_) => ProjectStateSerializer {
                            active_players: vec!(),
                            type_state: ProjectTypeStateSerializer::MarathonState { runner_times: HashMap::new() }, 
                        }
                    }
                },
                Err(_) => {
                    println!("Project state file does not exist, creating...");
                    File::create(save_file).map_err(|err| format!("Failed to create state file {}: {}", save_file.to_str().unwrap(), err))?;
                    ProjectStateSerializer {
                        active_players: vec!(),
                        type_state: ProjectTypeStateSerializer::MarathonState { runner_times: HashMap::new() }, 
                    }
                }
            };

            // Initialize discord bot
            let http = Http::new(&token);
            let owners = match http.get_current_application_info().await {
                Ok(info) => {
                    let mut owners = HashSet::new();
                    owners.insert(info.owner.id);
                    owners
                }
                Err(why) => return Err(format!("Could not access app info: {:?}", why))
            };

            let bot_user = match http.get_current_user().await {
                Ok(usr) => usr,
                Err(why) => return Err(format!("Could not access user info: {:?}", why))
            };

            println!("Found user {}", bot_user.name);

            let (tx, rx): (UnboundedSender<CommandMessage>, UnboundedReceiver<CommandMessage>) = mpsc::unbounded_channel();

            let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT;

            let framework = StandardFramework::new()
                .configure(|c| c.owners(owners).prefix("!"));

            let mut client = Client::builder(token.trim(), intents)
                .framework(framework)
                .type_map_insert::<MessageStore>(tx)
                .type_map_insert::<UserStore>(Arc::new(bot_user))
                .event_handler(Handler)
                .await.expect("Err creating client");

            let work_thread = Handle::current().spawn(run_update(project, state, save_file.to_owned(), rx));

            let _client_future = client.start().await;
            work_thread.await;
           /* 
            println!("Saving project state file...");


*/
            Ok(())
        },
    }   
}


#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use crate::{state::{ProjectState, Project, ProjectType, ProjectTypeState, ProjectStateSerializer}, user::Player, cmd::{parse_cmd, Command}};

    #[test]
    fn test_nicks() {
        let project = Project {
            kind: ProjectType::Marathon,
            players: vec!(
                Player {name:"Joe".to_owned(),nicks:vec!("Joseph".to_owned(),"John".to_owned()), twitch: "test".to_owned() },
                Player {name:"William".to_owned(),nicks:vec!("Will".to_owned(),"Bill".to_owned()), twitch: "test2".to_owned() },
            )
        };

        assert_eq!(project.find_by_nick("Joe").unwrap().name, "Joe");
        assert_eq!(project.find_by_nick("john").unwrap().name, "Joe");
        assert!(project.find_by_nick("Jow").is_none());
    }

    #[test]
    fn test_cmds() {
        let project = Project {
            kind: ProjectType::Marathon,
            players: vec!(
                Player {name:"Joe".to_owned(),nicks:vec!("Joseph".to_owned(),"John".to_owned()), twitch: "test".to_owned() },
                Player {name:"William".to_owned(),nicks:vec!("Will".to_owned(),"Bill".to_owned()), twitch: "test2".to_owned() },
            )
        };


        assert_eq!(Command::Toggle(&project.players[0]), parse_cmd("!toggle john", &project).unwrap());

        assert_eq!(Command::Swap(&project.players[0], &project.players[1]), parse_cmd("!swap john bill", &project).unwrap());
        assert!(parse_cmd("!swap john sam", &project).is_err());
        assert!(parse_cmd("!swap john will bill", &project).is_err());
        assert!(parse_cmd("!swap bill will", &project).is_err());
       

        assert_eq!(Command::SetPlayers(vec!(&project.players[0], &project.players[1])), parse_cmd("!set john bill", &project).unwrap());
        assert!(parse_cmd("!set earl bill", &project).is_err());
        assert!(parse_cmd("!set bill will dill", &project).is_err());

        assert_eq!(Command::SetScore(&project.players[0], Duration::from_millis(222)), parse_cmd("!record joe 222", &project).unwrap());
        assert!(parse_cmd("!record joe 22e", &project).is_err()); 
    }

    #[test]
    fn test_cmd_apply() {
        let project = Project {
            kind: ProjectType::Marathon,
            players: vec!(
                Player {name:"Joe".to_owned(),nicks:vec!("Joseph".to_owned(),"John".to_owned()), twitch: "test".to_owned() },
                Player {name:"William".to_owned(),nicks:vec!("Will".to_owned(),"Bill".to_owned()), twitch: "test2".to_owned() },
            )
        };

        let mut state = ProjectState {
            running: false,
            active_players: vec!(),
            streams: HashMap::new(),
            type_state: ProjectTypeState::MarathonState { runner_times: HashMap::new() }, 
        };

        state = state.apply_cmd(&parse_cmd("!toggle joe", &project).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "Joe");

        state = state.apply_cmd(&parse_cmd("!swap joe will", &project).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "William");

        state = state.apply_cmd(&parse_cmd("!swap joe will", &project).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "Joe");

        state = state.apply_cmd(&parse_cmd("!set joe will", &project).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "Joe");
        assert_eq!(state.active_players[1].name, "William");

        state = state.apply_cmd(&parse_cmd("!swap joe will", &project).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "William");
        assert_eq!(state.active_players[1].name, "Joe");

        state = state.apply_cmd(&parse_cmd("!toggle joe", &project).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "William");

        assert!(state.apply_cmd(&parse_cmd("!refresh", &project).unwrap()).is_err());
    }

    #[test]
    fn test_serialization() {
        let project = Project {
            kind: ProjectType::Marathon,
            players: vec!(
                Player {name:"Joe".to_owned(),nicks:vec!("Joseph".to_owned(),"John".to_owned()), twitch: "test".to_owned() },
                Player {name:"William".to_owned(),nicks:vec!("Will".to_owned(),"Bill".to_owned()), twitch: "test2".to_owned() },
            )
        };

        let saved_project = serde_json::to_string(&project).unwrap();
        let regen_project: Project = serde_json::from_str(&saved_project).unwrap();

        assert_eq!(project, regen_project);

        let mut state = ProjectState {
            running: false,
            active_players: vec!(),
            streams: HashMap::new(),
            type_state: ProjectTypeState::MarathonState { runner_times: HashMap::new() }, 
        };

        state = state.apply_cmd(&parse_cmd("!toggle joe", &project).unwrap()).unwrap();

        let saved_state = serde_json::to_string(&state.to_save_state()).unwrap();
        let regen_state_serial: ProjectStateSerializer = serde_json::from_str(&saved_state).unwrap();
        let regen_state = ProjectState::from_save_state(&regen_state_serial, &project).unwrap();

        assert_eq!(state, regen_state);
    }
}
