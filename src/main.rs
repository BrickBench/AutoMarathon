use std::{
    env::consts,
    fs::{self, read_to_string},
    path::PathBuf,
    sync::Arc,
};

use anyhow::anyhow;
use clap::{Parser, Subcommand};

use tokio::{
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        oneshot,
    },
    task::JoinSet,
};

use crate::{
    core::db::ProjectDb,
    integrations::discord::test_discord,
    integrations::obs::{run_obs, ObsActor},
    core::settings::Settings,
    core::stream::{run_state_manager, StreamActor},
};

mod error;
mod integrations;
mod core;

const AUTOMARATHON_VER: &str = "0.1";

#[macro_export]
macro_rules! send_message {
    ($actor: expr, $type: ident, $msg: ident, $($vals: expr),*) => {
        {
            let (tx, rx) = Rto::new();
            $actor.send($type::$msg($($vals),*, tx));
            match rx.await {
                Ok(val) => val,
                Err(e) => Err(e.into())
            }
        }
    };
}

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
    Create { project_folder: PathBuf },

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
        RunType::Create { project_folder } => {
            fs::create_dir_all(project_folder)?;
            ProjectDb::init(&project_folder.join("project.db")).await?;
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
            let db = Arc::new(ProjectDb::load(&project_folder.join("project.db")).await?);

            // Load settings
            let settings: Arc<Settings> = Arc::new(
                serde_json::from_str::<Settings>(
                    &read_to_string(project_folder.join("settings.json")).map_err(|_| anyhow!(format!(
                        "Failed to load settings.json file, could not read from {}/settings.json",
                        project_folder.to_str().unwrap()
                    )))?,
                )
                .map_err(|e| anyhow!(format!("Error while loading settings.json: {:?}", e)))?,
            );

            log::info!("AutoMarathon initialized");

            let mut tasks = JoinSet::<Result<(), anyhow::Error>>::new();

            // Set up messaging channels
            let (state_actor, state_rx) = StreamActor::new();
            let (obs_actor, obs_rx) = ObsActor::new();

            // Spawn optional integrations
            integrations::init_integrations(
                &mut tasks,
                settings.clone(),
                db.clone(),
                state_actor.clone(),
                obs_actor.clone(),
            )
            .await;

            // Spawn OBS task.
            tasks.spawn(run_obs(settings, db.clone(), obs_rx));

            // Spawn main task.
            tasks.spawn(run_state_manager(db, state_rx, obs_actor.clone()));

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
