use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::anyhow;
use obws::{
    requests::{
        inputs::{self, InputId, SetSettings, Volume},
        scene_items::{
            Bounds, CreateSceneItem, Position, SceneItemTransform, SetIndex, SetTransform,
        },
        scenes::SceneId,
        EventSubscription,
    },
    responses::scene_items::SceneItem,
};
use serde::Serialize;
use sqlx::prelude::FromRow;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    db::ProjectDb,
    error::Error,
    settings::Settings,
    stream::{ModifiedStreamState, StreamState},
    ActorRef, Rto,
};

/// Individual layout settings
#[derive(PartialEq, Debug, FromRow)]
pub struct ObsLayout {
    pub name: String,
    /// Whether this layout should be the default layout for the provided player count
    pub default_layout: bool,
    pub runner_count: u32,
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

/// Return the appropriate layout for the given project state
fn get_layout<'a>(state: &StreamState, layouts: &'a [ObsLayout]) -> Option<&'a ObsLayout> {
    layouts
        .iter()
        .find(|l| {
            if let Some(name) = state.requested_layout.clone() {
                l.name == name
            } else {
                false
            }
        })
        .filter(|l| l.runner_count as usize == state.stream_runners.len())
        .or_else(|| {
            layouts
                .iter()
                .find(|l| l.default_layout && l.runner_count as usize == state.stream_runners.len())
        }) // Use the default layout for the runner count
        .or_else(|| {
            layouts
                .iter()
                .find(|l| l.runner_count as usize == state.stream_runners.len())
        }) // Use any layout for the runner count
}

/// Requests for ObsActor
pub enum ObsCommand {
    UpdateState(String, Vec<ModifiedStreamState>, Rto<()>),
    SetLadderLeagueId(String, String, Rto<()>),
    StartStream(String, Rto<()>),
    EndStream(String, Rto<()>),
}

pub type ObsActor = ActorRef<ObsCommand>;

type HostMap = HashMap<String, obws::Client>;

pub async fn run_obs(
    settings: Arc<Settings>,
    db: Arc<ProjectDb>,
    mut rx: UnboundedReceiver<ObsCommand>,
) -> Result<(), anyhow::Error> {
    let mut host_map: HostMap = HostMap::new();

    loop {
        match rx.recv().await.unwrap() {
            ObsCommand::UpdateState(event, modifications, rto) => {
                match db.get_stream(&event).await {
                    Ok(stream) => {
                        if let Err(e) =
                            connect_client_for_host(&stream.obs_host, &mut host_map, &settings)
                                .await
                        {
                            rto.reply(Err(e));
                        } else {
                            let obs = host_map.get_mut(&stream.obs_host).unwrap();
                            rto.reply(
                                update_obs_state(&event, &db, &settings, &modifications, &obs)
                                    .await,
                            );
                        }
                    }
                    Err(e) => {
                        rto.reply(Err(e));
                    }
                }
            }
            ObsCommand::StartStream(event, rto) => match db.get_stream(&event).await {
                Ok(stream) => {
                    if let Err(e) =
                        connect_client_for_host(&stream.obs_host, &mut host_map, &settings).await
                    {
                        rto.reply(Err(e));
                    } else {
                        let obs = host_map.get_mut(&stream.obs_host).unwrap();
                        rto.reply(obs.streaming().start().await.map_err(|e| e.into()));
                    }
                }
                Err(e) => {
                    rto.reply(Err(e));
                }
            },
            ObsCommand::EndStream(event, rto) => match db.get_stream(&event).await {
                Ok(stream) => {
                    if let Err(e) =
                        connect_client_for_host(&stream.obs_host, &mut host_map, &settings).await
                    {
                        rto.reply(Err(e));
                    } else {
                        let obs = host_map.get_mut(&stream.obs_host).unwrap();
                        rto.reply(obs.streaming().stop().await.map_err(|e| e.into()));
                    }
                }
                Err(e) => {
                    rto.reply(Err(e));
                }
            },
            ObsCommand::SetLadderLeagueId(event, league, rto) => {
                log::debug!("Configuring ladder league URL for {}", event);
                match db.get_stream(&event).await {
                    Ok(stream) => {
                        if let Err(e) =
                            connect_client_for_host(&stream.obs_host, &mut host_map, &settings)
                                .await
                        {
                            rto.reply(Err(e));
                        } else {
                            let obs = host_map.get_mut(&stream.obs_host).unwrap();
                            rto.reply(update_ladder_league_urls(&obs, &league, &settings).await);
                        }
                    }
                    Err(e) => {
                        rto.reply(Err(e));
                    }
                }
            }
        };
    }
}

async fn connect_client_for_host(
    host: &str,
    host_map: &mut HostMap,
    settings: &Settings,
) -> anyhow::Result<()> {
    let old_host = host_map.get(host);
    if let Some(client) = old_host {
        if client.general().version().await.is_ok() {
            return Ok(());
        } else {
            log::debug!("Removing stale OBS client for host {}", host);
            host_map.remove(host);
        }
    }

    let config = settings
        .obs_hosts
        .get(host)
        .ok_or_else(|| anyhow!(format!("No OBS host configuration found for host {}", host)))?;

    let obs_config = obws::client::ConnectConfig {
        host: config.obs_ip.to_owned(),
        port: config.obs_port.to_owned(),
        password: config.obs_password.to_owned(),
        event_subscriptions: Some(EventSubscription::NONE),
        broadcast_capacity: None,
        connect_timeout: Duration::from_secs(30),
    };

    let obs = obws::Client::connect_with_config(obs_config).await?;

    let obs_version = obs.general().version().await?;
    log::info!(
        "Connected to OBS version {}, websocket {}, running on {} ({})",
        obs_version.obs_version.to_string(),
        obs_version.obs_web_socket_version.to_string(),
        obs_version.platform,
        obs_version.platform_description
    );

    host_map.insert(host.to_owned(), obs);

    Ok(())
}

pub async fn update_ladder_league_urls(
    obs: &obws::Client,
    league_id: &str,
    settings: &Settings,
) -> anyhow::Result<()> {
    log::debug!("Updating browser sources to use ID: {}", league_id);
    let host_prefix = "http://10.10.0.4:35065/";
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
                input: InputId::Name(&format!("runner_name_{}", i )),
                settings: &Browser { url: &name },
                overlay: Some(true),
            })
            .await?;
        obs.inputs()
            .set_settings(SetSettings {
                input: InputId::Name(&format!("post_info_{}", i)),
                settings: &Browser { url: &post_info },
                overlay: Some(true),
            })
            .await?;
        obs.inputs()
            .set_settings(SetSettings {
                input: InputId::Name(&format!("pictures_{}", i)),
                settings: &Browser { url: &pictures },
                overlay: Some(true),
            })
            .await?;
        obs.inputs()
            .set_settings(SetSettings {
                input: InputId::Name(&format!("overlay_{}", i)),
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
    event: &str,
    db: &ProjectDb,
    settings: &Settings,
    modifications: &[ModifiedStreamState],
    obs: &obws::Client,
) -> anyhow::Result<()> {
    log::debug!("Updating OBS: {:?}", modifications);

    let mut vlc_inputs = obs.inputs().list(Some("vlc_source")).await?;
    let scenes = obs.scenes().list().await?;
    let state = &db.get_stream(event).await?;

    match get_layout(state, &db.get_layouts().await?) {
        Some(layout) => {
            let target_layout_id = SceneId::Name(&layout.name);

            if !scenes.scenes.iter().any(|s| s.name != layout.name) {
                return Err(anyhow!(format!(
                    "OBS has no scene named {}, which the current layout requires.",
                    layout.name
                )));
            }

            // Modify commentary text
            if modifications.contains(&ModifiedStreamState::Commentary) {
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

            for (idx, runner) in state.stream_runners.iter().enumerate() {
                log::debug!("Updating player {}", runner.name);
                let stream_source_id_name = format!("streamer_{}", runner.name);
                let stream_source_id = InputId::Name(&stream_source_id_name);

                let mut just_created = false;
                let good_url = runner
                    .cached_stream_url
                    .to_owned()
                    .ok_or_else(|| anyhow!("No cached stream URL for runner {}", runner.name))?;

                if !vlc_inputs.iter().any(|i| i.id.name == stream_source_id) {
                    // Source does not exist, create source
                    log::debug!("Creating source for {}", runner.name);
                    let vlc_setting = VLC {
                        playlist: vec![PlaylistItem {
                            hidden: false,
                            selected: false,
                            value: &good_url,
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
                } else if modifications
                    .contains(&ModifiedStreamState::PlayerStream(runner.name.to_string()))
                {
                    // Source exists but requires new stream
                    log::debug!("Applying stream change to {}", runner.name);
                    let vlc_setting = VLC {
                        playlist: vec![PlaylistItem {
                            hidden: false,
                            selected: false,
                            value: &good_url,
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
                    obs.inputs()
                        .set_volume(
                            stream_source_id,
                            Volume::Mul(0.01 * runner.volume_percent as f32),
                        )
                        .await?;
                } else {
                    obs.inputs().set_muted(stream_source_id, true).await?;
                }

                // Update this player's view
                if modifications.contains(&ModifiedStreamState::PlayerView(runner.name.to_string()))
                    || modifications.contains(&ModifiedStreamState::Layout)
                    || just_created
                {
                    log::debug!("Deleting old items for {}", runner.name);
                    // Remove old scene_items
                    delete_scene_items_for_player(
                        obs,
                        target_layout_id,
                        &scene_items,
                        &stream_source_id_name,
                    )
                    .await?;

                    let name_field = &format!("name_{}", idx);
                    if scene_items.iter().any(|s| &s.source_name == name_field) {
                        log::debug!("Updating name field for to {}", runner.name);
                        // Update name field
                        let name_setting = SpecificFreetype {
                            text: &runner.name.to_uppercase(),
                        };

                        obs.inputs()
                            .set_settings(SetSettings {
                                input: InputId::Name(name_field),
                                settings: &name_setting,
                                overlay: Some(true),
                            })
                            .await?;
                    } else {
                        log::debug!("{} has no nametag, skipping", runner.name);
                    }

                    log::debug!("Creating new stream views for {}", runner.name);
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
            } else if modifications.contains(&ModifiedStreamState::Layout)
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
