use std::{
    env::consts,
    fs::{self, read_to_string},
    path::PathBuf,
    sync::Arc, collections::HashMap,
};

use clap::{Parser, Subcommand};

use project::ProjectType;
use tokio::{
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        oneshot,
    },
    task::JoinSet,
};

use crate::{
    integrations::discord::test_discord,
    obs::{run_obs, LayoutFile, ObsActor},
    player::Player,
    project::{Project, ProjectTypeSettings},
    settings::Settings,
    state::{run_state_manager, StateActor},
};

mod error;
mod integrations;
mod obs;
mod player;
mod project;
mod settings;
mod state;

const AUTOMARATHON_VER: &str = "0.1";

struct ActorRef<T> {
    tx: UnboundedSender<T>,
}

impl<T> ActorRef<T> {
    pub fn send(&self, msg: T) {
        let _ = self.tx.send(msg);
    }

    pub fn new() -> (Self, UnboundedReceiver<T>) {
        let (tx, rx) = mpsc::unbounded_channel();

        (Self { tx }, rx)
    }
}

impl<T> Clone for ActorRef<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

struct Rto<T> {
    rto: oneshot::Sender<Result<T, anyhow::Error>>,
}

impl<T> Rto<T> {
    pub fn reply(self, msg: Result<T, anyhow::Error>) {
        let _ = self.rto.send(msg);
    }

    pub fn new() -> (Rto<T>, oneshot::Receiver<Result<T, anyhow::Error>>) {
        let (tx, rx) = oneshot::channel::<Result<T, anyhow::Error>>();
        (Self { rto: tx }, rx)
    }
}

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
        project_folder: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();

    env_logger::builder()
        .filter(Some("tracing::span"), log::LevelFilter::Warn)
        .filter(Some("serenity"), log::LevelFilter::Warn)
        .filter(Some("hyper"), log::LevelFilter::Warn)
        .filter_level(log::LevelFilter::Info)
        .init();

    log::info!(
        "Launching AutoMarathon {} on {}",
        AUTOMARATHON_VER,
        consts::OS
    );

    match &args.command {
        // Create an empty project file
        RunType::Create {
            project_type,
            players,
            project_file,
        } => {
            let new_project_type_data = match &project_type {
                ProjectType::Marathon => ProjectTypeSettings::Marathon,
                ProjectType::Relay => ProjectTypeSettings::Relay { teams: HashMap::new() },
            };

            let new_project = Project {
                project_type: new_project_type_data,
                players: players
                    .iter()
                    .map(|p| Player {
                        name: p.to_owned(),
                        nicks: Some(vec![]),
                        stream: Some("FILL_THIS".to_owned()),
                        therun: None,
                    })
                    .collect(),
                integrations: [].to_vec(),
            };

            let new_json = serde_json::to_string_pretty(&new_project).unwrap();

            fs::write(project_file, new_json)?;

            log::info!("Project created, please open the file in a text editor and fill in the missing fields.");
            Ok(())
        }

        // Test for a valid project file
        RunType::Test { settings_file } => {
            let settings: Settings =
                serde_json::from_str::<Settings>(&read_to_string(settings_file)?)?;

            test_discord(&settings.discord_token.unwrap()).await?;

            Ok(())
        }

        // Run a project
        RunType::Run {
            project_folder
        } => {
            // Load layouts
            let layouts: Arc<LayoutFile> = Arc::new(serde_json::from_str::<LayoutFile>(
                &read_to_string(project_folder.join("layouts.json"))?,
            )?);

            // Load project
            let project: Arc<Project> = Arc::new(serde_json::from_str::<Project>(
                &read_to_string(project_folder.join("project.json"))?,
            )?);

            // Load settings
            let settings: Arc<Settings> = Arc::new(serde_json::from_str::<Settings>(
                &read_to_string(project_folder.join("settings.json"))?,
            )?);

            log::info!("Loaded project");

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

            let mut tasks = JoinSet::<Result<(), anyhow::Error>>::new();

            // Set up messaging channels
            let (state_actor, state_rx) = StateActor::new();
            let (obs_actor, obs_rx) = ObsActor::new();

            // Spawn optional integrations
            integrations::init_integrations(
                &mut tasks,
                settings.clone(),
                layouts.clone(),
                project.clone(),
                state_actor.clone(),
            );

            // Spawn OBS task.
            tasks.spawn(run_obs(
                project.clone(),
                settings,
                layouts.clone(),
                obs,
                obs_rx,
            ));

            // Spawn main task.
            tasks.spawn(run_state_manager(
                project,
                layouts,
                project_folder.join("save.json").to_owned(),
                state_rx,
                obs_actor.clone(),
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
