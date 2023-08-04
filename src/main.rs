use std::{
    collections::HashMap,
    fs::{self, read_to_string, File},
    path::PathBuf,
};

use clap::{Parser, Subcommand};

use error::Error;
use log::{debug, error, info, warn};
use tokio::{
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        oneshot,
    },
    task::JoinSet,
};

use crate::{
    cmd::{parse_cmds, Command},
    discord::init_discord,
    discord::test_discord,
    obs::update_obs,
    obs::LayoutFile,
    player::Player,
    settings::Settings,
    state::{FieldStateSerializer, Project, ProjectState, ProjectStateSerializer, ProjectTemplate},
    therun::create_player_websocket,
    web::run_http_server,
};

mod cmd;
mod discord;
mod error;
mod obs;
mod player;
mod settings;
mod state;
mod therun;
mod web;

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
        project_type: ProjectTemplate,

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

pub enum Response {
    Ok,
    Error(String),
    CurrentState(ProjectStateSerializer),
}

/// A type for inter-thread communication with a string command and a callback for returning errors.
type CommandMessage = (String, Option<oneshot::Sender<Response>>);

async fn run_update<'a>(
    project: Project,
    state_src: ProjectStateSerializer,
    obs_cfg: LayoutFile,
    state_file: PathBuf,
    mut rx: UnboundedReceiver<CommandMessage>,
    obs: obws::Client,
) -> Result<(), Error> {
    let mut state = ProjectState::from_save_state(&state_src, &project, &obs_cfg).unwrap();

    state.running = true;

    loop {
        let (msg, resp) = rx.recv().await.unwrap();

        let mut resp_msg = Response::Ok;

        debug!("Processing {}", msg);

        if msg.starts_with("exit") {
            break Ok(());
        }

        let cmd = parse_cmds(&msg, &project, &obs_cfg, &state);

        match &cmd {
            Ok(cmds) if cmds.len() == 1 && cmds[0] == Command::GetState => {
                resp_msg = Response::CurrentState(state.to_save_state());
            }
            Ok(cmds) => {
                let state_res = state.apply_cmds(cmds);
                match state_res {
                    Ok((new_state, modifications)) => {
                        state = new_state;

                        match update_obs(&project, &state, modifications, &obs_cfg, &obs).await {
                            Ok(_) => {
                                let state_save =
                                    serde_json::to_string_pretty(&state.to_save_state()).unwrap();
                                fs::write(&state_file, state_save).expect("Failed to save file.");
                            }
                            Err(err) => {
                                error!("Error while applying layout: {}", err.to_string());
                                resp_msg = Response::Error(err.to_string());
                            }
                        }
                    }
                    Err(err) => {
                        error!("Error while applying commands: {}", err.to_string());
                        resp_msg = Response::Error(err.to_string());
                    }
                }
            }
            Err(err) => {
                error!("Error while parsing commands: {}", err.to_string());
                resp_msg = Response::Error(err.to_string());
            }
        }

        if let Some(rx) = resp {
            rx.send(resp_msg);
        }
    }
}

async fn read_terminal(tx: UnboundedSender<CommandMessage>) -> Result<(), Error> {
    loop {
        let mut cmd: String = String::new();
        std::io::stdin()
            .read_line(&mut cmd)
            .expect("Failed to read from stdin");

        if cmd.to_lowercase() == "exit" {
            break Ok(());
        } else {
            let (rtx, rrx) = oneshot::channel();
            let _ = tx.send((cmd, Some(rtx)));

            if let Ok(resp) = rrx.await {
                match resp {
                    Response::Ok => {}
                    Response::Error(message) => {
                        error!("Failed to run command: {}", message);
                    }
                    Response::CurrentState(value) => info!("{:?}", value),
                }
            }
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

    env_logger::builder()
        .filter_level(log::LevelFilter::Warn)
        .init();

    match &args.command {
        RunType::Create {
            project_type,
            players,
            project_file,
        } => {
            let new_project = Project {
                template: project_type.to_owned(),
                players: players
                    .iter()
                    .map(|p| Player {
                        name: p.to_owned(),
                        nicks: vec![],
                        stream: "FILL_THIS".to_owned(),
                    })
                    .collect(),
                integrations: [].to_vec(),
                extra_fields: [].to_vec(),
            };

            let new_json = serde_json::to_string_pretty(&new_project).unwrap();

            fs::write(project_file, new_json)?;

            info!("Project created, please open the file in a text editor and fill in the missing fields.");
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

            info!("Loaded project");

            // Load or create project state
            let state: ProjectStateSerializer = match read_to_string(save_file) {
                Ok(file) => {
                    info!("Loading project state file...");
                    let save_res = serde_json::from_str::<ProjectStateSerializer>(&file);
                    match save_res {
                        Ok(save) => {
                            ProjectState::from_save_state(&save, &project, &layouts)
                                .map_err(|e| e.to_string())?;
                            save
                        }
                        Err(_) => ProjectStateSerializer {
                            active_players: vec![],
                            player_fields: FieldStateSerializer {
                                player_fields: project
                                    .players
                                    .iter()
                                    .map(|p| (p.name.clone(), HashMap::new()))
                                    .collect(),
                            },
                            layouts_by_count: HashMap::new(),
                            active_commentators: vec![],
                            ignored_commentators: vec![],
                        },
                    }
                }
                Err(_) => {
                    info!("Project state file does not exist, creating...");
                    File::create(save_file)?;
                    ProjectStateSerializer {
                        active_players: vec![],
                        player_fields: FieldStateSerializer {
                            player_fields: project
                                .players
                                .iter()
                                .map(|p| (p.name.clone(), HashMap::new()))
                                .collect(),
                        },
                        layouts_by_count: HashMap::new(),
                        active_commentators: vec![],
                        ignored_commentators: vec![],
                    }
                }
            };

            let (tx, rx): (
                UnboundedSender<CommandMessage>,
                UnboundedReceiver<CommandMessage>,
            ) = mpsc::unbounded_channel();

            // Initialize OBS websocket
            info!("Connecting to OBS");
            let obs = obws::Client::connect(
                settings.obs_ip.to_owned().unwrap_or("localhost".to_owned()),
                settings.obs_port.to_owned().unwrap_or(4455),
                settings.obs_password.to_owned(),
            )
            .await?;

            let obs_version = obs.general().version().await?;
            info!(
                "Connected to OBS version {}, websocket {}, running on {} ({})",
                obs_version.obs_version.to_string(),
                obs_version.obs_web_socket_version.to_string(),
                obs_version.platform,
                obs_version.platform_description
            );

            info!("AutoMarathon initialized");

            let mut tasks = JoinSet::<Result<(), Error>>::new();

            if project.integrations.contains(&state::Integration::TheRun) {
                for player in &project.players {
                    let twitch_handle = player.stream.split('/').last().unwrap();
                    tasks.spawn(create_player_websocket(
                        player.name.clone(),
                        twitch_handle.to_string(),
                        tx.clone(),
                    ));
                }
            }

            if project.integrations.contains(&state::Integration::Discord) {
                tasks.spawn(init_discord(settings.clone(), tx.clone()));
            }

            tasks.spawn(read_terminal(tx.clone()));
            tasks.spawn(run_http_server(tx.clone()));
            tasks.spawn(run_update(
                project,
                state,
                layouts,
                save_file.to_owned(),
                rx,
                obs,
            ));

            loop {
                match tasks.join_next().await {
                    Some(Ok(_)) => info!("Service Done"),
                    Some(Err(err)) => error!("{}", err.to_string()),
                    None => break Ok(()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use crate::{
        cmd::{parse_cmd, Command},
        obs::LayoutFile,
        player::Player,
        state::{FieldState, Project, ProjectState, ProjectStateSerializer, ProjectTemplate},
    };

    #[test]
    fn test_nicks() {
        let project = Project {
            kind: ProjectTemplate::Marathon,
            players: vec![
                Player {
                    name: "Joe".to_owned(),
                    nicks: vec!["Joseph".to_owned(), "John".to_owned()],
                    stream: "test".to_owned(),
                },
                Player {
                    name: "William".to_owned(),
                    nicks: vec!["Will".to_owned(), "Bill".to_owned()],
                    stream: "test2".to_owned(),
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
            kind: ProjectTemplate::Marathon,
            players: vec![
                Player {
                    name: "Joe".to_owned(),
                    nicks: vec!["Joseph".to_owned(), "John".to_owned()],
                    stream: "test".to_owned(),
                },
                Player {
                    name: "William".to_owned(),
                    nicks: vec!["Will".to_owned(), "Bill".to_owned()],
                    stream: "test2".to_owned(),
                },
            ],
        };

        let state = ProjectState {
            running: false,
            active_players: vec![],
            streams: HashMap::new(),
            player_fields: FieldState::MarathonState {
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
            Command::SetEventRecord(&project.players[0], Duration::from_millis(222)),
            parse_cmd("!record joe 222", &project, &layouts, &state).unwrap()
        );
        assert!(parse_cmd("!record joe 22e", &project, &layouts, &state).is_err());
    }

    #[test]
    fn test_cmd_apply() {
        let project = Project {
            kind: ProjectTemplate::Marathon,
            players: vec![
                Player {
                    name: "Joe".to_owned(),
                    nicks: vec!["Joseph".to_owned(), "John".to_owned()],
                    stream: "test".to_owned(),
                },
                Player {
                    name: "William".to_owned(),
                    nicks: vec!["Will".to_owned(), "Bill".to_owned()],
                    stream: "test2".to_owned(),
                },
            ],
        };

        let mut state = ProjectState {
            running: false,
            active_players: vec![],
            streams: HashMap::new(),
            player_fields: FieldState::MarathonState {
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
            kind: ProjectTemplate::Marathon,
            players: vec![
                Player {
                    name: "Joe".to_owned(),
                    nicks: vec!["Joseph".to_owned(), "John".to_owned()],
                    stream: "test".to_owned(),
                },
                Player {
                    name: "William".to_owned(),
                    nicks: vec!["Will".to_owned(), "Bill".to_owned()],
                    stream: "test2".to_owned(),
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
            player_fields: FieldState::MarathonState {
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
