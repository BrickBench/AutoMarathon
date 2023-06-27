use std::{path::PathBuf, fs::{File, read_to_string, self}, collections::HashMap};

use clap::{Parser, Subcommand};

use discord::test_discord;
use error::Error;
use futures::{future::BoxFuture, FutureExt};
use state::{Project, ProjectType, ProjectState, ProjectStateSerializer};
use tokio::sync::mpsc::{self, UnboundedSender, UnboundedReceiver};
use user::Player;

use crate::{state::ProjectTypeStateSerializer, cmd::parse_cmds, discord::{init_discord, DiscordConfig}, obs::ObsConfiguration};

mod user;
mod cmd;
mod obs;
mod error;
mod discord;
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

    /// Create a discord configuration file.
    /// This initializes and validates a configuration file.
    /// DO NOT SHARE THIS FILE, it contains the private token for your Discord bot.
    Discord {
        /// A token to a bot that is present in the server to be used for commands.
        /// For full functionality, this bot should have the ADD_REACTIONS, SEND_MESSAGES, and
        /// READ_MESSAGE_HISTORY permissions. 
        #[arg(short, long)]
        token: String,

        /// The channel name that this bot will monitor for command messages.
        #[arg(short, long)]
        channel: String,

        /// The output path of the discord file.
        discord_file: PathBuf
    },

    /// Run a project sourced from a project file.
    Run {

        /// Location for project state save file.
        #[arg(short, long)]
        save_file: PathBuf,

        /// Location for OBS configuration and layout file.
        /// A sample is provided in the `obs/layouts` directory.
        #[arg(short, long)]
        obs_file: PathBuf,
        
        /// Location of Discord configuration file. 
        /// If provided, Discord integration will be enabled.
        /// This configuration file can be created with the `AutoMarathon discord` command.
        #[arg(short, long)]
        discord_file: Option<PathBuf>,

        project_file: PathBuf,
    }
}

/// A type for inter-thread communication with a string command and a callback for returning errors.
type CommandMessage = (String, Box<dyn Send + Sync + FnOnce(Option<String>) -> BoxFuture<'static, ()>>);

async fn run_update<'a>(project: Project, state_src: ProjectStateSerializer, obs_cfg: ObsConfiguration, state_file: PathBuf, mut rx: UnboundedReceiver<CommandMessage>, obs: obws::Client) {
    let mut state = ProjectState::from_save_state(&state_src, &project, &obs_cfg).unwrap();
    
    loop {
        let (msg, resp) = rx.recv().await.unwrap();

        if msg.starts_with("exit") {
            break;
        }

        let cmd = parse_cmds(&msg, &project, &obs_cfg, &state);

        match &cmd {
            Ok(cmd) => {
                let state_res = state.apply_cmds(cmd);
                match state_res {
                    Ok(new_state) => {
                        state = new_state;
                        let resp = tokio::spawn(resp(None));
                        
                        if let Err(e) = state.apply_layout(&obs_cfg, &obs).await {
                            println!("Error while applying layout: {}", e.to_string())
                        }

                        let state_save = serde_json::to_string_pretty(&state.to_save_state()).unwrap();
                        fs::write(&state_file, state_save).expect("Failed to save file.");

                        resp.await.expect("Error in response.");
                    }
                    Err(err) => {
                        resp(Some(err.to_string())).await;
                    }
                }
            },
            Err(err) => {
                resp(Some(err.to_string())).await;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {

    let args = Args {
        command: RunType::Run { 
            save_file: PathBuf::from("save.json"), 
            obs_file: PathBuf::from("obs.json"), 
            discord_file: None, 
            project_file: PathBuf::from("proj.json")
        }
    };//Args::parse();
    
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
            
            fs::write(project_file, new_json)?;
            
            println!("Project created, please open the file in a text editor and fill in the missing fields.");
            Ok(())
        },
        RunType::Discord { token, channel, discord_file } => {
            test_discord(token).await?;

            println!("Token is valid, saving file...");
            let discord_config = DiscordConfig { token: token.to_owned(), channel: channel.to_owned() };

            let discord_txt = serde_json::to_string_pretty(&discord_config).unwrap();
            fs::write(&discord_file, discord_txt)?;
            
            println!("Discord file saved.");

            Ok(())
        }
        RunType::Run { save_file, obs_file, discord_file, project_file} => {

            // Load  layouts
            let obs_cfg: ObsConfiguration = serde_json::from_str::<ObsConfiguration>(&read_to_string(&obs_file)?)?; 
            // Load project
            let project: Project = serde_json::from_str::<Project>(&read_to_string(&project_file)?)?; 

            println!("Loaded project");

            // Load or create project state
            let state: ProjectStateSerializer = match read_to_string(save_file) {
                Ok(file) => {
                    println!("Loading project state file...");
                    let save_res = serde_json::from_str::<ProjectStateSerializer>(&file);
                    match save_res {
                        Ok(save) => {
                            ProjectState::from_save_state(&save, &project, &obs_cfg).map_err(|e| e.to_string())?;
                            save
                        }
                        Err(_) => ProjectStateSerializer {
                            active_players: vec!(),
                            type_state: ProjectTypeStateSerializer::MarathonState { runner_times: HashMap::new() }, 
                            layouts_by_count: HashMap::new() 
                        }
                    }
                },
                Err(_) => {
                    println!("Project state file does not exist, creating...");
                    File::create(save_file)?;
                    ProjectStateSerializer {
                        active_players: vec!(),
                        type_state: ProjectTypeStateSerializer::MarathonState { runner_times: HashMap::new() }, 
                        layouts_by_count: HashMap::new() 
                    }
                }
            };

            let (tx, rx): (UnboundedSender<CommandMessage>, UnboundedReceiver<CommandMessage>) = mpsc::unbounded_channel();
            
            // Initialize OBS websocket
            println!("Connecting to OBS");
            let obs = obws::Client::connect(
                obs_cfg.ip.to_owned().unwrap_or("localhost".to_owned()), 
                obs_cfg.port.to_owned().unwrap_or(4455), 
                obs_cfg.password.to_owned()).await?;
            
            let obs_version = obs.general().version().await?;
            println!("Connected to OBS version {}, websocket {}, running on {} ({})", 
                obs_version.obs_version.to_string(),
                obs_version.obs_web_socket_version.to_string(),
                obs_version.platform, obs_version.platform_description);

            println!("AutoMarathon initialized");


            let discord_config = match discord_file {
                Some(discord_path) =>  {
                    Some(serde_json::from_str::<DiscordConfig>(&read_to_string(&discord_path)?)?)
                },
                None => {
                    println!("Discord file was not provided, Discord integration is disabled.");
                    None
                }
            };
       
            let tx2 = tx.clone();
            std::thread::spawn(move || 
                loop {
                    let mut cmd: String = String::new();
                    std::io::stdin().read_line(&mut cmd).expect("Failed to read from stdin");

                    if cmd.to_lowercase() == "exit" {
                        break;
                    } else {
                        tx2.send((
                            cmd,
                            Box::new(move |e: Option<String>| async move {
                                if let Some(err) = e {
                                   println!("Failed to run command: {}", err); 
                                } 
                            }.boxed())
                        )).map_err(|_| "Failed to send command.").unwrap();
                    }
                }
            );
            
            let work_thread = run_update(project, state, obs_cfg, save_file.to_owned(), rx, obs);
            if let Some(config) = discord_config{
                let (discord, work) = tokio::join! (
                    tokio::spawn(init_discord(config, tx.clone())),
                    tokio::spawn(work_thread)
                );

                discord.unwrap()?;
                work.unwrap();

            } else {
                tokio::spawn(work_thread).await.expect("Work thread failed.");
            }

            Ok(())
        },
    } 
}


#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use crate::{state::{ProjectState, Project, ProjectType, ProjectTypeState, ProjectStateSerializer}, user::Player, cmd::{parse_cmd, Command}, obs::ObsConfiguration};

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


        let state = ProjectState {
            running: false,
            active_players: vec!(),
            streams: HashMap::new(),
            type_state: ProjectTypeState::MarathonState { runner_times: HashMap::new() },
            layouts_by_count: HashMap::new() 
        };


        let obs = ObsConfiguration {
            ip: None,
            port: None, 
            password: None, 
            scene: "Scene".to_string(),
            layouts: HashMap::new()
        };

        assert_eq!(Command::Toggle(&project.players[0]), parse_cmd("!toggle john", &project, &obs, &state).unwrap());

        assert_eq!(Command::Swap(&project.players[0], &project.players[1]), parse_cmd("!swap john bill", &project, &obs, &state).unwrap());
        assert!(parse_cmd("!swap john sam", &project, &obs, &state).is_err());
        assert!(parse_cmd("!swap john will bill", &project, &obs, &state).is_err());
        assert!(parse_cmd("!swap bill will", &project, &obs, &state).is_err());
       

        assert_eq!(Command::SetPlayers(vec!(&project.players[0], &project.players[1])), parse_cmd("!set john bill", &project, &obs, &state).unwrap());
        assert!(parse_cmd("!set earl bill", &project, &obs, &state).is_err());
        assert!(parse_cmd("!set bill will dill", &project, &obs, &state).is_err());

        assert_eq!(Command::SetScore(&project.players[0], Duration::from_millis(222)), parse_cmd("!record joe 222", &project, &obs, &state).unwrap());
        assert!(parse_cmd("!record joe 22e", &project, &obs, &state).is_err()); 
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
            layouts_by_count: HashMap::new() 
        };


        let obs = ObsConfiguration {
            ip: None,
            port: None, 
            password: None,
            scene: "Scene".to_string(),
            layouts: HashMap::new()
        };

        state = state.apply_cmd(&parse_cmd("!toggle joe", &project, &obs, &state).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "Joe");

        state = state.apply_cmd(&parse_cmd("!swap joe will", &project, &obs, &state).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "William");

        state = state.apply_cmd(&parse_cmd("!swap joe will", &project, &obs, &state).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "Joe");

        state = state.apply_cmd(&parse_cmd("!set joe will", &project, &obs, &state).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "Joe");
        assert_eq!(state.active_players[1].name, "William");

        state = state.apply_cmd(&parse_cmd("!swap joe will", &project, &obs, &state).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "William");
        assert_eq!(state.active_players[1].name, "Joe");

        state = state.apply_cmd(&parse_cmd("!toggle joe", &project, &obs, &state).unwrap()).unwrap();
        assert_eq!(state.active_players[0].name, "William");

        assert!(state.apply_cmd(&parse_cmd("!refresh", &project, &obs, &state).unwrap()).is_err());
    }

    #[test]
    fn test_serialization() {
        let obs = ObsConfiguration {
            ip: None,
            port: None, 
            password: None, 
            scene: "Scene".to_string(),
            layouts: HashMap::new()
        };

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
            layouts_by_count: HashMap::new() 
        };

        state = state.apply_cmd(&parse_cmd("!toggle joe", &project, &obs, &state).unwrap()).unwrap();

        let saved_state = serde_json::to_string(&state.to_save_state()).unwrap();
        let regen_state_serial: ProjectStateSerializer = serde_json::from_str(&saved_state).unwrap();
        let regen_state = ProjectState::from_save_state(&regen_state_serial, &project, &obs).unwrap();

        assert_eq!(state, regen_state);
    }
}
