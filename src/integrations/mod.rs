use std::sync::Arc;

use tokio::task::JoinSet;

use crate::{db::ProjectDb, obs::ObsActor, settings::Settings, stream::StreamActor};

use self::{
    ladder_league::{run_ladder_league, LadderLeagueActor},
    therun::TheRunActor,
};

pub mod discord;
pub mod ladder_league;
pub mod therun;
pub mod web;

pub async fn init_integrations(
    tasks: &mut JoinSet<Result<(), anyhow::Error>>,
    settings: Arc<Settings>,
    db: Arc<ProjectDb>,
    state_actor: StreamActor,
    obs_actor: ObsActor,
) {
    let mut therun_actor: Option<TheRunActor> = None;

    // Add TheRun.gg module to tasks
    /*
    if project.integrations.contains(&Integration::TheRun) {
        let (therun_actor_2, rx) = TheRunActor::new();

        tasks.spawn(run_therun(project_store.clone(), state_actor.clone(), rx));

        therun_actor = Some(therun_actor_2);
    }*/

    let (ladder_actor, rx) = LadderLeagueActor::new();
    tasks.spawn(run_ladder_league(
        rx,
        db.clone(),
        state_actor.clone(),
        obs_actor.clone(),
    ));

    // Add discord integration to tasks
    tasks.spawn(discord::init_discord(
        settings.clone(),
        db.clone(),
        state_actor.clone(),
        obs_actor.clone(),
        ladder_actor.clone(),
    ));

    // Add webserver to tasks
    tasks.spawn(web::run_http_server(db));
}
