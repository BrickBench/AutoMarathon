use std::sync::Arc;

use tokio::{sync::mpsc::UnboundedSender, task::JoinSet};

use crate::{
    error::Error,
    obs::LayoutFile,
    settings::Settings,
    state::{self, Project},
    CommandMessage,
};

pub mod discord;
pub mod therun;
pub mod web;

pub fn init_integrations(
    tasks: &mut JoinSet<Result<(), Error>>,
    command_sender: UnboundedSender<CommandMessage>,
    settings: Arc<Settings>,
    layouts: Arc<LayoutFile>,
    project: Arc<Project>,
) {
    // Add TheRun.gg module to tasks
    if project.integrations.contains(&state::Integration::TheRun) {
        for player in &project.players {
            let therun = player
                .therun
                .to_owned()
                .unwrap_or(player.stream.split('/').last().unwrap().to_string());
            tasks.spawn(therun::create_player_websocket_monitor(
                player.name.clone(),
                therun.to_string(),
                command_sender.clone(),
            ));
        }
    }

    // Add discord integration to tasks
    if project.integrations.contains(&state::Integration::Discord) {
        tasks.spawn(discord::init_discord(
            command_sender.clone(),
            settings.clone(),
            project.clone(),
            layouts.clone(),
        ));
    }

    // Add webserver to tasks
    tasks.spawn(web::run_http_server(command_sender.clone()));
}
