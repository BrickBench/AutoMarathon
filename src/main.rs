use core::{
    event::{run_event_actor, EventActor},
    runner::{run_runner_actor, RunnerActor},
};
use std::{env::consts, fs::read_to_string, path::PathBuf, sync::Arc};

use anyhow::anyhow;
use clap::Parser;

use integrations::web::{run_http_server, WebActor};
use tokio::{
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        oneshot,
    },
    task::JoinSet,
};

use crate::{
    core::db::ProjectDb,
    core::settings::Settings,
    core::stream::{run_stream_manager, StreamActor},
    integrations::obs::{run_obs, ObsActor},
};

mod core;
mod error;
mod integrations;

const AUTOMARATHON_VER: &str = "0.1";

#[macro_export]
macro_rules! send_message {
    ($actor: expr, $type: ident, $msg: ident, $($vals: expr),*) => {
        {
            let (tx, rx) = Rto::new();
            if log::log_enabled!(log::Level::Debug) {
                let mut log_str =
                        format!("Sending message {}::{} to actor {} with contents:",
                        stringify!($type), stringify!($msg), stringify!($actor));
                $(log_str.push_str(&format!(" `{:?}`", &$vals));)*
                log::debug!("{}", log_str);
            }

            $actor.send($type::$msg($($vals),*, tx));
            match rx.await {
                Ok(val) => val,
                Err(e) => Err(e.into())
            }
        }
    };
    ($actor: expr, $type: ident, $msg: ident) => {
        {
            let (tx, rx) = Rto::new();
            if log::log_enabled!(log::Level::Debug) {
                log::debug!("Sending message {}::{} to actor {}",
                        stringify!($type), stringify!($msg), stringify!($actor));
            }

            $actor.send($type::$msg(tx));
            match rx.await {
                Ok(val) => val,
                Err(e) => Err(e.into())
            }
        }
    };
}

#[macro_export]
macro_rules! send_nonblocking {
    ($actor: expr, $type: ident, $msg: ident, $($vals: expr),*) => {
        {
            let (tx, rx) = Rto::new();
            $actor.send($type::$msg($($vals),*, tx));
            rx
        }
    };
}

/// Directory containing all actors
#[derive(Clone)]
pub struct Directory {
    pub stream_actor: StreamActor,
    pub obs_actor: ObsActor,
    pub runner_actor: RunnerActor,
    pub event_actor: EventActor,
    pub web_actor: WebActor,
}

/// Actor reference
pub struct ActorRef<T> {
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
pub struct Rto<T> {
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
    /// The folder containing the project files.
    project_folder: PathBuf,
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
        .filter(Some("sqlx"), log::LevelFilter::Info)
        .filter_level(log::LevelFilter::Debug)
        .init();

    log::info!(
        "Launching AutoMarathon {} on {}",
        AUTOMARATHON_VER,
        consts::OS
    );

    if !args.project_folder.exists() {
        return Err(anyhow!(
            "Project folder {} does not exist",
            args.project_folder.to_str().unwrap()
        ));
    }

    // Set up messaging channels
    let (state_actor, state_rx) = StreamActor::new();
    let (obs_actor, obs_rx) = ObsActor::new();
    let (runner_actor, runner_rx) = RunnerActor::new();
    let (event_actor, event_rx) = EventActor::new();
    let (web_actor, web_rx) = WebActor::new();

    let directory = Directory {
        stream_actor: state_actor.clone(),
        obs_actor: obs_actor.clone(),
        runner_actor: runner_actor.clone(),
        event_actor: event_actor.clone(),
        web_actor: web_actor.clone(),
    };

    let db = Arc::new(
        ProjectDb::load(&args.project_folder.join("project.db"), directory.clone()).await?,
    );

    // Load settings
    let settings: Arc<Settings> = Arc::new(
        serde_json::from_str::<Settings>(
            &read_to_string(args.project_folder.join("settings.json")).map_err(|_| {
                anyhow!(format!(
                    "Failed to load settings.json file, could not read from {}/settings.json",
                    args.project_folder.to_str().unwrap()
                ))
            })?,
        )
        .map_err(|e| anyhow!(format!("Error while loading settings.json: {:?}", e)))?,
    );

    let mut tasks = JoinSet::<Result<(), anyhow::Error>>::new();

    // Spawn core tasks
    tasks.spawn(run_obs(settings.clone(), db.clone(), obs_rx));
    tasks.spawn(run_stream_manager(db.clone(), state_rx, directory.clone()));
    tasks.spawn(run_event_actor(db.clone(), event_rx, directory.clone()));
    tasks.spawn(run_http_server(db.clone(), directory.clone(), web_rx));
    tasks.spawn(run_runner_actor(db.clone(), runner_rx));

    // Spawn integrations
    if settings.discord_token.is_some() {
        tasks.spawn(integrations::discord::init_discord(
            settings.clone(),
            db.clone(),
            directory.clone(),
        ));
    }

    log::info!("AutoMarathon initialized");

    loop {
        match tasks.join_next().await {
            Some(Ok(Ok(()))) => log::info!("Service Done"),
            Some(Ok(Err(err))) => log::error!("Service error: {}", err.to_string()),
            Some(Err(err)) => log::error!("Failed to join tasks: {}", err.to_string()),
            None => break Ok(()),
        }
    }
}
