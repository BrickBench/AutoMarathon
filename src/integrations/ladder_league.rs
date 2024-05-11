use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{db::ProjectDb, obs::ObsActor, runner::Runner, event::Event, stream::StreamActor, ActorRef, Rto};

#[derive(PartialEq, Debug, Serialize, Deserialize, Clone)]
struct LadderLeaguePlayer {
    name: String,
    twitch: String,
}

/// Requests for ObsActor
pub enum LadderLeagueCommand {
    ActivateRace(String, String, String, Rto<()>),
}

pub type LadderLeagueActor = ActorRef<LadderLeagueCommand>;

pub async fn run_ladder_league(
    mut rx: UnboundedReceiver<LadderLeagueCommand>,
    db: Arc<ProjectDb>,
    state_actor: StreamActor,
    obs_actor: ObsActor,
) -> Result<(), anyhow::Error> {
    loop {
        match rx.recv().await.unwrap() {
            LadderLeagueCommand::ActivateRace(event_id, race_id, obs_host, rto) => {
                rto.reply(activate_ladder_league_id(&event_id, &race_id, &obs_host, &db, &state_actor, &obs_actor).await)
            }
        };
    }
}

pub async fn activate_ladder_league_id(
    event_id: &str,
    race_id: &str,
    obs_host: &str,
    db: &ProjectDb,
    state_actor: &StreamActor,
    obs_actor: &ObsActor,
) -> anyhow::Result<()> {
    log::info!("Activating race {}", race_id);
    let request_url = format!(
        "http://localhost:35065/runnerstwitch?spreadsheet_id={}",
        race_id
    );

    let response = reqwest::get(&request_url).await?;
    let users: Vec<LadderLeaguePlayer> = response.json().await?;

    let good_users: Vec<_> = users
        .iter()
        .map(|p| Runner {
            name: p.name.clone(),
            stream: Some(p.twitch.clone()),
            therun: None,
            cached_stream_url: None,
        })
        .collect();

    log::debug!("Got updated users: {:?}", users);

    db.add_event(
        &Event {
            name: event_id.to_string(),
            therun_race_id: None, 
            start_time: None,
            end_time: None,
            is_relay: false,
            is_marathon: false,
        }
    ).await?;

    for runner in &good_users {
        db.add_runner(&runner, &[]).await?;
        db.add_runner_to_event(event_id, &runner.name).await?;
    }
       

    let (rtx, rrx) = Rto::new();
    let (rtx2, rrx2) = Rto::new();

    obs_actor.send(crate::obs::ObsCommand::SetLadderLeagueId(
        event_id.to_owned(),
        race_id.to_owned(),
        rtx,
    ));

    state_actor.send(crate::stream::StreamRequest::CreateStream(
        event_id.to_owned(),
        obs_host.to_owned(),
        rtx2,
    ));
    let _ = rrx.await;
    let _ = rrx2.await;

    Ok(())
}
