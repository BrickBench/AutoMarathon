use obws::requests::{
    inputs::SetSettings,
    scene_items::{
        Bounds, Duplicate, Position, SceneItemTransform, SetEnabled, SetTransform,
    },
};
use serde::{Deserialize, Serialize};

use crate::{error::Error, state::{ProjectState, Project, ModifiedState}};

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

                if item.source_name.starts_with("stream_") && !obs.scene_items().locked(&obs_cfg.scene, item.id).await? {
                    obs.scene_items().remove(&obs_cfg.scene, item.id).await?;
                }
            }

            let group = obs.scene_items().list_group(&layout.name).await?;
            for idx in 0..layout.players {
                let player = state.active_players[idx];

                let stream_id = format!("stream_{}", project.players.iter().position(|p| player == p).unwrap());

                let name_id = format!("name_{}", idx);


                if modifications.contains(&ModifiedState::PlayerStream(player)) {
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
                }

                let name_setting = SpecificFreetype { text: &player.name };
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

                let stream_views = group.iter().filter(|item| {
                    item.source_name
                        .starts_with(&format!("{}_stream_{}", &layout.name, idx))
                });

                for view in stream_views {
                    let dup_settings = Duplicate {
                        scene: &obs_cfg.scene,
                        item_id: stream.id,
                        destination: None,
                    };
                    let new_item = obs.scene_items().duplicate(dup_settings).await?;
                    obs.scene_items().set_enabled(SetEnabled {
                        scene: &obs_cfg.scene,
                        item_id: new_item,
                        enabled: true,
                    }).await?;

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

            if obs.ui().studio_mode_enabled().await? {
                obs.transitions().trigger().await?;
            }

            Ok(())
        }
        _ => Err(Error::LayoutError(
            "No known layout for the current player count.".to_string(),
        )),
    }
}
