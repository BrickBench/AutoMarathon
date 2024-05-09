use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    obs::ObsActor, player::Player, state::StateActor, ActorRef, Rto,
};

#[derive(PartialEq, Debug, Serialize, Deserialize, Clone)]
struct LadderLeaguePlayer {
    name: String,
    twitch: String,
}

/// Requests for ObsActor
pub enum LadderLeagueCommand {
    ActivateRace(String, Rto<()>),
}

pub type LadderLeagueActor = ActorRef<LadderLeagueCommand>;

pub async fn run_ladder_league(
    mut rx: UnboundedReceiver<LadderLeagueCommand>,
    state_actor: StateActor,
    obs_actor: ObsActor,
) -> Result<(), anyhow::Error> {
    loop {
        match rx.recv().await.unwrap() {
            LadderLeagueCommand::ActivateRace(race_id, rto) => {
                let request_url = format!(
                    "https://whatever.com/runnerstwitch?spreadsheet_id={}",
                    race_id
                );

                let response = reqwest::get(&request_url).await?;
                let users: Vec<Player> = response.json().await?;

                let (rtx, rrx) = Rto::new();
                let (rtx2, rrx2) = Rto::new();
                let (rtx3, rrx3) = Rto::new();

                obs_actor.send(crate::obs::ObsCommand::SetLadderLeagueId(race_id, rtx));
                state_actor.send(crate::state::StateRequest::SetRunners(users, rtx2));
                let _ = rrx2.await;

                state_actor.send(crate::state::StateRequest::UpdateState(crate::state::StateCommand::Reset, rtx3));
                let _ = rrx.await;
                let _ = rrx3.await;
                rto.reply(Ok(()))
            }
        };
    }
}
