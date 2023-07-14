use std::collections::HashMap;

use obws::requests::{scene_items::{SetTransform, SceneItemTransform, Position, Scale, SetEnabled}, inputs::SetSettings};
use serde::{Deserialize, Serialize};

use crate::{state::ProjectState, error::Error};

#[derive(Debug, Serialize, Deserialize)]
pub struct LayoutFile {
    pub scene: String,
    pub layouts: HashMap<String, Layout>,
}

#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub struct Layout {
    pub name: String,
    pub default: Option<bool>,
    pub displays: Vec<UserDisplay>,
}

#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub struct UserDisplay {
    pub stream: PosSize,
    pub name: PosSize,
    pub extra_elements: Option<HashMap<String, PosSize>>,
    pub description: Option<PosSize>,
}

#[derive(PartialEq, Eq, Debug, Copy, Clone, Serialize, Deserialize)]
pub struct PosSize {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
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
    state: &ProjectState<'a>,
    obs_cfg: &LayoutFile,
    obs: &obws::Client,
) -> Result<(), Error> {
    let items = obs.scene_items().list(&obs_cfg.scene).await?;

    match state 
        .layouts_by_count
        .get(&state.active_players.len().try_into().unwrap())
        .map(|l| *l)
        .or_else(|| {
            obs_cfg.layouts.values().find(|l| {
                l.default.unwrap_or(false) && l.displays.len() == state.active_players.len()
            })
        })
        .or_else(|| {
            obs_cfg
                .layouts
                .values()
                .find(|l| l.displays.len() == state.active_players.len())
        }) {
        Some(layout) => {
            for item in &items {
                if item.source_name.starts_with("stream_")
                    || item.source_name.starts_with("name_")
                    || obs_cfg.layouts.contains_key(&item.source_name)
                {
                    obs.scene_items()
                        .set_enabled(SetEnabled {
                            scene: &obs_cfg.scene,
                            item_id: item.id,
                            enabled: false,
                        })
                        .await?;
                }
            }

            for (idx, view) in layout.displays.iter().enumerate() {
                let player = state.active_players[idx];

                let stream_id = "stream_".to_string() + &idx.to_string();
                let stream = items
                    .iter()
                    .find(|item| item.source_name == stream_id)
                    .ok_or(Error::GeneralError(format!("Unknown source {}", stream_id)))?;

                let name_id = "name_".to_string() + &idx.to_string();
                let name = items
                    .iter()
                    .find(|item| item.source_name == name_id)
                    .ok_or(Error::GeneralError(format!("Unknown source {}", stream_id)))?;

                let stream_transform = SetTransform {
                    scene: &obs_cfg.scene,
                    item_id: stream.id,
                    transform: SceneItemTransform {
                        position: Some(Position {
                            x: Some(view.stream.x as f32),
                            y: Some(view.stream.y as f32),
                        }),
                        rotation: None,
                        scale: Some(Scale {
                            x: Some(1.0),
                            y: Some(1.0),
                        }),
                        alignment: None,
                        bounds: Some(obws::requests::scene_items::Bounds {
                            r#type: Some(obws::common::BoundsType::MaxOnly),
                            alignment: None,
                            width: Some(view.stream.width as f32),
                            height: Some(view.stream.width as f32),
                        }),
                        crop: None,
                    },
                };

                let vlc_setting = VLC {
                    playlist: vec![PlaylistItem {
                        hidden: false,
                        selected: false,
                        value: &state.streams.get(&player).unwrap(),
                    }],
                };

                obs.scene_items().set_transform(stream_transform).await?;
                obs.scene_items()
                    .set_enabled(SetEnabled {
                        scene: &obs_cfg.scene,
                        item_id: stream.id,
                        enabled: true,
                    })
                    .await?;
                obs.inputs()
                    .set_settings(SetSettings {
                        input: &stream_id,
                        settings: &vlc_setting,
                        overlay: Some(true),
                    })
                    .await?;

                let name_transform = SetTransform {
                    scene: &obs_cfg.scene,
                    item_id: name.id,
                    transform: SceneItemTransform {
                        position: Some(Position {
                            x: Some(view.name.x as f32),
                            y: Some(view.name.y as f32),
                        }),
                        rotation: None,
                        scale: Some(Scale {
                            x: Some(1.0),
                            y: Some(1.0),
                        }),
                        alignment: None,
                        bounds: Some(obws::requests::scene_items::Bounds {
                            r#type: Some(obws::common::BoundsType::MaxOnly),
                            alignment: None,
                            width: Some(view.name.width as f32),
                            height: Some(view.name.width as f32),
                        }),
                        crop: None,
                    },
                };

                let name_setting = SpecificFreetype { text: &player.name };
                obs.scene_items().set_transform(name_transform).await?;
                obs.scene_items()
                    .set_enabled(SetEnabled {
                        scene: &obs_cfg.scene,
                        item_id: name.id,
                        enabled: true,
                    })
                    .await?;
                obs.inputs()
                    .set_settings(SetSettings {
                        input: &name_id,
                        settings: &name_setting,
                        overlay: Some(true),
                    })
                    .await?;
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
