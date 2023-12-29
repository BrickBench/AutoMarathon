use std::sync::Arc;

use tokio::task::JoinSet;

use crate::{
    obs::LayoutFile,
    project::{Integration, Project},
    settings::Settings,
    state::StateActor,
};

use self::therun::{run_therun, TheRunActor};

pub mod discord;
pub mod therun;
pub mod web;

pub fn init_integrations(
    tasks: &mut JoinSet<Result<(), anyhow::Error>>,
    settings: Arc<Settings>,
    layouts: Arc<LayoutFile>,
    project: Arc<Project>,
    state_actor: StateActor,
) {
    let mut therun_actor: Option<TheRunActor> = None;

    // Add TheRun.gg module to tasks
    if project.integrations.contains(&Integration::TheRun) {
        let (therun_actor_2, rx) = TheRunActor::new();

        tasks.spawn(run_therun(project.clone(), state_actor.clone(), rx));

        therun_actor = Some(therun_actor_2);
    }

    // Add discord integration to tasks
    if project.integrations.contains(&Integration::Discord) {
        tasks.spawn(discord::init_discord(
            settings.clone(),
            project.clone(),
            layouts.clone(),
            state_actor.clone(),
        ));
    }

    // Add webserver to tasks
    tasks.spawn(web::run_http_server(project, state_actor, therun_actor));
}
