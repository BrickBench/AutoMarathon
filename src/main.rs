use std::{
    collections::HashMap,
    fs::{self, read_to_string, File},
    path::PathBuf,
};

use clap::{Parser, Subcommand};

use discord::test_discord;
use error::Error;
use futures::{future::BoxFuture, FutureExt};
use obs::update_obs;
use state::{Project, ProjectState, ProjectStateSerializer, ProjectType};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use user::Player;

use crate::{
    cmd::parse_cmds, discord::init_discord, obs::LayoutFile, settings::Settings,
    state::ProjectTypeStateSerializer,
};

mod cmd;
mod discord;
mod error;
mod obs;
mod settings;
mod state;
mod user;

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

    /// Validate a configuration file.
    Test {
        /// Configuration file to validate.
        #[arg(short, long)]
        settings_file: PathBuf,
    },

    /// Run a project sourced from a project file.
    Run {
        /// Location for project state save file.
        #[arg(short, long)]
        save_file: PathBuf,

        /// Location for OBS configuration and layout file.
        /// A sample is provided in the `obs/layouts` directory.
        #[arg(short, long)]
        layout_file: PathBuf,

        /// Location of Discord configuration file.
        /// If provided, Discord integration will be enabled.
        /// This configuration file can be created with the `AutoMarathon discord` command.
        #[arg(short, long)]
        settings_file: PathBuf,

        project_file: PathBuf,
    },
}

/// A type for inter-thread communication with a string command and a callback for returning errors.
type CommandMessage = (
    String,
    Box<dyn Send + Sync + FnOnce(Option<String>) -> BoxFuture<'static, ()>>,
);

async fn run_update<'a>(
    project: Project,
    state_src: ProjectStateSerializer,
    obs_cfg: LayoutFile,
    state_file: PathBuf,
    mut rx: UnboundedReceiver<CommandMessage>,
    obs: obws::Client,
) {
    let mut state = ProjectState::from_save_state(&state_src, &project, &obs_cfg).unwrap();

    state.running = true;

    loop {
        println!("At gaming");
        let (msg, resp) = rx.recv().await.unwrap();

        println!("Processing {}", msg);

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

                        if let Err(e) = update_obs(&state, &obs_cfg, &obs).await {
                            println!("Error while applying layout: {}", e.to_string())
                        }

                        let state_save =
                            serde_json::to_string_pretty(&state.to_save_state()).unwrap();
                        fs::write(&state_file, state_save).expect("Failed to save file.");

                        resp.await.expect("Error in response.");
                    }
                    Err(err) => {
                        resp(Some(err.to_string())).await;
                    }
                }
            }
            Err(err) => {
                resp(Some(err.to_string())).await;
            }
        }
    }
}

async fn read_terminal(tx: UnboundedSender<CommandMessage>) {
    loop {
        let mut cmd: String = String::new();
        std::io::stdin()
            .read_line(&mut cmd)
            .expect("Failed to read from stdin");

        println!("GEE {:?}", cmd);

        if cmd.to_lowercase() == "exit" {
            break;
        } else {
            tx.send((
                cmd,
                Box::new(move |e: Option<String>| {
                    async move {
                        if let Some(err) = e {
                            println!("Failed to run command: {}", err);
                        }
                    }
                    .boxed()
                }),
            ))
            .map_err(|_| "Failed to send command.")
            .unwrap();
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args = Args {
        command: RunType::Run {
            save_file: PathBuf::from("save.json"),
            layout_file: PathBuf::from("layouts.json"),
            settings_file: PathBuf::from("settings.json"),
            project_file: PathBuf::from("proj.json"),
        },
    }; //Args::parse();

    match &args.command {
        RunType::Create {
            project_type,
            players,
            project_file,
        } => {
            let new_project = Project {
                kind: project_type.to_owned(),
                players: players
                    .iter()
                    .map(|p| Player {
                        name: p.to_owned(),
                        nicks: vec![],
                        twitch: "FILL_THIS".to_owned(),
                    })
                    .collect(),
            };

            let new_json = serde_json::to_string_pretty(&new_project).unwrap();

            fs::write(project_file, new_json)?;

            println!("Project created, please open the file in a text editor and fill in the missing fields.");
            Ok(())
        }
        RunType::Test { settings_file } => {
            let settings: Settings =
                serde_json::from_str::<Settings>(&read_to_string(&settings_file)?)?;

            test_discord(&settings.discord_token.unwrap()).await?;

            Ok(())
        }
        RunType::Run {
            save_file,
            layout_file,
            settings_file,
            project_file,
        } => {
            // Load  layouts
            let layouts: LayoutFile =
                serde_json::from_str::<LayoutFile>(&read_to_string(&layout_file)?)?;
            // Load project
            let project: Project =
                serde_json::from_str::<Project>(&read_to_string(&project_file)?)?;
            // Load settings
            let settings: Settings =
                serde_json::from_str::<Settings>(&read_to_string(&settings_file)?)?;

            println!("Loaded project");

            // Load or create project state
            let state: ProjectStateSerializer = match read_to_string(save_file) {
                Ok(file) => {
                    println!("Loading project state file...");
                    let save_res = serde_json::from_str::<ProjectStateSerializer>(&file);
                    match save_res {
                        Ok(save) => {
                            ProjectState::from_save_state(&save, &project, &layouts)
                                .map_err(|e| e.to_string())?;
                            save
                        }
                        Err(_) => ProjectStateSerializer {
                            active_players: vec![],
                            type_state: ProjectTypeStateSerializer::MarathonState {
                                runner_times: HashMap::new(),
                            },
                            layouts_by_count: HashMap::new(),
                        },
                    }
                }
                Err(_) => {
                    println!("Project state file does not exist, creating...");
                    File::create(save_file)?;
                    ProjectStateSerializer {
                        active_players: vec![],
                        type_state: ProjectTypeStateSerializer::MarathonState {
                            runner_times: HashMap::new(),
                        },
                        layouts_by_count: HashMap::new(),
                    }
                }
            };

            let (tx, rx): (
                UnboundedSender<CommandMessage>,
                UnboundedReceiver<CommandMessage>,
            ) = mpsc::unbounded_channel();

            // Initialize OBS websocket
            println!("Connecting to OBS");
            let obs = obws::Client::connect(
                settings.obs_ip.to_owned().unwrap_or("localhost".to_owned()),
                settings.obs_port.to_owned().unwrap_or(4455),
                settings.obs_password.to_owned(),
            )
            .await?;

            let obs_version = obs.general().version().await?;
            println!(
                "Connected to OBS version {}, websocket {}, running on {} ({})",
                obs_version.obs_version.to_string(),
                obs_version.obs_web_socket_version.to_string(),
                obs_version.platform,
                obs_version.platform_description
            );

            println!("AutoMarathon initialized");

            let terminal_task = read_terminal(tx);
            let work_task = run_update(project, state, layouts, save_file.to_owned(), rx, obs);
            if let Some(_token) = &settings.discord_token {
                /* let (discord, work, terminal) = tokio::join!(
                    tokio::spawn(init_discord(settings.clone(), tx.clone())),
                    tokio::spawn(work_task),
                    tokio::spawn(terminal_task)
                );

                discord.unwrap()?;
                work.unwrap();
                terminal.unwrap();*/
            } else {
                let (terminal, work) =
                    tokio::join!(tokio::spawn(terminal_task), tokio::spawn(work_task));

                work.unwrap();
                terminal.unwrap();
            }

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use crate::{
        cmd::{parse_cmd, Command},
        obs::LayoutFile,
        state::{Project, ProjectState, ProjectStateSerializer, ProjectType, ProjectTypeState},
        user::Player,
    };

    #[test]
    fn test_nicks() {
        let project = Project {
            kind: ProjectType::Marathon,
            players: vec![
                Player {
                    name: "Joe".to_owned(),
                    nicks: vec!["Joseph".to_owned(), "John".to_owned()],
                    twitch: "test".to_owned(),
                },
                Player {
                    name: "William".to_owned(),
                    nicks: vec!["Will".to_owned(), "Bill".to_owned()],
                    twitch: "test2".to_owned(),
                },
            ],
        };

        assert_eq!(project.find_by_nick("Joe").unwrap().name, "Joe");
        assert_eq!(project.find_by_nick("john").unwrap().name, "Joe");
        assert!(project.find_by_nick("Jow").is_none());
    }

    #[test]
    fn test_cmds() {
        let project = Project {
            kind: ProjectType::Marathon,
            players: vec![
                Player {
                    name: "Joe".to_owned(),
                    nicks: vec!["Joseph".to_owned(), "John".to_owned()],
                    twitch: "test".to_owned(),
                },
                Player {
                    name: "William".to_owned(),
                    nicks: vec!["Will".to_owned(), "Bill".to_owned()],
                    twitch: "test2".to_owned(),
                },
            ],
        };

        let state = ProjectState {
            running: false,
            active_players: vec![],
            streams: HashMap::new(),
            type_state: ProjectTypeState::MarathonState {
                runner_times: HashMap::new(),
            },
            layouts_by_count: HashMap::new(),
        };

        let layouts = LayoutFile {
            scene: "Scene".to_string(),
            layouts: HashMap::new(),
        };

        assert_eq!(
            Command::Toggle(&project.players[0]),
            parse_cmd("!toggle john", &project, &layouts, &state).unwrap()
        );

        assert_eq!(
            Command::Swap(&project.players[0], &project.players[1]),
            parse_cmd("!swap john bill", &project, &layouts, &state).unwrap()
        );
        assert!(parse_cmd("!swap john sam", &project, &layouts, &state).is_err());
        assert!(parse_cmd("!swap john will bill", &project, &layouts, &state).is_err());
        assert!(parse_cmd("!swap bill will", &project, &layouts, &state).is_err());

        assert_eq!(
            Command::SetPlayers(vec!(&project.players[0], &project.players[1])),
            parse_cmd("!set john bill", &project, &layouts, &state).unwrap()
        );
        assert!(parse_cmd("!set earl bill", &project, &layouts, &state).is_err());
        assert!(parse_cmd("!set bill will dill", &project, &layouts, &state).is_err());

        assert_eq!(
            Command::SetScore(&project.players[0], Duration::from_millis(222)),
            parse_cmd("!record joe 222", &project, &layouts, &state).unwrap()
        );
        assert!(parse_cmd("!record joe 22e", &project, &layouts, &state).is_err());
    }

    #[test]
    fn test_cmd_apply() {
        let project = Project {
            kind: ProjectType::Marathon,
            players: vec![
                Player {
                    name: "Joe".to_owned(),
                    nicks: vec!["Joseph".to_owned(), "John".to_owned()],
                    twitch: "test".to_owned(),
                },
                Player {
                    name: "William".to_owned(),
                    nicks: vec!["Will".to_owned(), "Bill".to_owned()],
                    twitch: "test2".to_owned(),
                },
            ],
        };

        let mut state = ProjectState {
            running: false,
            active_players: vec![],
            streams: HashMap::new(),
            type_state: ProjectTypeState::MarathonState {
                runner_times: HashMap::new(),
            },
            layouts_by_count: HashMap::new(),
        };

        let layouts = LayoutFile {
            scene: "Scene".to_string(),
            layouts: HashMap::new(),
        };

        state = state
            .apply_cmd(&parse_cmd("!toggle joe", &project, &layouts, &state).unwrap())
            .unwrap();
        assert_eq!(state.active_players[0].name, "Joe");

        state = state
            .apply_cmd(&parse_cmd("!swap joe will", &project, &layouts, &state).unwrap())
            .unwrap();
        assert_eq!(state.active_players[0].name, "William");

        state = state
            .apply_cmd(&parse_cmd("!swap joe will", &project, &layouts, &state).unwrap())
            .unwrap();
        assert_eq!(state.active_players[0].name, "Joe");

        state = state
            .apply_cmd(&parse_cmd("!set joe will", &project, &layouts, &state).unwrap())
            .unwrap();
        assert_eq!(state.active_players[0].name, "Joe");
        assert_eq!(state.active_players[1].name, "William");

        state = state
            .apply_cmd(&parse_cmd("!swap joe will", &project, &layouts, &state).unwrap())
            .unwrap();
        assert_eq!(state.active_players[0].name, "William");
        assert_eq!(state.active_players[1].name, "Joe");

        state = state
            .apply_cmd(&parse_cmd("!toggle joe", &project, &layouts, &state).unwrap())
            .unwrap();
        assert_eq!(state.active_players[0].name, "William");

        assert!(state
            .apply_cmd(&parse_cmd("!refresh", &project, &layouts, &state).unwrap())
            .is_err());
    }

    #[test]
    fn test_serialization() {
        let layouts = LayoutFile {
            scene: "Scene".to_string(),
            layouts: HashMap::new(),
        };

        let project = Project {
            kind: ProjectType::Marathon,
            players: vec![
                Player {
                    name: "Joe".to_owned(),
                    nicks: vec!["Joseph".to_owned(), "John".to_owned()],
                    twitch: "test".to_owned(),
                },
                Player {
                    name: "William".to_owned(),
                    nicks: vec!["Will".to_owned(), "Bill".to_owned()],
                    twitch: "test2".to_owned(),
                },
            ],
        };

        let saved_project = serde_json::to_string(&project).unwrap();
        let regen_project: Project = serde_json::from_str(&saved_project).unwrap();

        assert_eq!(project, regen_project);

        let mut state = ProjectState {
            running: false,
            active_players: vec![],
            streams: HashMap::new(),
            type_state: ProjectTypeState::MarathonState {
                runner_times: HashMap::new(),
            },
            layouts_by_count: HashMap::new(),
        };

        state = state
            .apply_cmd(&parse_cmd("!toggle joe", &project, &layouts, &state).unwrap())
            .unwrap();

        let saved_state = serde_json::to_string(&state.to_save_state()).unwrap();
        let regen_state_serial: ProjectStateSerializer =
            serde_json::from_str(&saved_state).unwrap();
        let regen_state =
            ProjectState::from_save_state(&regen_state_serial, &project, &layouts).unwrap();

        assert_eq!(state, regen_state);
    }
}
