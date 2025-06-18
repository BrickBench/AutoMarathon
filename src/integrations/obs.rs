use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::Duration,
};

use anyhow::anyhow;
use obws::{
    common::MonitorType,
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
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    core::{
        db::ProjectDb,
        event::Event,
        settings::Settings,
        stream::{ModifiedStreamState, StreamState},
    },
    error::Error,
    ActorRef, Rto,
};

// OBS FreeType partial settings parameters
#[derive(Serialize)]
struct SpecificFreetype<'a> {
    text: &'a str,
}

/// OBS VLC partial source parameters
#[allow(clippy::upper_case_acronyms)]
#[derive(Serialize, Deserialize)]
struct VLC {
    playlist: Vec<PlaylistItem>,
}

/// OBS PlaylistItem parameters
#[derive(Serialize, Deserialize)]
struct PlaylistItem {
    hidden: bool,
    selected: bool,
    value: String,
}

/// A VLC source location in OBS, derived from some existing source
#[derive(Serialize, Clone, Debug)]
pub struct VlcSourceBounds {
    name: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    crop_left: u32,
    crop_right: u32,
    crop_top: u32,
    crop_bottom: u32,
}

/// A scene in OBS
#[derive(Serialize, Clone, Debug)]
pub struct ObsScene {
    /// Scene name
    pub name: String,
    /// Whether the scene is active and visible
    pub active: bool,
    /// Sources grouped by runner index
    pub sources: HashMap<usize, Vec<VlcSourceBounds>>,
}

/// A user currently present in the commentary Discord channel.
/// This may map to a `Person` based on their username
#[derive(Serialize, Clone, Debug)]
pub struct DiscordUser {
    /// The Discord username of this user.
    pub username: String,
    /// The Discord unique ID of this user.
    pub id: u64,
    /// The volume of this Discord user.
    pub volume_percent: u32,
    /// If this user is a participant in the event,
    /// this is the participant ID
    pub participant: Option<i64>,
}

/// The status of an OBS host
#[derive(Serialize, Clone, Debug)]
pub struct ObsHostState {
    /// Whether the host is connected.
    pub connected: bool,
    /// Whether the host is streaming.
    pub streaming: bool,
    /// The frame rate of the stream.
    pub stream_frame_rate: u32,
    /// The scenes present in the host by name.
    pub scenes: HashMap<String, ObsScene>,
    /// The discord users present in this host's voice channel.
    /// Maps from Discord UUID -> user
    pub discord_users: HashMap<u64, DiscordUser>,
}

/// Requests for ObsActor
pub enum HostCommand {
    UpdateState(i64, Vec<ModifiedStreamState>, Rto<()>),
    GetState(Rto<HashMap<String, ObsHostState>>),
    StartStream(String, Rto<()>),
    EndStream(String, Rto<()>),
    SetStreamCommentators(String, Vec<(u64, String)>, Rto<()>),
    SetCommentatorVolume(String, u32, Rto<()>),
    GetCommentatorState(String, Rto<HashMap<u64, DiscordUser>>),
}

pub type HostActor = ActorRef<HostCommand>;

type HostMap = HashMap<String, obws::Client>;

pub async fn run_obs(
    settings: Arc<Settings>,
    db: Arc<ProjectDb>,
    mut rx: UnboundedReceiver<HostCommand>,
) -> Result<(), anyhow::Error> {
    let mut host_map: HostMap = HostMap::new();
    let mut discord_users: HashMap<String, HashMap<u64, DiscordUser>> = HashMap::new();

    log::info!("Creating initial connection to OBS");
    for host in settings.obs_hosts.keys() {
        discord_users.insert(host.clone(), HashMap::new());
        let _ = connect_client_for_host(host, &mut host_map, &settings)
            .await
            .inspect_err(|e| {
                log::warn!("Failed to connect to OBS host {}: {:?}", host, e);
            });
    }

    loop {
        match rx.recv().await.unwrap() {
            HostCommand::UpdateState(event, modifications, rto) => {
                match db.get_stream(event).await {
                    Ok(stream) => {
                        if let Err(e) =
                            connect_client_for_host(&stream.obs_host, &mut host_map, &settings)
                                .await
                        {
                            rto.reply(Err(e));
                        } else {
                            let obs = host_map.get_mut(&stream.obs_host).unwrap();
                            rto.reply(
                                update_obs_state(&stream, &db, &settings, &modifications, obs)
                                    .await,
                            );
                        }
                    }
                    Err(e) => {
                        rto.reply(Err(e));
                    }
                }
            }
            HostCommand::StartStream(host, rto) => {
                if let Err(e) = connect_client_for_host(&host, &mut host_map, &settings).await {
                    rto.reply(Err(e));
                } else {
                    let obs = host_map.get_mut(&host).unwrap();
                    rto.reply(obs.streaming().start().await.map_err(|e| e.into()));
                }
            }
            HostCommand::SetStreamCommentators(host, commentators, rto) => {
                let host_users = discord_users.get_mut(&host).unwrap();
                host_users.clear();

                for commentator in commentators.iter() {
                    let maybe_participant =
                        db.get_participant_by_discord_id(&commentator.1).await.ok();

                    let volume = db
                        .get_discord_user_volume(&commentator.0.to_string())
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or(100);

                    host_users.insert(
                        commentator.0,
                        DiscordUser {
                            username: commentator.1.clone(),
                            id: commentator.0,
                            volume_percent: volume as u32,
                            participant: maybe_participant.map(|p| p.id),
                        },
                    );
                }

                rto.reply(Ok(()));
            }
            HostCommand::GetCommentatorState(host, rto) => {
                rto.reply(Ok(discord_users.get(&host).unwrap().clone()));
            }
            HostCommand::SetCommentatorVolume(name, volume, rto) => {
                for host in settings.obs_hosts.keys() {
                    let users = discord_users.get_mut(host).unwrap();
                    if let Some(user) = users.values_mut().find(|u| u.username == name) {
                        user.volume_percent = volume;
                        db.set_discord_user_volume(&user.id.to_string(), volume as i32)
                            .await?;
                    }
                }

                rto.reply(Ok(()))
            }
            HostCommand::EndStream(host, rto) => {
                if let Err(e) = connect_client_for_host(&host, &mut host_map, &settings).await {
                    rto.reply(Err(e));
                } else {
                    let obs = host_map.get_mut(&host).unwrap();
                    rto.reply(obs.streaming().stop().await.map_err(|e| e.into()));
                }
            }
            HostCommand::GetState(rto) => {
                rto.reply(get_obs_state(&mut host_map, &discord_users, &settings).await)
            }
        };
    }
}

/// Return the state of each OBS host by name.
async fn get_obs_state(
    host_map: &mut HostMap,
    commentary_map: &HashMap<String, HashMap<u64, DiscordUser>>,
    settings: &Settings,
) -> anyhow::Result<HashMap<String, ObsHostState>> {
    let mut states = HashMap::new();
    for host in settings.obs_hosts.keys() {
        let connected = connect_client_for_host(host, host_map, settings).await;
        if connected.is_ok() {
            let obs = host_map.get_mut(host).unwrap();

            let mut info = get_obs_client_info(obs).await?;
            info.discord_users = commentary_map.get(host).unwrap().clone();

            states.insert(host.clone(), info);
        } else {
            states.insert(
                host.clone(),
                ObsHostState {
                    connected: false,
                    streaming: false,
                    stream_frame_rate: 0,
                    scenes: HashMap::new(),
                    discord_users: commentary_map.get(host).unwrap().clone(),
                },
            );
        }
    }

    Ok(states)
}

static STREAM_ITEM_NAME_REGEX: OnceLock<Regex> = OnceLock::new();

async fn get_obs_client_info(obs: &obws::Client) -> anyhow::Result<ObsHostState> {
    let mut state = ObsHostState {
        connected: true,
        streaming: false,
        stream_frame_rate: 30,
        scenes: HashMap::new(),
        discord_users: HashMap::new(),
    };

    let settings = obs.config().video_settings().await?;

    state.connected = true;
    state.stream_frame_rate = settings.fps_numerator / settings.fps_denominator;
    state.streaming = obs.streaming().status().await?.active;

    let scenes = obs.scenes().list().await?.scenes;
    let current_scene = obs.scenes().current_program_scene().await?;
    for scene in scenes {
        let mut out_scene = ObsScene {
            name: scene.name.clone(),
            active: scene.name == current_scene.id.name,
            sources: HashMap::new(),
        };

        let scene_items = obs.scene_items().list(SceneId::Name(&scene.name)).await?;

        #[allow(clippy::regex_creation_in_loops)] // inited once
        let regex =
            STREAM_ITEM_NAME_REGEX.get_or_init(|| Regex::new(r"streamer_(\d+)_.*").unwrap());
        for item in scene_items {
            if regex.is_match(&item.source_name) {
                let caps = regex.captures(&item.source_name).unwrap();
                let idx = caps.get(1).unwrap().as_str().parse::<usize>()?;

                let transform = obs
                    .scene_items()
                    .transform(SceneId::Name(&scene.name), item.id)
                    .await?;

                out_scene
                    .sources
                    .entry(idx)
                    .or_default()
                    .push(VlcSourceBounds {
                        name: item.source_name.clone(),
                        x: transform.position_x,
                        y: transform.position_y,
                        width: transform.bounds_width,
                        height: transform.bounds_height,
                        crop_left: transform.crop_left,
                        crop_right: transform.crop_right,
                        crop_top: transform.crop_top,
                        crop_bottom: transform.crop_bottom,
                    });
            }
        }
        state.scenes.insert(scene.name, out_scene);
    }

    Ok(state)
}

/// Attemt to connect to an OBS instance
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

/// Delete all scene items for a player
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

/// Trigger the transition specified in the settings
pub async fn do_transition(
    obs: &obws::Client,
    settings: &Settings,
    long: bool,
) -> anyhow::Result<()> {
    let transition = if long {
        &settings.obs_long_transition
    } else {
        &settings.obs_short_transition
    };

    log::debug!("Triggering Studio Mode transition: {:?}", transition);

    match transition {
        Some(name) => {
            obs.transitions().set_current(name).await?;
            obs.transitions().trigger().await?;
        }
        None => obs.transitions().trigger().await?,
    }
    Ok(())
}

/// Return the appropriate scene layout for the given project state
fn get_layout<'a>(
    event: &Event,
    state: &StreamState,
    obs_state: &'a ObsHostState,
) -> Option<&'a ObsScene> {
    if let Some(layout) = obs_state.scenes.get(&state.requested_layout.clone()?) {
        return Some(layout);
    }

    for layout in &event.preferred_layouts {
        if let Some(layout) = obs_state.scenes.get(layout) {
            if layout.sources.len() == state.stream_runners.len() {
                return Some(layout);
            }
        }
    }

    obs_state
        .scenes
        .values()
        .find(|l| l.sources.len() == state.stream_runners.len())
}

/// Determine the correct streamlink URL for the given
/// element width and desired FPS.
fn calculate_best_url(
    width: u32,
    stream_fps: u32,
    urls: &HashMap<String, String>,
) -> Option<(String, u32)> {
    if urls.is_empty() {
        return None;
    }

    let desire_60fps = stream_fps >= 30;

    let mut closest_title = "best";
    let mut closest_url = urls["best"].clone();
    let mut closest_width = 1080;
    let mut closest_diff = u32::MAX;

    for (res, url) in urls {
        let elements: Vec<&str> = res.split('p').collect();
        let res_width = elements[0].parse::<u32>();

        if let Ok(stream_width) = res_width {
            let stream_fps = if elements.len() > 1 && elements[1].contains("60") {
                60
            } else {
                30
            };

            let stream_diff = (stream_width as i32 - width as i32).unsigned_abs();
            if stream_width >= width && stream_diff <= closest_diff {
                if stream_diff == closest_diff {
                    if (desire_60fps && stream_fps == 60) || (!desire_60fps && stream_fps == 30) {
                        closest_title = res;
                        closest_url.clone_from(url);
                    }
                } else {
                    closest_title = res;
                    closest_url.clone_from(url);
                    closest_width = stream_width;
                    closest_diff = stream_width - width;
                }
            }
        }
    }

    log::debug!("Selected stream URL {} for width {}", closest_title, width);

    Some((closest_url, closest_width))
}

/// Apply project state to OBS
pub async fn update_obs_state(
    state: &StreamState,
    db: &ProjectDb,
    settings: &Settings,
    modifications: &[ModifiedStreamState],
    obs: &obws::Client,
) -> anyhow::Result<()> {
    log::debug!("Updating OBS: {:?}", modifications);

    let mut vlc_inputs = obs.inputs().list(Some("vlc_source")).await?;

    vlc_inputs.retain(|v| v.id.name.starts_with("stream"));

    let obs_state = get_obs_client_info(obs).await?;
    let scenes = obs.scenes().list().await?;
    let event = &db.get_event(state.event).await?;

    match get_layout(event, state, &obs_state) {
        Some(layout) => {
            let target_layout_id = SceneId::Name(&layout.name);

            if scenes.scenes.iter().all(|s| s.name != layout.name) {
                return Err(anyhow!(format!(
                    "OBS has no scene named {}, which the current layout requires.",
                    layout.name
                )));
            }

            let scene_items = obs.scene_items().list(target_layout_id).await?;
            for (idx, runner) in state.stream_runners.iter() {
                let runner = db.get_runner(*runner).await?;
                log::debug!("Updating player {}", runner.participant_data.name);
                let stream_source_id_name = format!("streamer_{}", runner.participant_data.name);
                let stream_source_id = InputId::Name(&stream_source_id_name);

                // Get the stream views for this runner.
                let stream_views = layout
                    .sources
                    .get(&(*idx as usize))
                    .cloned()
                    .unwrap_or_default();

                let mut just_created = false;
                let mut good_url = None;
                if !stream_views.is_empty() {
                    let max_width = stream_views
                        .iter()
                        .map(|v| v.width as u32)
                        .max()
                        .unwrap_or(1920);

                    if let Some(stream) = runner.override_stream_url {
                        if !stream.trim().is_empty() {
                            good_url = Some((stream, 1080));
                        }
                    }

                    if good_url.is_none() {
                        good_url = calculate_best_url(
                            max_width,
                            obs_state.stream_frame_rate,
                            &runner.stream_urls,
                        );
                    }

                    match &good_url {
                        Some((url, _)) => {
                            if !vlc_inputs.iter().any(|i| i.id.name == stream_source_id) {
                                // Source does not exist, create source
                                log::debug!("Creating source for {}", runner.participant_data.name);
                                let vlc_setting = VLC {
                                    playlist: vec![PlaylistItem {
                                        hidden: false,
                                        selected: false,
                                        value: url.to_owned(),
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
                                obs.inputs()
                                    .set_audio_monitor_type(stream_source_id, MonitorType::None)
                                    .await?;
                                just_created = true;
                            } else {
                                // Source exists, check if stream is up to date
                                let old_setting =
                                    obs.inputs().settings::<VLC>(stream_source_id).await?;
                                if old_setting.settings.playlist.is_empty()
                                    || old_setting.settings.playlist[0].value != *url
                                {
                                    log::debug!(
                                        "Applying stream change to {}",
                                        runner.participant_data.name
                                    );
                                    let vlc_setting = VLC {
                                        playlist: vec![PlaylistItem {
                                            hidden: false,
                                            selected: false,
                                            value: url.to_owned(),
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
                                } else {
                                    log::debug!(
                                        "Stream for {} is up to date",
                                        runner.participant_data.name
                                    );
                                }
                            }

                            if state
                                .audible_runner
                                .as_ref()
                                .map(|r| *r == runner.participant)
                                .unwrap_or(*idx == 0)
                            {
                                obs.inputs().set_muted(stream_source_id, false).await?;
                                obs.inputs()
                                    .set_volume(
                                        stream_source_id,
                                        Volume::Db(-(runner.stream_volume_percent as f32).abs()),
                                    )
                                    .await?;
                            } else {
                                obs.inputs().set_muted(stream_source_id, true).await?;
                            }
                        }
                        None => {
                            log::warn!(
                                "No stream URL for {}, skipping...",
                                runner.participant_data.name
                            )
                        }
                    }
                }

                // Update this player's views
                if modifications.contains(&ModifiedStreamState::RunnerView(runner.participant))
                    || modifications.contains(&ModifiedStreamState::Layout)
                    || just_created
                {
                    log::debug!("Deleting old items for {}", runner.participant_data.name);

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
                        log::debug!(
                            "Updating name field for to {}",
                            runner.participant_data.name
                        );
                        // Update name field
                        let name_setting = SpecificFreetype {
                            text: &runner.participant_data.name.to_uppercase(),
                        };

                        obs.inputs()
                            .set_settings(SetSettings {
                                input: InputId::Name(name_field),
                                settings: &name_setting,
                                overlay: Some(true),
                            })
                            .await?;
                    } else {
                        log::debug!("{} has no nametag, skipping", runner.participant_data.name);
                    }

                    if good_url.is_some() {
                        log::debug!(
                            "Creating new stream views for {}",
                            runner.participant_data.name
                        );

                        let crop_scale: f64 = good_url.unwrap().1 as f64 / 1080.0;

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

                            let location_idx = 0;

                            obs.scene_items()
                                .set_index(SetIndex {
                                    scene: target_layout_id,
                                    item_id: new_item,
                                    index: location_idx,
                                })
                                .await?;

                            let new_transform = SetTransform {
                                scene: target_layout_id,
                                item_id: new_item,
                                transform: SceneItemTransform {
                                    position: Some(Position {
                                        x: Some(view.x),
                                        y: Some(view.y),
                                    }),
                                    rotation: None,
                                    scale: None,
                                    alignment: None, // TODO
                                    bounds: Some(Bounds {
                                        r#type: Some(obws::common::BoundsType::Stretch),
                                        alignment: None, // TODO 2
                                        width: Some(view.width),
                                        height: Some(view.height),
                                    }),
                                    crop: Some(obws::requests::scene_items::Crop {
                                        left: Some((view.crop_left as f64 * crop_scale) as u32),
                                        right: Some((view.crop_right as f64 * crop_scale) as u32),
                                        top: Some((view.crop_top as f64 * crop_scale) as u32),
                                        bottom: Some((view.crop_bottom as f64 * crop_scale) as u32),
                                    }),
                                },
                            };
                            obs.scene_items().set_transform(new_transform).await.map_err(|e| anyhow!(format!(
                            "Failed to set stream bounds: \n{:?}. \n\nIs the stream view {} transform set correctly (eg. with a bounding box enabled)?", e, view.name)))?;
                        }
                    } else {
                        log::warn!(
                            "Not creating stream views for {} due to missing stream URL",
                            runner.participant_data.name
                        );
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

            let changing_layout = modifications.contains(&ModifiedStreamState::Layout)
                && scenes.current_program_scene.unwrap().name != layout.name;

            if obs.ui().studio_mode_enabled().await? {
                obs.scenes()
                    .set_current_preview_scene(target_layout_id)
                    .await?;
                do_transition(obs, settings, changing_layout).await?;
                obs.scenes()
                    .set_current_preview_scene(target_layout_id)
                    .await?;
            } else if changing_layout {
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
