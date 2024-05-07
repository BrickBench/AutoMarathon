use std::{sync::Arc, time::Duration};

use obws::requests::{
    inputs::SetSettings,
    scene_items::{
        Bounds, Duplicate, Position, SceneItemTransform, SetEnabled, SetIndex, SetTransform,
    },
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    error::Error,
    project::Project,
    settings::Settings,
    state::{ModifiedState, ProjectState},
    Rto, ActorRef,
};

/// Json struct for OBS layout settings
#[derive(Debug, Serialize, Deserialize)]
pub struct LayoutFile {
    pub scene: String,
    pub stream_name: Option<String>,
    pub player_name: Option<String>,
    pub layouts: Vec<ObsLayout>,
}

/// Individual layout settings
#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub struct ObsLayout {
    pub name: String,
    /// Whether this layout should be the default layout for the provided player count
    pub default: Option<bool>,
    pub players: usize,
}

// OBS FreeType settings parameters
#[derive(Serialize)]
struct SpecificFreetype<'a> {
    text: &'a str,
}

/// OBS VLC source parameters
#[allow(clippy::upper_case_acronyms)]
#[derive(Serialize)]
struct VLC<'a> {
    playlist: Vec<PlaylistItem<'a>>,
}

/// OBS PlaylistItem parameters
#[derive(Serialize)]
struct PlaylistItem<'a> {
    hidden: bool,
    selected: bool,
    value: &'a str,
}

impl LayoutFile {
    /// Return a layout in this layout file by name
    pub fn get_layout_by_name(&self, name: &str) -> Result<&ObsLayout, anyhow::Error> {
        for layout in &self.layouts {
            if layout.name == name {
                return Ok(layout);
            }
        }

        Err(Error::UnknownLayout(name.to_owned()))?
    }
}

/// Return the appropriate layout for the given project state
fn get_layout<'a>(state: &ProjectState, layouts: &'a LayoutFile) -> Option<&'a ObsLayout> {
    state
        .layouts_by_count
        .get(&state.active_players.len().try_into().unwrap()) // Use previous layout
        .and_then(|l| layouts.get_layout_by_name(l).ok())
        .or_else(|| {
            layouts
                .layouts
                .iter()
                .find(|l| l.default.unwrap_or(false) && l.players == state.active_players.len())
        }) // Use the default layout for the player count
        .or_else(|| {
            layouts
                .layouts
                .iter()
                .find(|l| l.players == state.active_players.len())
        }) // Use any layout for the player count
}

/// Requests for ObsActor
pub enum ObsCommand {
    UpdateObs(ProjectState, Vec<ModifiedState>, Rto<()>),
}

pub type ObsActor = ActorRef<ObsCommand>;

pub async fn run_obs(
    project: Arc<Project>,
    settings: Arc<Settings>,
    layouts: Arc<LayoutFile>,
    obs: obws::Client,
    mut rx: UnboundedReceiver<ObsCommand>,
) -> Result<(), anyhow::Error> {
    loop {
        match rx.recv().await.unwrap() {
            ObsCommand::UpdateObs(state, modifications, rto) => rto
                .reply(update_obs(&project, &state, &settings, &modifications, &layouts, &obs).await),
        };
    }
}

/// Apply project state to OBS
pub async fn update_obs(
    project: &Project,
    state: &ProjectState,
    settings: &Settings,
    modifications: &[ModifiedState],
    layouts: &LayoutFile,
    obs: &obws::Client,
) -> Result<(), anyhow::Error> {
    log::debug!("Updating OBS: {:?}", modifications);
    let items = obs.scene_items().list(&layouts.scene).await?;

    match get_layout(state, layouts) {
        Some(layout) => {
            // Reset
            if modifications.contains(&ModifiedState::Layout) {
                log::debug!("Activating new layout");
                for item in &items {
                    if let Ok(item_layout) = layouts.get_layout_by_name(&item.source_name) {
                        obs.scene_items()
                            .set_enabled(SetEnabled {
                                scene: &layouts.scene,
                                item_id: item.id,
                                enabled: layout == item_layout,
                            })
                            .await?;
                    }
                }
            }

            // Modify commentary text
            if modifications.contains(&ModifiedState::Commentary) {
                log::debug!("Updating commentator list");
                let comm_setting = SpecificFreetype {
                    text: &state.get_commentators().join(" "),
                };
                obs.inputs()
                    .set_settings(SetSettings {
                        input: "commentary",
                        settings: &comm_setting,
                        overlay: Some(true),
                    })
                    .await?;
            }

            // Set stream mute status
            log::debug!("Muting {} streams", project.players.len());
            for idx in 0..project.players.len() {
                obs.inputs()
                    .set_muted(&format!("streamer_{}", idx), true)
                    .await?;
            }

            // Remove streamer views for changed or invisible runners
            log::debug!("Removing leftover streams");
            for player in &project.players {
                if (state.active_players.contains(&player.name)
                    && modifications.contains(&ModifiedState::PlayerView(player.name.to_string())))
                    || !state.active_players.contains(&player.name)
                    || modifications.contains(&ModifiedState::Layout)
                {
                    for item in &items {
                        if item.source_name
                            == format!(
                                "streamer_{}",
                                &project.players.iter().position(|p| p == player).unwrap()
                            )
                            && !obs.scene_items().locked(&layouts.scene, item.id).await?
                        {
                            let _ = obs.scene_items().remove(&layouts.scene, item.id).await;
                        }
                    }
                }
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
            // Refresh items after deletions
            let items = obs.scene_items().list(&layouts.scene).await?;
            let group = obs.scene_items().list_group(&layout.name).await?;

            for idx in 0..layout.players {
                let player = &state.active_players[idx];

                let stream_id = format!(
                    "streamer_{}",
                    project
                        .players
                        .iter()
                        .position(|p| player == &p.name)
                        .unwrap()
                );

                if idx == 0 {
                    obs.inputs().set_muted(&stream_id, false).await?;
                }

                // Update this player's stream
                if modifications.contains(&ModifiedState::PlayerStream(player.to_string())) {
                    log::debug!("Applying stream change to {}", player);
                    let vlc_setting = VLC {
                        playlist: vec![PlaylistItem {
                            hidden: false,
                            selected: false,
                            value: state.streams.get(player).unwrap(),
                        }],
                    };
                    obs.inputs()
                        .set_settings(SetSettings {
                            input: &stream_id,
                            settings: &vlc_setting,
                            overlay: Some(true),
                        })
                        .await?;

                    tokio::time::sleep(Duration::from_millis(200)).await;
                }

                // Update this player's view
                if modifications.contains(&ModifiedState::PlayerView(player.to_string()))
                    || modifications.contains(&ModifiedState::Layout)
                {
                    log::debug!("Applying position change to {}", player);
                    let name_id = format!("name_{}", idx);
                    let name_setting = SpecificFreetype {
                        text: &player.to_uppercase(),
                    };
                    obs.inputs()
                        .set_settings(SetSettings {
                            input: &name_id,
                            settings: &name_setting,
                            overlay: Some(true),
                        })
                        .await?;

                    let stream = items
                        .iter()
                        .find(|item| item.source_name == stream_id)
                        .ok_or(Error::Unknown(format!("Unknown source {}", stream_id)))?;

                    let stream_views: Vec<&obws::responses::scene_items::SceneItem> = group
                        .iter()
                        .filter(|item| {
                            item.source_name
                                .starts_with(&format!("{}_stream_{}", &layout.name, idx))
                        })
                        .collect();

                    // Create a VLC source for each identified stream view
                    for view in stream_views {
                        let new_item = obs
                            .scene_items()
                            .duplicate(Duplicate {
                                scene: &layouts.scene,
                                item_id: stream.id,
                                destination: None,
                            })
                            .await?;

                        tokio::time::sleep(Duration::from_millis(200)).await;
                        obs.scene_items()
                            .set_enabled(SetEnabled {
                                scene: &layouts.scene,
                                item_id: new_item,
                                enabled: true,
                            })
                            .await?;

                        obs.scene_items()
                            .set_index(SetIndex {
                                scene: &layouts.scene,
                                item_id: new_item,
                                index: 0,
                            })
                            .await?;

                        let existing_transform =
                            obs.scene_items().transform(&layout.name, view.id).await?;

                        let new_transform = SetTransform {
                            scene: &layouts.scene,
                            item_id: new_item,
                            transform: SceneItemTransform {
                                position: Some(Position {
                                    x: Some(existing_transform.position_x),
                                    y: Some(existing_transform.position_y),
                                }),
                                rotation: None,
                                scale: None,
                                alignment: Some(existing_transform.alignment),
                                bounds: Some(Bounds {
                                    r#type: Some(obws::common::BoundsType::Stretch),
                                    alignment: Some(existing_transform.bounds_alignment),
                                    width: Some(existing_transform.bounds_width),
                                    height: Some(existing_transform.bounds_height),
                                }),
                                crop: Some(obws::requests::scene_items::Crop {
                                    left: Some(existing_transform.crop_left),
                                    right: Some(existing_transform.crop_right),
                                    top: Some(existing_transform.crop_top),
                                    bottom: Some(existing_transform.crop_bottom),
                                }),
                            },
                        };
                        obs.scene_items().set_transform(new_transform).await?;
                    }
                }
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
            if obs.ui().studio_mode_enabled().await? {
                log::debug!("Triggering Studio Mode transition");
                match &settings.obs_transition {
                    Some(name) => {
                        obs.transitions().set_current(name).await?;
                        obs.transitions().trigger().await?;
                    }
                    None => obs.transitions().trigger().await?,
                }
            }

            log::debug!("OBS update complete");

            Ok(())
        }
        _ => Err(Error::UnknownLayout(
            "No known layout for the current player count.".to_string(),
        ))?,
    }
}
