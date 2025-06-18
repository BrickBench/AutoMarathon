use async_ringbuf::{
    traits::{Consumer, Producer, Split},
    wrap::AsyncWrap,
    AsyncHeapRb, AsyncRb,
};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, SampleRate,
};
use realfft::RealFftPlanner;
use ringbuf::storage::Heap;
use songbird::model::id::UserId;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::Mutex;

use dashmap::DashMap;
use poise::serenity_prelude as serenity;
use songbird::{model::payload::Speaking, CoreEvent, Event, EventContext};

use crate::{
    core::{db::ProjectDb, settings::Settings},
    web::streams::{ParticipantVoiceState, VoiceState, WebCommand},
    Directory,
};

const SAMPLE_RATE: u32 = 48_000;
const BUF_SIZE: usize = (SAMPLE_RATE as usize * 2) / 50; // 2 channels, 20ms buffer
const FFT_RESOLUTION: usize = 16;
const LATENCY: Duration = Duration::from_millis(1000);
const WEB_UPDATE_RATE: usize = 10;

type AudioProducer = AsyncWrap<Arc<AsyncRb<Heap<f32>>>, true, false>;

#[derive(Clone)]
struct Receiver {
    inner: Arc<InnerReceiver>,
}

struct InnerReceiver {
    update_count: AtomicUsize,
    last_tick_was_empty: AtomicBool,
    known_ssrcs: DashMap<u32, UserId>,
    producer: Mutex<AudioProducer>,
    settings: Arc<Settings>,
    db: Arc<ProjectDb>,
    directory: Directory,
    host: String,
}

impl Receiver {
    pub fn new(
        producer: AudioProducer,
        settings: Arc<Settings>,
        db: Arc<ProjectDb>,
        directory: Directory,
        host: &str,
    ) -> Self {
        Self {
            inner: Arc::new(InnerReceiver {
                update_count: AtomicUsize::new(0),
                last_tick_was_empty: AtomicBool::default(),
                known_ssrcs: DashMap::new(),
                producer: Mutex::new(producer),
                settings,
                db,
                directory,
                host: host.to_string(),
            }),
        }
    }
}

#[serenity::async_trait]
impl songbird::EventHandler for Receiver {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        use EventContext as Ctx;

        let mut float_voice = [0.0_f32; BUF_SIZE];
        let mut buffer = [0.0_f32; BUF_SIZE];
        match ctx {
            Ctx::SpeakingStateUpdate(Speaking {
                speaking: _,
                ssrc,
                user_id,
                ..
            }) => {
                if let Some(user) = user_id {
                    self.inner.known_ssrcs.insert(*ssrc, *user);
                }
            }
            Ctx::VoiceTick(tick) => {
                let current_update = self.inner.update_count.load(Ordering::SeqCst);

                let mut planner = RealFftPlanner::<f32>::new();
                let fft = planner.plan_fft_forward(BUF_SIZE);
                let mut voice_users = HashMap::new();
                let mut fft_result = fft.make_output_vec();

                let should_update_voice = current_update >= WEB_UPDATE_RATE;

                self.inner
                    .last_tick_was_empty
                    .store(false, Ordering::SeqCst);

                for (ssrc, data) in &tick.speaking {
                    if let Some(decoded_voice) = data.decoded_voice.as_ref() {
                        if let Some(id) = self.inner.known_ssrcs.get(ssrc) {
                            let volume = self
                                .inner
                                .db
                                .get_discord_user_volume(&id.0.to_string())
                                .await
                                .ok()
                                .flatten()
                                .unwrap_or(100) as f32
                                / 100.0;

                            let voice_len = decoded_voice.len();
                            for i in 0..voice_len {
                                float_voice[i] = decoded_voice[i] as f32 / i16::MAX as f32;
                                buffer[i] += float_voice[i] * volume;
                            }

                            if should_update_voice {
                                let fft = if self.inner.settings.transmit_voice_dft.unwrap_or(false)
                                {
                                    fft.process(&mut float_voice, &mut fft_result).unwrap();

                                    let step_size = fft_result.len() / FFT_RESOLUTION;
                                    let mut compressed_result = [0.0_f32; FFT_RESOLUTION];
                                    (0..compressed_result.len()).for_each(|i| {
                                        let mut sum = 0.0;
                                        (i * step_size..(i + 1) * step_size).for_each(|j| {
                                            sum += fft_result[j].norm_sqr();
                                        });
                                        compressed_result[i] = sum / step_size as f32;
                                    });
                                    compressed_result
                                } else {
                                    [0.0_f32; FFT_RESOLUTION]
                                };

                                voice_users.insert(
                                    id.0,
                                    ParticipantVoiceState {
                                        active: true,
                                        peak_db: *float_voice
                                            .iter()
                                            .max_by(|a, b| a.total_cmp(b))
                                            .unwrap_or(&0.0),
                                        voice_dft: fft,
                                    },
                                );
                            }
                        }
                    }
                }

                for ssrc in &tick.silent {
                    if let Some(id) = self.inner.known_ssrcs.get(ssrc) {
                        voice_users.insert(
                            id.0,
                            ParticipantVoiceState {
                                active: false,
                                peak_db: 0.0,
                                voice_dft: [0.0_f32; FFT_RESOLUTION],
                            },
                        );
                    }
                }

                if should_update_voice {
                    self.inner
                        .directory
                        .web_actor
                        .send(WebCommand::SendVoiceState(VoiceState {
                            host: self.inner.host.clone(),
                            voice_users,
                        }));

                    self.inner.update_count.store(0, Ordering::SeqCst);
                } else {
                    self.inner
                        .update_count
                        .store(current_update + 1, Ordering::SeqCst);
                }
                self.inner.producer.lock().await.push_slice(&buffer);
            }
            _ => {
                // We won't be registering this struct for any more event classes.
                unimplemented!()
            }
        }

        None
    }
}

const USE_JACK: bool = false;
async fn create_audio_device() -> anyhow::Result<Device> {
    log::info!("Initializing audio device");

    if USE_JACK {
        log::info!("Using JACK audio host");
        let host =  cpal::host_from_id(cpal::available_hosts()
        .into_iter()
        .find(|id| *id == cpal::HostId::Jack)
        .expect(
            "make sure --features jack is specified. only works on OSes where jack is available",
        ))?;

        host.output_devices()?.for_each(|device| {
            println!("Output device: {:?} ", device.name());
        });

        host.default_output_device()
            .ok_or(anyhow::anyhow!("No default output device found."))
    } else {
        let host = cpal::default_host();
        host.output_devices()?
            .find(|d| d.name().unwrap_or_default().contains("pipewire"))
            .or_else(|| host.default_output_device())
            .ok_or(anyhow::anyhow!("No default output device found."))
    }
}

pub async fn connect_to_voice(
    context: &serenity::Context,
    directory: Directory,
    db: Arc<ProjectDb>,
    settings: Arc<Settings>,
    host_name: &str,
) -> anyhow::Result<()> {
    log::info!("Connecting to voice channel");
    let host = settings
        .obs_hosts
        .get(host_name)
        .expect("Host not found")
        .clone();

    let guild_id = std::num::NonZero::new(host.discord_voice_channel_guild_id.unwrap()).unwrap();
    let channel_id = std::num::NonZero::new(host.discord_voice_channel_id.unwrap()).unwrap();
    let manager = songbird::get(context)
        .await
        .expect("Failed to init songbird")
        .clone();
    {
        let lock = manager.get_or_insert(guild_id);

        let mut handler = lock.lock().await;

        let output_device = create_audio_device().await?;

        log::info!("Using output device: {}", output_device.name()?);
        let good_config_range = output_device
            .supported_output_configs()?
            .find(|config| {
                config.channels() == 2
                    && config.min_sample_rate().0 <= SAMPLE_RATE
                    && config.max_sample_rate().0 >= SAMPLE_RATE
            })
            .ok_or(anyhow::anyhow!(
                "No suitable output config found for sample rate {} and 2 channels",
                SAMPLE_RATE
            ))?;

        let config = good_config_range.with_sample_rate(SampleRate(SAMPLE_RATE));

        log::info!(
            "Output device config: {:?} channels, {}Hz sample rate",
            config.channels(),
            config.sample_rate().0
        );

        let latency_frames = config.sample_rate().0 as f64 * LATENCY.as_secs_f64();
        let latency_samples = latency_frames as usize * config.channels() as usize;

        let ringbuf = AsyncHeapRb::<f32>::new(latency_samples * 2);
        let (mut ring_producer, mut ring_consumer) = ringbuf.split();
        for _ in 0..latency_samples {
            ring_producer.try_push(0.0).unwrap();
        }

        let output_fn = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            for sample in data {
                *sample = ring_consumer.try_pop().unwrap_or(0.0);
            }
        };

        let output_stream = output_device.build_output_stream(
            &config.into(),
            output_fn,
            move |err| log::error!("an error occurred on stream: {}", err),
            None,
        )?;

        let receiver = Receiver::new(ring_producer, settings, db, directory, host_name);

        handler.add_global_event(CoreEvent::SpeakingStateUpdate.into(), receiver.clone());
        handler.add_global_event(CoreEvent::VoiceTick.into(), receiver.clone());

        output_stream.play()?;

        // live forever
        std::mem::forget(output_stream);
    }

    match manager.join(guild_id, channel_id).await {
        Ok(call) => {
            let call = call.lock().await;
            log::info!(
                "Joined voice channel at {:?}, {:?}",
                call.config().decode_sample_rate,
                call.config().decode_channels
            );
        }
        Err(err) => {
            // Although we failed to join, we need to clear out existing event handlers on the call.
            _ = manager.remove(guild_id).await;

            log::error!("Failed to join voice channel: {:?}", err.to_string());
        }
    }

    Ok(())
}
