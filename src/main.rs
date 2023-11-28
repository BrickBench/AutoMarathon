use std::{
    collections::HashMap,
    fs::{self, read_to_string, File},
    path::PathBuf,
    sync::Arc, 
    env::consts,
};

use clap::{Parser, Subcommand};

use cmd::CommandSource;
use error::Error;
use tokio::{
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        oneshot,
    },
    task::JoinSet,
};

use crate::{
    cmd::{parse_cmd, Command},
    integrations::discord::test_discord,
    obs::update_obs,
    obs::LayoutFile,
    player::Player,
    settings::Settings,
    state::{FieldStateSerializer, Project, ProjectState, ProjectStateSerializer, ProjectTemplate},
};

mod cmd;
mod error;
mod integrations;
mod obs;
mod player;
mod settings;
mod state;

const AUTOMARATHON_VER: &str = "0.1";

#[derive(Parser, Debug)]
#[command(name = "AutoMarathon")]
#[command(author = "javster101")]
#[command(version = AUTOMARATHON_VER)]
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

/// Response types for CommandMessage
pub enum Response {
    Ok,
    Error(String),
    CurrentState(ProjectStateSerializer),
}

/// A string message with optional response channel
type CommandMessage = (CommandSource, Option<oneshot::Sender<Response>>);

async fn run_update<'a>(
    project: Arc<Project>,
    state_src: ProjectStateSerializer,
    obs_cfg: Arc<LayoutFile>,
    settings: Arc<Settings>,
    state_file: PathBuf,
    mut rx: UnboundedReceiver<CommandMessage>,
    obs: obws::Client,
) -> Result<(), Error> {
    let mut state = ProjectState::from_save_state(&state_src, &project, &obs_cfg).unwrap();

    state.running = true;

    loop {
        let (msg, resp) = rx.recv().await.unwrap();

        let mut resp_msg = Response::Ok;

        log::debug!("Processing {:?}", msg);

        let cmd = parse_cmd(&msg, &project, &obs_cfg);

        match &cmd {
            Ok(cmds) if cmds == &Command::GetState => {
                resp_msg = Response::CurrentState(state.to_save_state());
            }
            Ok(cmds) => {
                let state_res = state.apply_cmd(cmds);
                match state_res {
                    Ok((new_state, modifications)) => {
                        if !modifications.is_empty() {
                            match update_obs(
                                &project,
                                &new_state,
                                &settings,
                                modifications,
                                &obs_cfg,
                                &obs,
                            )
                            .await
                            {
                                Ok(_) => {
                                    state = new_state;
                                }
                                Err(err) => {
                                    log::error!("Error while applying layout: {}", err.to_string());
                                    resp_msg = Response::Error(err.to_string());
                                }
                            }
                        } else {
                            state = new_state;
                        }

                        let state_save =
                            serde_json::to_string_pretty(&state.to_save_state()).unwrap();
                        fs::write(&state_file, state_save).expect("Failed to save file.");
                    }
                    Err(err) => {
                        log::error!("Error while applying commands: {}", err.to_string());
                        resp_msg = Response::Error(err.to_string());
                    }
                }
            }
            Err(err) => {
                log::error!("Error while parsing commands: {}", err.to_string());
                resp_msg = Response::Error(err.to_string());
            }
        }

        if let Some(rx) = resp {
            let _ = rx.send(resp_msg);
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
            //let _ = tx.send((cmd, Some(rtx)));

            if let Ok(resp) = rrx.await {
                match resp {
                    Response::Ok => {}
                    Response::Error(message) => {
                        log::error!("Failed to run command: {}", message);
                    }
                    Response::CurrentState(value) => log::info!("{:?}", value),
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
        .filter(Some("tracing::span"), log::LevelFilter::Warn)
        .filter(Some("serenity"), log::LevelFilter::Warn)
        .filter(Some("hyper"), log::LevelFilter::Warn)
        .filter_level(log::LevelFilter::Debug)
        .init();

    log::info!("Launching AutoMarathon {} on {}", AUTOMARATHON_VER, consts::OS);

    match &args.command {
        // Create an empty project file
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
                        therun: None,
                    })
                    .collect(),
                integrations: [].to_vec(),
                extra_fields: [].to_vec(),
            };

            let new_json = serde_json::to_string_pretty(&new_project).unwrap();

            fs::write(project_file, new_json)?;

            log::info!("Project created, please open the file in a text editor and fill in the missing fields.");
            Ok(())
        }

        // Test for a valid project file
        RunType::Test { settings_file } => {
            let settings: Settings =
                serde_json::from_str::<Settings>(&read_to_string(&settings_file)?)?;

            test_discord(&settings.discord_token.unwrap()).await?;

            Ok(())
        }

        // Run a project
        RunType::Run {
            save_file,
            layout_file,
            settings_file,
            project_file,
        } => {
            // Load  layouts
            let layouts: Arc<LayoutFile> = Arc::new(serde_json::from_str::<LayoutFile>(
                &read_to_string(&layout_file)?,
            )?);

            // Load project
            let project: Arc<Project> = Arc::new(serde_json::from_str::<Project>(
                &read_to_string(&project_file)?,
            )?);

            // Load settings
            let settings: Arc<Settings> = Arc::new(serde_json::from_str::<Settings>(
                &read_to_string(&settings_file)?,
            )?);

            log::info!("Loaded project");

            // Load or create project state
            let state: ProjectStateSerializer = match read_to_string(save_file) {
                Ok(file) => {
                    log::info!("Loading project state file...");
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
                                event_fields: HashMap::new(),
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
                    log::info!("Project state file does not exist, creating...");
                    File::create(save_file)?;
                    ProjectStateSerializer {
                        active_players: vec![],
                        player_fields: FieldStateSerializer {
                            event_fields: HashMap::new(),
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

            // Initialize OBS websocket
            log::info!("Connecting to OBS");
            let obs = obws::Client::connect(
                settings.obs_ip.to_owned().unwrap_or("localhost".to_owned()),
                settings.obs_port.to_owned().unwrap_or(4455),
                settings.obs_password.to_owned(),
            )
            .await?;

            let obs_version = obs.general().version().await?;
            log::info!(
                "Connected to OBS version {}, websocket {}, running on {} ({})",
                obs_version.obs_version.to_string(),
                obs_version.obs_web_socket_version.to_string(),
                obs_version.platform,
                obs_version.platform_description
            );

            log::info!("AutoMarathon initialized");

            // Set up messaging channels
            let (command_sender, command_receiver): (
                UnboundedSender<CommandMessage>,
                UnboundedReceiver<CommandMessage>,
            ) = mpsc::unbounded_channel();

            let mut tasks = JoinSet::<Result<(), Error>>::new();

            // Spawn optional integrations
            integrations::init_integrations(
                &mut tasks,
                command_sender.clone(),
                settings.clone(),
                layouts.clone(),
                project.clone(),
            );

            // Spawn main task.
            tasks.spawn(read_terminal(command_sender.clone()));
            tasks.spawn(run_update(
                project,
                state,
                layouts,
                settings,
                save_file.to_owned(),
                command_receiver,
                obs,
            ));

            loop {
                match tasks.join_next().await {
                    Some(Ok(_)) => log::info!("Service Done"),
                    Some(Err(err)) => log::error!("{}", err.to_string()),
                    None => break Ok(()),
                }
            }
        }
    }
}
