use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    db::ProjectDb,
    event::Event,
    obs::{ObsActor, ObsCommand},
    runner::Runner,
    send_message,
    stream::{StreamActor, StreamCommand, StreamRequest},
    ActorRef, Rto,
};

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
            LadderLeagueCommand::ActivateRace(event_id, race_id, obs_host, rto) => rto.reply(
                activate_ladder_league_id(
                    &event_id,
                    &race_id,
                    &obs_host,
                    &db,
                    &state_actor,
                    &obs_actor,
                )
                .await
                .inspect_err(|e| println!("{:?}", e)),
            ),
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

    let response = reqwest::get(&request_url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to query ladder league players: {:?}", e))?;

    let users: Vec<LadderLeaguePlayer> = response
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to parse ladder league players: {:?}", e))?;

    log::debug!("Got users: {:?}", users);

    let existing_users = db.get_runners().await?;
    let users: Vec<_> = users
        .iter()
        .map(|p| Runner {
            name: p.name.clone(),
            stream: Some(p.twitch.clone()),
            therun: None,
            cached_stream_url: None,
            volume_percent: 50,
        })
        .collect();

    // Add new runners
    for runner in &users
        .iter()
        .filter(|r| !existing_users.iter().any(|r2| r.name == r2.name))
        .collect::<Vec<_>>()
    {
        db.add_runner(&runner, &[]).await?;
    }

    if !db
        .get_event_names()
        .await?
        .iter()
        .any(|e| e.eq_ignore_ascii_case(event_id))
    {
        db.add_event(&Event {
            name: event_id.to_string(),
            therun_race_id: None,
            start_time: None,
            end_time: None,
            is_relay: false,
            is_marathon: false,
        })
        .await?;

        for runner in &users {
            db.add_runner_to_event(event_id, &runner.name).await?;
        }
    }

    if !db
        .get_streamed_events()
        .await?
        .iter()
        .any(|e| e.eq_ignore_ascii_case(event_id))
    {
        send_message!(
            state_actor,
            StreamRequest,
            CreateStream,
            event_id.to_owned(),
            obs_host.to_owned()
        )?;
    }

    send_message!(
        state_actor,
        StreamRequest,
        UpdateStream,
        Some(event_id.to_owned()),
        StreamCommand::Layout("pregame".to_owned())
    );

    send_message!(
        state_actor,
        StreamRequest,
        UpdateStream,
        Some(event_id.to_owned()),
        StreamCommand::SetPlayers(vec![])
    );

    send_message!(
        obs_actor,
        ObsCommand,
        SetLadderLeagueId,
        event_id.to_owned(),
        race_id.to_owned()
    )?;
    Ok(())
}
