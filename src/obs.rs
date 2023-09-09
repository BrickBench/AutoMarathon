use std::time::Duration;

use obws::requests::{
    inputs::SetSettings,
    scene_items::{
        Bounds, Duplicate, Position, SceneItemTransform, SetEnabled, SetIndex, SetTransform,
    },
};
use serde::{Deserialize, Serialize};

use crate::{
    error::Error,
    state::{ModifiedState, Project, ProjectState}, settings::Settings,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct LayoutFile {
    pub scene: String,
    pub stream_name: Option<String>,
    pub player_name: Option<String>,
    pub layouts: Vec<Layout>,
}

#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub struct Layout {
    pub name: String,
    pub default: Option<bool>,
    pub players: usize,
}

#[derive(Serialize)]
struct SpecificFreetype<'a> {
    text: &'a str,
}

#[derive(Serialize)]
struct VLC<'a> {
    playlist: Vec<PlaylistItem<'a>>,
}

#[derive(Serialize)]
struct PlaylistItem<'a> {
    hidden: bool,
    selected: bool,
    value: &'a str,
}

pub async fn update_obs<'a>(
    project: &Project,
    state: &ProjectState<'a>,
    settings: &Settings,
    modifications: Vec<ModifiedState<'a>>,
    obs_cfg: &LayoutFile,
    obs: &obws::Client,
) -> Result<(), Error> {
    let items = obs.scene_items().list(&obs_cfg.scene).await?;

    match state
        .layouts_by_count
        .get(&state.active_players.len().try_into().unwrap())
        .map(|l| *l)
        .or_else(|| {
            obs_cfg
                .layouts
                .iter()
                .find(|l| l.default.unwrap_or(false) && l.players == state.active_players.len())
        })
        .or_else(|| {
            obs_cfg
                .layouts
                .iter()
                .find(|l| l.players == state.active_players.len())
        }) {
        Some(layout) => {

            // Reset
            if modifications.contains(&ModifiedState::Layout) {
                for item in &items {
                    if let Some(item_layout) = obs_cfg
                        .layouts
                        .iter()
                        .find(|l| &l.name == &item.source_name)
                    {
                        obs.scene_items()
                            .set_enabled(SetEnabled {
                                scene: &obs_cfg.scene,
                                item_id: item.id,
                                enabled: layout == item_layout,
                            })
                            .await?;
                    }
                }
            }

            // Modify commentary text
            if modifications.contains(&ModifiedState::Commentary) {
                let mut commentators = state.active_commentators.clone();
                commentators.retain(|c| !&state.ignored_commentators.contains(c));

                let comm_setting = SpecificFreetype {
                    text: &commentators.join("\n"),
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
            for idx in 0..project.players.len() {
                obs.inputs().set_muted(&format!("streamer_{}", idx), true).await?;
            } 

            // Remove streamer views for changed or invisible runners
            for player in &project.players {
                if (state.active_players.contains(&player)
                    && modifications.contains(&ModifiedState::PlayerView(player)))
                    || !state.active_players.contains(&player) 
                    || modifications.contains(&ModifiedState::Layout)
                {
                    for item in &items {
                        if item.source_name
                            == format!(
                                "streamer_{}",
                                &project.players.iter().position(|p| p == player).unwrap()
                            )
                            && !obs.scene_items().locked(&obs_cfg.scene, item.id).await?
                        {
                            let _ = obs.scene_items().remove(&obs_cfg.scene, item.id).await;
                        }
                    }
                }
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
            // Refresh items after deletions
            let items = obs.scene_items().list(&obs_cfg.scene).await?;
            let group = obs.scene_items().list_group(&layout.name).await?;

            for idx in 0..layout.players {
                let player = state.active_players[idx];

                let stream_id = format!(
                    "streamer_{}",
                    project.players.iter().position(|p| player == p).unwrap()
                );

                if idx == 0 {
                    obs.inputs().set_muted(&stream_id, false).await?;
                }

                // Update this player's stream
                if modifications.contains(&ModifiedState::PlayerStream(player)) {
                    log::debug!("Applying stream change to {}", player.name);
                    let vlc_setting = VLC {
                        playlist: vec![PlaylistItem {
                            hidden: false,
                            selected: false,
                            value: &state.streams.get(&player).unwrap(),
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
                if modifications.contains(&ModifiedState::PlayerView(player))
                    || modifications.contains(&ModifiedState::Layout)
                {
                    log::debug!("Applying position change to {}", player.name);
                    let name_id = format!("name_{}", idx);
                    let name_setting = SpecificFreetype { text: &player.name.to_uppercase() };
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
                        .ok_or(Error::GeneralError(format!("Unknown source {}", stream_id)))?;

                    let stream_views: Vec<&obws::responses::scene_items::SceneItem> = group.iter().filter(|item| {
                        item.source_name
                            .starts_with(&format!("{}_stream_{}", &layout.name, idx))
                    }).collect();

                    // Create a VLC source for each identified stream view
                    for view in stream_views {
                        let new_item = obs
                            .scene_items()
                            .duplicate(Duplicate {
                                scene: &obs_cfg.scene,
                                item_id: stream.id,
                                destination: None,
                            })
                            .await?;

                        tokio::time::sleep(Duration::from_millis(200)).await;
                        obs.scene_items()
                            .set_enabled(SetEnabled {
                                scene: &obs_cfg.scene,
                                item_id: new_item,
                                enabled: true,
                            })
                            .await?;

                        obs.scene_items()
                            .set_index(SetIndex {
                                scene: &obs_cfg.scene,
                                item_id: new_item,
                                index: 0,
                            })
                            .await?;

                        let existing_transform =
                            obs.scene_items().transform(&layout.name, view.id).await?;

                        let new_transform = SetTransform {
                            scene: &obs_cfg.scene,
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
                match &settings.obs_transition {
                    Some(name) => {
                        obs.transitions().set_current(name).await?;
                        obs.transitions().trigger().await?;
                    },
                    None => obs.transitions().trigger().await?,
                }
            }

            Ok(())
        }
        _ => Err(Error::LayoutError(
            "No known layout for the current player count.".to_string(),
        )),
    }
}
