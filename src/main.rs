use std::{
    env::consts,
    fs::read_to_string,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use anyhow::anyhow;
use clap::{Parser, Subcommand};

use obws::requests::EventSubscription;
use sqlx::{migrate::MigrateDatabase, query, sqlite::Sqlite, SqlitePool};
use tokio::{
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        oneshot, RwLock,
    },
    task::JoinSet,
};

use crate::{
    integrations::discord::test_discord,
    obs::{run_obs, LayoutFile, ObsActor},
    project::{Project, ProjectStore},
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

/// Actor reference
struct ActorRef<T> {
    tx: UnboundedSender<T>,
}

impl<T> ActorRef<T> {
    /// Send a message to the provided actor
    pub fn send(&self, msg: T) {
        let _ = self.tx.send(msg);
    }

    /// Spawn an actor, returning an ActorRef and the corresponding receiver
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

/// Oneshot fallible return channel
struct Rto<T> {
    rto: oneshot::Sender<anyhow::Result<T>>,
}

impl<T> Rto<T> {
    /// Send a reply through this Rto
    pub fn reply(self, msg: anyhow::Result<T>) {
        let _ = self.rto.send(msg);
    }

    pub fn new() -> (Rto<T>, oneshot::Receiver<anyhow::Result<T>>) {
        let (tx, rx) = oneshot::channel::<anyhow::Result<T>>();
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
    Create { project_file: PathBuf },

    /// Validate a configuration file.
    Test {
        /// Configuration file to validate.
        #[arg(short, long)]
        settings_file: PathBuf,
    },

    /// Run a project sourced from a project file.
    Run { project_folder: PathBuf },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    env_logger::builder()
        .filter(Some("tracing::span"), log::LevelFilter::Warn)
        .filter(Some("serenity"), log::LevelFilter::Warn)
        .filter(Some("hyper"), log::LevelFilter::Warn)
        .filter(Some("h2"), log::LevelFilter::Warn)
        .filter(Some("rustls"), log::LevelFilter::Warn)
        .filter_level(log::LevelFilter::Debug)
        .init();

    log::info!(
        "Launching AutoMarathon {} on {}",
        AUTOMARATHON_VER,
        consts::OS
    );

    match &args.command {
        // Create an empty project file
        RunType::Create { project_file } => {
            let url = format!("sqlite://{}", project_file.to_str().unwrap());
            Sqlite::create_database(&url).await?;

            let db = SqlitePool::connect(&url).await?;
            let project_table = sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY NOT NULL,
                    display_name VARCHAR(250) NOT NULL, stream VARCHAR(200) NOT NULL, therun VARCHAR(250));")
                    .execute(&db).await.unwrap();

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
        RunType::Run { project_folder } => {
            // Load layouts
            let layouts: Arc<LayoutFile> = Arc::new(
                serde_json::from_str::<LayoutFile>(&read_to_string(
                    project_folder.join("layouts.json"),
                )?)
                .map_err(|e| anyhow!(format!("Error while loading layouts.json: {:?}", e)))?,
            );

            // Load project
            let project: ProjectStore = Arc::new(RwLock::new(
                serde_json::from_str::<Project>(&read_to_string(
                    project_folder.join("project.json"),
                )?)
                .map_err(|e| anyhow!(format!("Error while loading project.json: {:?}", e)))?,
            ));

            // Load settings
            let settings: Arc<Settings> = Arc::new(
                serde_json::from_str::<Settings>(&read_to_string(
                    project_folder.join("settings.json"),
                )?)
                .map_err(|e| anyhow!(format!("Error while loading settings.json: {:?}", e)))?,
            );

            log::info!("Loaded project");

            // Initialize OBS websocket
            log::info!("Connecting to OBS");
            let obs_config = obws::client::ConnectConfig {
                host: settings.obs_ip.to_owned().unwrap_or("localhost".to_owned()),
                port: settings.obs_port.to_owned().unwrap_or(4455),
                password: settings.obs_password.to_owned(),
                event_subscriptions: Some(EventSubscription::NONE),
                broadcast_capacity: None,
                connect_timeout: Duration::from_secs(30),
            };
            let obs = obws::Client::connect_with_config(obs_config).await?;

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
                obs_actor.clone(),
            )
            .await;

            // Spawn OBS task.
            tasks.spawn(run_obs(settings, layouts.clone(), obs, obs_rx));

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
                    Some(Ok(Ok(()))) => log::info!("Service Done"),
                    Some(Ok(Err(err))) => log::error!("Service error: {}", err.to_string()),
                    Some(Err(err)) => log::error!("Failed to join tasks: {}", err.to_string()),
                    None => break Ok(()),
                }
            }
        }
    }
}
