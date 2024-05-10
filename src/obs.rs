use std::{sync::Arc, time::Duration};

use anyhow::anyhow;
use obws::{
    requests::{
        inputs::{self, InputId, SetSettings},
        scene_items::{
            Bounds, CreateSceneItem, Position, SceneItemTransform, SetIndex, SetTransform,
        },
        scenes::SceneId,
    },
    responses::scene_items::SceneItem,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    error::Error,
    settings::Settings,
    state::{ModifiedState, ProjectState},
    ActorRef, Rto,
};

/// Json struct for OBS layout settings
#[derive(Debug, Serialize, Deserialize)]
pub struct LayoutFile {
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

// OBS FreeType partial settings parameters
#[derive(Serialize)]
struct SpecificFreetype<'a> {
    text: &'a str,
}

/// OBS VLC partial source parameters
#[allow(clippy::upper_case_acronyms)]
#[derive(Serialize)]
struct VLC<'a> {
    playlist: Vec<PlaylistItem<'a>>,
}

/// OBS browser partial source parameters
#[derive(Serialize)]
struct Browser<'a> {
    url: &'a str,
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
    UpdateState(ProjectState, Vec<ModifiedState>, Rto<()>),
    SetLadderLeagueId(String, Rto<()>),
    StartStream(Rto<()>),
    EndStream(Rto<()>),
}

pub type ObsActor = ActorRef<ObsCommand>;

pub async fn run_obs(
    settings: Arc<Settings>,
    layouts: Arc<LayoutFile>,
    obs: obws::Client,
    mut rx: UnboundedReceiver<ObsCommand>,
) -> Result<(), anyhow::Error> {
    loop {
        match rx.recv().await.unwrap() {
            ObsCommand::UpdateState(state, modifications, rto) => {
                rto.reply(update_obs_state(&state, &settings, &modifications, &layouts, &obs).await)
            }
            ObsCommand::StartStream(rto) => {
                rto.reply(obs.streaming().start().await.map_err(|e| e.into()))
            }
            ObsCommand::EndStream(rto) => {
                rto.reply(obs.streaming().stop().await.map_err(|e| e.into()))
            }
            ObsCommand::SetLadderLeagueId(league, rto) => {
                rto.reply(update_ladder_league_urls(&obs, &league, &settings).await)
            }
        };
    }
}

pub async fn update_ladder_league_urls(
    obs: &obws::Client,
    league_id: &str,
    settings: &Settings,
) -> anyhow::Result<()> {
    log::debug!("Updating browser sources to use ID: {}", league_id);
    let host_prefix = "http://localhost:35065/";
    let site = format!("{}?spreadsheet_id={}", host_prefix, league_id);

    obs.inputs()
        .set_settings(SetSettings {
            input: InputId::Name("league_view"),
            settings: &Browser { url: &site },
            overlay: Some(true),
        })
        .await?;

    for i in 0..3 {
        let post_info = format!(
            "{}post_race_info?spreadsheet_id={}&runner={}",
            host_prefix, league_id, i
        );
        let pictures = format!(
            "{}pictures?spreadsheet_id={}&runner={}",
            host_prefix, league_id, i
        );
        let name = format!(
            "{}runnername?spreadsheet_id={}&runner={}",
            host_prefix, league_id, i
        );
        let overlay = format!(
            "{}runner_overlay?spreadsheet_id={}&runner={}",
            host_prefix, league_id, i
        );
        obs.inputs()
            .set_settings(SetSettings {
                input: InputId::Name(&format!("runner_name_{}", i + 1)),
                settings: &Browser { url: &name },
                overlay: Some(true),
            })
            .await?;
        obs.inputs()
            .set_settings(SetSettings {
                input: InputId::Name(&format!("post_info_{}", i + 1)),
                settings: &Browser { url: &post_info },
                overlay: Some(true),
            })
            .await?;
        obs.inputs()
            .set_settings(SetSettings {
                input: InputId::Name(&format!("pictures_{}", i + 1)),
                settings: &Browser { url: &pictures },
                overlay: Some(true),
            })
            .await?;
        obs.inputs()
            .set_settings(SetSettings {
                input: InputId::Name(&format!("overlay_{}", i + 1)),
                settings: &Browser { url: &overlay },
                overlay: Some(true),
            })
            .await?;
    }

    do_transition(obs, settings).await?;

    Ok(())
}

pub async fn delete_scene_items_for_player(
    obs: &obws::Client,
    layout: SceneId<'_>,
    scene_items: &[SceneItem],
    source_id_name: &str,
) -> anyhow::Result<()> {
    for old_item in scene_items {
        if old_item.source_name == source_id_name
            && obs.scene_items().enabled(layout, old_item.id).await?
        {
            obs.scene_items().remove(layout, old_item.id).await?;
        }
    }
    Ok(())
}

pub async fn do_transition(obs: &obws::Client, settings: &Settings) -> anyhow::Result<()> {
    log::debug!("Triggering Studio Mode transition");
    match &settings.obs_transition {
        Some(name) => {
            obs.transitions().set_current(name).await?;
            obs.transitions().trigger().await?;
        }
        None => obs.transitions().trigger().await?,
    }
    Ok(())
}

/// Apply project state to OBS
pub async fn update_obs_state(
    state: &ProjectState,
    settings: &Settings,
    modifications: &[ModifiedState],
    layouts: &LayoutFile,
    obs: &obws::Client,
) -> anyhow::Result<()> {
    log::debug!("Updating OBS: {:?}", modifications);

    let mut vlc_inputs = obs.inputs().list(Some("vlc_source")).await?;
    let scenes = obs.scenes().list().await?;

    match get_layout(state, layouts) {
        Some(layout) => {
            let target_layout_id = SceneId::Name(&layout.name);

            if !scenes.scenes.iter().any(|s| s.name != layout.name) {
                return Err(anyhow!(format!(
                    "OBS has no scene named {}, which the current layout requires.",
                    layout.name
                )));
            }

            // Modify commentary text
            if modifications.contains(&ModifiedState::Commentary) {
                log::debug!("Updating commentator list");
                let comm_setting = SpecificFreetype {
                    text: &state.get_commentators().join("\n"),
                };
                obs.inputs()
                    .set_settings(SetSettings {
                        input: InputId::Name("commentary"),
                        settings: &comm_setting,
                        overlay: Some(true),
                    })
                    .await?;
            }

            let scene_items = obs.scene_items().list(target_layout_id).await?;

            for (idx, player) in state.active_players.iter().enumerate() {
                log::debug!("Updating player {}", player);
                let stream_source_id_name = format!("streamer_{}", player);
                let stream_source_id = InputId::Name(&stream_source_id_name);

                let mut just_created = false;
                if !vlc_inputs.iter().any(|i| i.id.name == stream_source_id) {
                    // Source does not exist, create source
                    log::debug!("Creating source for {}", player);
                    let vlc_setting = VLC {
                        playlist: vec![PlaylistItem {
                            hidden: false,
                            selected: false,
                            value: state.streams.get(player).unwrap(),
                        }],
                    };

                    let new_input = inputs::Create {
                        scene: target_layout_id,
                        input: &stream_source_id_name,
                        kind: "vlc_source",
                        settings: Some(vlc_setting),
                        enabled: Some(false),
                    };

                    obs.inputs().create(new_input).await?;
                    just_created = true;
                } else if modifications.contains(&ModifiedState::PlayerStream(player.to_string())) {
                    // Source exists but requires new stream
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
                            input: stream_source_id,
                            settings: &vlc_setting,
                            overlay: Some(true),
                        })
                        .await?;

                    tokio::time::sleep(Duration::from_millis(200)).await;
                }

                if idx == 0 {
                    obs.inputs().set_muted(stream_source_id, false).await?;
                } else {
                    obs.inputs().set_muted(stream_source_id, true).await?;
                }

                // Update this player's view
                if modifications.contains(&ModifiedState::PlayerView(player.to_string()))
                    || modifications.contains(&ModifiedState::Layout)
                    || just_created
                {
                    log::debug!("Deleting old items for {}", player);
                    // Remove old scene_items
                    delete_scene_items_for_player(
                        obs,
                        target_layout_id,
                        &scene_items,
                        &stream_source_id_name,
                    )
                    .await?;

                    log::debug!("Updating name field for to {}", player);
                    // Update name field
                    let name_setting = SpecificFreetype {
                        text: &player.to_uppercase(),
                    };

                    obs.inputs()
                        .set_settings(SetSettings {
                            input: InputId::Name(&format!("name_{}", idx)),
                            settings: &name_setting,
                            overlay: Some(true),
                        })
                        .await?;

                    log::debug!("Creating new stream views for {}", player);
                    // Get the user-defined list of stream views in the layout
                    let stream_views: Vec<&obws::responses::scene_items::SceneItem> = scene_items
                        .iter()
                        .filter(|item| item.source_name.starts_with(&format!("stream_{}", idx)))
                        .collect();

                    // Create a VLC source scene item for each identified stream view
                    for view in stream_views {
                        let new_item = obs
                            .scene_items()
                            .create(CreateSceneItem {
                                scene: target_layout_id,
                                source: stream_source_id.into(),
                                enabled: Some(true),
                            })
                            .await?;

                        tokio::time::sleep(Duration::from_millis(200)).await;

                        obs.scene_items()
                            .set_index(SetIndex {
                                scene: target_layout_id,
                                item_id: new_item,
                                index: 0,
                            })
                            .await?;

                        let existing_transform = obs
                            .scene_items()
                            .transform(target_layout_id, view.id)
                            .await?;

                        let new_transform = SetTransform {
                            scene: target_layout_id,
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
                        obs.scene_items().set_transform(new_transform).await.map_err(|e| anyhow!(format!(
                            "Failed to set stream bounds: \n{:?}. \n\nIs the stream view {} transform set correctly (eg. with a bounding box enabled)?", e, view.source_name)))?;
                    }
                }

                // Remove this input from the list of inputs
                vlc_inputs.retain_mut(|i| i.id.name != stream_source_id);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            // Delete any input that was not visited
            for input in vlc_inputs {
                if settings.keep_unused_streams.unwrap_or(true) {
                    log::debug!("Deleting old items for unused input {}", input.id.name);
                    delete_scene_items_for_player(
                        obs,
                        target_layout_id,
                        &scene_items,
                        &input.id.name,
                    )
                    .await?;
                } else {
                    log::debug!("Deleting stale VLC input {}", input.id.name);
                    obs.inputs().remove(InputId::Name(&input.id.name)).await?;
                }
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
            if obs.ui().studio_mode_enabled().await? {
                obs.scenes()
                    .set_current_preview_scene(target_layout_id)
                    .await?;
                log::debug!("Triggering Studio Mode transition");
                match &settings.obs_transition {
                    Some(name) => {
                        obs.transitions().set_current(name).await?;
                        obs.transitions().trigger().await?;
                    }
                    None => obs.transitions().trigger().await?,
                }
                obs.scenes()
                    .set_current_preview_scene(target_layout_id)
                    .await?;
            } else if modifications.contains(&ModifiedState::Layout)
                && scenes.current_program_scene.unwrap().name != layout.name
            {
                log::debug!("Activating new layout: {}", layout.name);
                obs.scenes()
                    .set_current_program_scene(target_layout_id)
                    .await?;
            }

            log::debug!("OBS update complete");

            Ok(())
        }
        _ => Err(Error::UnknownLayout(
            "No known layout for the current player count.".to_string(),
        ))?,
    }
}
