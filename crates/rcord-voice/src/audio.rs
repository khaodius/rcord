//! Audio devices and the media loop: microphone to Opus packets, and Opus packets from every
//! speaker to the output device.
//!
//! The device callbacks only move samples in and out of two small queues. Everything else
//! runs on one dedicated thread, woken every 5 ms:
//!
//! - **Capture**: downmix to mono, resample to 48 kHz, apply input gain, suppress noise
//!   (RNNoise), decide whether to transmit (voice activity or push to talk), and encode 20 ms
//!   Opus frames with in-band forward error correction.
//! - **Playback**: each speaker has a jitter buffer and an Opus decoder; lost frames are
//!   rebuilt from the next packet's FEC data or concealed. Speakers are mixed with their own
//!   volume, the output gain is applied with a soft limiter, and the result is resampled to
//!   the device. A frame is produced whenever the device queue runs low, so playback follows
//!   the device clock and never drifts.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use parking_lot::Mutex;

use crate::jitter::{JitterBuffer, Playout};
use crate::resample::Resampler;

pub const SAMPLE_RATE: u32 = 48_000;
/// Samples per channel in one 20 ms frame.
pub const FRAME: usize = 960;
/// Opus's three-byte silence frame. Five are sent after speech ends so decoders do not
/// interpolate into the gap.
pub const SILENCE: [u8; 3] = [0xf8, 0xff, 0xfe];

/// How the microphone decides when to transmit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transmit {
    /// Transmit while the voice is louder than the threshold. `None` adapts the threshold
    /// to the room's noise floor.
    VoiceActivity { threshold_db: Option<f32> },
    /// Transmit while the key is held, and for `release_ms` after.
    PushToTalk { release_ms: u32 },
}

/// How hard the microphone is cleaned up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Denoise {
    Off,
    /// RNNoise removes steady background noise; voice detection uses its voice probability.
    #[default]
    Standard,
    /// Standard, plus anything RNNoise is not confident is a voice is turned down, so
    /// keyboards and clicks between words are cut too.
    Strong,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioSettings {
    /// Device names as [`devices`] lists them; `None` follows the system default.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    /// Linear gain, 0 to 2.
    pub input_gain: f32,
    pub output_gain: f32,
    pub transmit: Transmit,
    pub noise_suppression: Denoise,
    /// Brings quiet and loud voices to a steady level.
    pub automatic_gain: bool,
    pub bitrate_kbps: u32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            input_device: None,
            output_device: None,
            input_gain: 1.0,
            output_gain: 1.0,
            transmit: Transmit::VoiceActivity { threshold_db: None },
            noise_suppression: Denoise::Standard,
            automatic_gain: true,
            bitrate_kbps: 64,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceList {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

/// Names of the audio devices on this system, for the settings page.
pub fn devices() -> DeviceList {
    let host = cpal::default_host();
    let names = |it: Result<_, cpal::Error>| -> Vec<String> {
        it.map(|devs: Box<dyn Iterator<Item = cpal::Device>>| {
            devs.filter_map(|d| d.description().ok().map(|d| d.name().to_owned()))
                .collect()
        })
        .unwrap_or_default()
    };
    DeviceList {
        inputs: names(
            host.input_devices()
                .map(|d| Box::new(d) as Box<dyn Iterator<Item = cpal::Device>>),
        ),
        outputs: names(
            host.output_devices()
                .map(|d| Box::new(d) as Box<dyn Iterator<Item = cpal::Device>>),
        ),
    }
}

/// An Opus frame received from a participant, already decrypted.
#[derive(Debug)]
pub struct Inbound {
    pub user_id: u64,
    /// Sound from a stream being watched rather than from someone's microphone; it has
    /// its own volume and never lights the speaking ring.
    pub stream: bool,
    pub ssrc: u32,
    pub sequence: u16,
    pub opus: Vec<u8>,
}

/// What the media loop hands to the connection.
#[derive(Debug, PartialEq, Eq)]
pub enum Outbound {
    /// A 20 ms Opus frame to send.
    Opus(Vec<u8>),
    /// The microphone started or stopped transmitting.
    Speaking(bool),
    /// An encoded video frame to send on a Go Live stream.
    Video(crate::video::EncodedFrame),
}

/// Changes the app shows.
#[derive(Debug, Clone, PartialEq)]
pub enum AudioEvent {
    /// A participant started or stopped making sound. `0` is this client.
    Speaking { user_id: u64, speaking: bool },
    /// Microphone level in dBFS after processing, about ten times a second.
    InputLevel(f32),
    /// A device could not be opened or was lost.
    DeviceError(String),
}

struct Shared {
    settings: Mutex<AudioSettings>,
    settings_changed: AtomicBool,
    devices_changed: AtomicBool,
    muted: AtomicBool,
    deafened: AtomicBool,
    push_to_talk: AtomicBool,
    stop: AtomicBool,
    volumes: Mutex<HashMap<u64, f32>>,
    /// Volume of each watched stream's sound, by streamer.
    stream_volumes: Mutex<HashMap<u64, f32>>,
}

/// Controls a running media loop. Dropping it stops the loop and closes the devices.
pub struct AudioEngine {
    shared: Arc<Shared>,
    inbound: mpsc::Sender<Inbound>,
}

impl std::fmt::Debug for AudioEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioEngine").finish_non_exhaustive()
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

impl AudioEngine {
    pub fn start(
        settings: AudioSettings,
        outbound: tokio::sync::mpsc::UnboundedSender<Outbound>,
        events: async_channel::Sender<AudioEvent>,
    ) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            settings: Mutex::new(settings),
            settings_changed: AtomicBool::new(false),
            devices_changed: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            deafened: AtomicBool::new(false),
            push_to_talk: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            volumes: Mutex::new(HashMap::new()),
            stream_volumes: Mutex::new(HashMap::new()),
        });
        let (tx, rx) = mpsc::channel();
        let thread_shared = shared.clone();
        std::thread::Builder::new()
            .name("rcord-audio".into())
            .spawn(move || MediaLoop::new(thread_shared, rx, outbound, events).run())?;
        Ok(Self {
            shared,
            inbound: tx,
        })
    }

    /// Where the connection delivers received frames.
    pub fn sink(&self) -> mpsc::Sender<Inbound> {
        self.inbound.clone()
    }

    pub fn set_muted(&self, muted: bool) {
        self.shared.muted.store(muted, Ordering::Relaxed);
    }

    pub fn set_deafened(&self, deafened: bool) {
        self.shared.deafened.store(deafened, Ordering::Relaxed);
    }

    /// Push-to-talk key state, from the app's key handling.
    pub fn set_push_to_talk(&self, held: bool) {
        self.shared.push_to_talk.store(held, Ordering::Relaxed);
    }

    /// Volume for one participant, 0 to 2.
    pub fn set_user_volume(&self, user_id: u64, volume: f32) {
        self.shared
            .volumes
            .lock()
            .insert(user_id, volume.clamp(0.0, 2.0));
    }

    /// Volume for the sound of `streamer`'s stream, 0 to 2.
    pub fn set_stream_volume(&self, streamer: u64, volume: f32) {
        self.shared
            .stream_volumes
            .lock()
            .insert(streamer, volume.clamp(0.0, 2.0));
    }

    pub fn update_settings(&self, settings: AudioSettings) {
        let mut cur = self.shared.settings.lock();
        if cur.input_device != settings.input_device || cur.output_device != settings.output_device
        {
            self.shared.devices_changed.store(true, Ordering::Relaxed);
        }
        *cur = settings;
        self.shared.settings_changed.store(true, Ordering::Relaxed);
    }
}

/// Plays the microphone back through the speakers, processed exactly as a call sends it:
/// gain, noise suppression, the voice gate and Opus. Dropping it stops the test.
pub struct MicTest {
    engine: AudioEngine,
}

impl std::fmt::Debug for MicTest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MicTest").finish_non_exhaustive()
    }
}

impl MicTest {
    pub fn start(
        settings: AudioSettings,
    ) -> std::io::Result<(Self, async_channel::Receiver<AudioEvent>)> {
        let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, event_rx) = async_channel::unbounded();
        let engine = AudioEngine::start(settings, out_tx, event_tx)?;
        let sink = engine.sink();
        std::thread::Builder::new()
            .name("rcord-mic-test".into())
            .spawn(move || {
                let mut sequence = 0u16;
                while let Some(out) = out_rx.blocking_recv() {
                    if let Outbound::Opus(opus) = out {
                        let packet = Inbound {
                            user_id: 0,
                            stream: false,
                            ssrc: 1,
                            sequence,
                            opus,
                        };
                        if sink.send(packet).is_err() {
                            break;
                        }
                        sequence = sequence.wrapping_add(1);
                    }
                }
            })?;
        Ok((Self { engine }, event_rx))
    }

    pub fn update_settings(&self, settings: AudioSettings) {
        self.engine.update_settings(settings);
    }
}

type Queue = Arc<Mutex<VecDeque<f32>>>;

/// The open devices. Streams are not `Send` on every platform, so they live and die on the
/// media thread.
struct Devices {
    _input: Option<cpal::Stream>,
    _output: Option<cpal::Stream>,
    capture: Queue,
    playback: Queue,
    in_rate: u32,
    in_channels: usize,
    out_rate: u32,
    out_channels: usize,
    lost: Arc<AtomicBool>,
}

fn find_device(want: Option<&str>, input: bool) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = want {
        let found = if input {
            host.input_devices().ok().and_then(|mut it| {
                it.find(|d| d.description().is_ok_and(|desc| desc.name() == name))
            })
        } else {
            host.output_devices().ok().and_then(|mut it| {
                it.find(|d| d.description().is_ok_and(|desc| desc.name() == name))
            })
        };
        if found.is_some() {
            return found;
        }
        tracing::warn!("audio device {name:?} not found, using the default");
    }
    if input {
        host.default_input_device()
    } else {
        host.default_output_device()
    }
}

/// Picks the device's default format, at 48 kHz when the device offers it.
fn pick_config(
    default: cpal::SupportedStreamConfig,
    ranges: impl Iterator<Item = cpal::SupportedStreamConfigRange>,
) -> cpal::SupportedStreamConfig {
    if default.sample_rate() == SAMPLE_RATE {
        return default;
    }
    ranges
        .filter(|r| {
            r.sample_format() == default.sample_format() && r.channels() == default.channels()
        })
        .find_map(|r| r.try_with_sample_rate(SAMPLE_RATE))
        .unwrap_or(default)
}

impl Devices {
    fn open(settings: &AudioSettings) -> (Self, Vec<String>) {
        let mut errors = Vec::new();
        let capture: Queue = Arc::default();
        let playback: Queue = Arc::default();
        let lost = Arc::new(AtomicBool::new(false));
        let mut dev = Self {
            _input: None,
            _output: None,
            capture: capture.clone(),
            playback: playback.clone(),
            in_rate: SAMPLE_RATE,
            in_channels: 1,
            out_rate: SAMPLE_RATE,
            out_channels: 2,
            lost: lost.clone(),
        };

        match find_device(settings.input_device.as_deref(), true) {
            Some(d) => match d.default_input_config().map(|def| {
                let ranges = d.supported_input_configs().into_iter().flatten();
                pick_config(def, ranges)
            }) {
                Ok(cfg) => {
                    dev.in_rate = cfg.sample_rate();
                    dev.in_channels = usize::from(cfg.channels());
                    match build_input(&d, cfg.config(), cfg.sample_format(), capture, lost.clone())
                    {
                        Ok(s) => {
                            if let Err(e) = s.play() {
                                errors.push(format!("Microphone could not start: {e}"));
                            }
                            dev._input = Some(s);
                        }
                        Err(e) => errors.push(format!("Microphone could not open: {e}")),
                    }
                }
                Err(e) => errors.push(format!("Microphone could not open: {e}")),
            },
            None => errors.push("No microphone found.".into()),
        }

        match find_device(settings.output_device.as_deref(), false) {
            Some(d) => match d.default_output_config().map(|def| {
                let ranges = d.supported_output_configs().into_iter().flatten();
                pick_config(def, ranges)
            }) {
                Ok(cfg) => {
                    dev.out_rate = cfg.sample_rate();
                    dev.out_channels = usize::from(cfg.channels());
                    match build_output(&d, cfg.config(), cfg.sample_format(), playback, lost) {
                        Ok(s) => {
                            if let Err(e) = s.play() {
                                errors.push(format!("Speakers could not start: {e}"));
                            }
                            dev._output = Some(s);
                        }
                        Err(e) => errors.push(format!("Speakers could not open: {e}")),
                    }
                }
                Err(e) => errors.push(format!("Speakers could not open: {e}")),
            },
            None => errors.push("No speakers or headphones found.".into()),
        }
        (dev, errors)
    }
}

fn build_input(
    device: &cpal::Device,
    config: StreamConfig,
    format: SampleFormat,
    queue: Queue,
    lost: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    fn build<T: SizedSample>(
        device: &cpal::Device,
        config: StreamConfig,
        queue: Queue,
        lost: Arc<AtomicBool>,
    ) -> Result<cpal::Stream, cpal::Error>
    where
        f32: FromSample<T>,
    {
        // About half a second at 48 kHz stereo; more means the media loop stalled.
        let cap = 48_000;
        device.build_input_stream::<T, _, _>(
            config,
            move |data: &[T], _| {
                let mut q = queue.lock();
                q.extend(data.iter().map(|s| s.to_sample::<f32>()));
                let excess = q.len().saturating_sub(cap);
                q.drain(..excess);
            },
            move |e| {
                tracing::warn!("microphone stream error: {e}");
                lost.store(true, Ordering::Relaxed);
            },
            None,
        )
    }
    match format {
        SampleFormat::F32 => build::<f32>(device, config, queue, lost),
        SampleFormat::I16 => build::<i16>(device, config, queue, lost),
        SampleFormat::I32 => build::<i32>(device, config, queue, lost),
        SampleFormat::U16 => build::<u16>(device, config, queue, lost),
        SampleFormat::U8 => build::<u8>(device, config, queue, lost),
        SampleFormat::I8 => build::<i8>(device, config, queue, lost),
        SampleFormat::F64 => build::<f64>(device, config, queue, lost),
        other => return Err(format!("unsupported sample format {other}")),
    }
    .map_err(|e| e.to_string())
}

fn build_output(
    device: &cpal::Device,
    config: StreamConfig,
    format: SampleFormat,
    queue: Queue,
    lost: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    fn build<T: SizedSample + FromSample<f32>>(
        device: &cpal::Device,
        config: StreamConfig,
        queue: Queue,
        lost: Arc<AtomicBool>,
    ) -> Result<cpal::Stream, cpal::Error> {
        device.build_output_stream::<T, _, _>(
            config,
            move |out: &mut [T], _| {
                let mut q = queue.lock();
                for s in out.iter_mut() {
                    *s = T::from_sample(q.pop_front().unwrap_or(0.0));
                }
            },
            move |e| {
                tracing::warn!("output stream error: {e}");
                lost.store(true, Ordering::Relaxed);
            },
            None,
        )
    }
    match format {
        SampleFormat::F32 => build::<f32>(device, config, queue, lost),
        SampleFormat::I16 => build::<i16>(device, config, queue, lost),
        SampleFormat::I32 => build::<i32>(device, config, queue, lost),
        SampleFormat::U16 => build::<u16>(device, config, queue, lost),
        SampleFormat::U8 => build::<u8>(device, config, queue, lost),
        SampleFormat::I8 => build::<i8>(device, config, queue, lost),
        SampleFormat::F64 => build::<f64>(device, config, queue, lost),
        other => return Err(format!("unsupported sample format {other}")),
    }
    .map_err(|e| e.to_string())
}

/// Decides when the microphone transmits.
#[derive(Debug)]
pub struct Gate {
    noise_floor_db: f32,
    hold: u32,
    open: bool,
}

/// Frames the gate stays open after the voice drops below the threshold.
const HANGOVER_FRAMES: u32 = 15;

impl Default for Gate {
    fn default() -> Self {
        Self {
            noise_floor_db: -60.0,
            hold: 0,
            open: false,
        }
    }
}

impl Gate {
    /// Feeds one frame's level and RNNoise's voice probability (if it ran) and returns
    /// whether to transmit it.
    pub fn update(
        &mut self,
        level_db: f32,
        voice_probability: Option<f32>,
        transmit: Transmit,
        push_to_talk: bool,
    ) -> bool {
        // Track the noise floor: fall quickly, rise slowly.
        if level_db < self.noise_floor_db {
            self.noise_floor_db = level_db.max(-90.0);
        } else {
            self.noise_floor_db += 0.02;
        }
        let (active, hangover) = match transmit {
            Transmit::PushToTalk { release_ms } => (push_to_talk, release_ms / 20),
            Transmit::VoiceActivity {
                threshold_db: Some(t),
            } => (level_db > t, HANGOVER_FRAMES),
            Transmit::VoiceActivity { threshold_db: None } => {
                let loud = level_db > (self.noise_floor_db + 15.0).max(-55.0);
                let voiced = voice_probability.is_none_or(|p| p > 0.5);
                (loud && voiced, HANGOVER_FRAMES)
            }
        };
        if active {
            self.hold = hangover;
            self.open = true;
        } else if self.hold > 0 {
            self.hold -= 1;
        } else {
            self.open = false;
        }
        self.open
    }
}

/// Automatic gain control: moves the microphone's speech level toward -20 dBFS, rising
/// slowly and falling fast, and only learns from frames that contain a voice, so silence is
/// never pumped up.
#[derive(Debug)]
pub struct Agc {
    gain_db: f32,
}

impl Default for Agc {
    fn default() -> Self {
        Self { gain_db: 0.0 }
    }
}

impl Agc {
    const TARGET_DB: f32 = -20.0;
    const MAX_GAIN_DB: f32 = 18.0;
    const MIN_GAIN_DB: f32 = -12.0;

    pub fn gain_db(&self) -> f32 {
        self.gain_db
    }

    pub fn process(&mut self, frame: &mut [f32], voice_probability: Option<f32>) {
        let before = level_db(frame);
        let voiced = voice_probability.map_or(before > -50.0, |p| p > 0.6);
        if voiced {
            let error = Self::TARGET_DB - (before + self.gain_db);
            let step = if error > 0.0 { 0.15 } else { 1.5 };
            self.gain_db = (self.gain_db + error.clamp(-step, step))
                .clamp(Self::MIN_GAIN_DB, Self::MAX_GAIN_DB);
        }
        let g = 10f32.powf(self.gain_db / 20.0);
        for s in frame.iter_mut() {
            *s = soft_clip(*s * g);
        }
    }
}

pub fn level_db(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return -100.0;
    }
    let mean = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    (10.0 * mean.max(1e-10).log10()).max(-100.0)
}

/// A gentle limiter: linear up to 0.8, then curves into ±1 instead of clipping.
fn soft_clip(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.8 {
        x
    } else {
        x.signum() * (0.8 + 0.2 * ((a - 0.8) / 0.2).tanh())
    }
}

struct Speaker {
    user_id: u64,
    stream: bool,
    jitter: JitterBuffer,
    decoder: opus::Decoder,
    speaking: bool,
    quiet_frames: u32,
    last_heard: Instant,
}

struct MediaLoop {
    shared: Arc<Shared>,
    inbound: mpsc::Receiver<Inbound>,
    outbound: tokio::sync::mpsc::UnboundedSender<Outbound>,
    events: async_channel::Sender<AudioEvent>,
    settings: AudioSettings,
    encoder: Option<opus::Encoder>,
    denoise: Option<Box<nnnoiseless::DenoiseState<'static>>>,
    gate: Gate,
    agc: Agc,
    strong_gain: f32,
    transmitting: bool,
    silence_left: u8,
    level_frames: u32,
    mic_resampler: Resampler,
    mic: Vec<f32>,
    out_resampler: Resampler,
    speakers: HashMap<u32, Speaker>,
}

impl MediaLoop {
    fn new(
        shared: Arc<Shared>,
        inbound: mpsc::Receiver<Inbound>,
        outbound: tokio::sync::mpsc::UnboundedSender<Outbound>,
        events: async_channel::Sender<AudioEvent>,
    ) -> Self {
        let settings = shared.settings.lock().clone();
        Self {
            shared,
            inbound,
            outbound,
            events,
            settings,
            encoder: None,
            denoise: None,
            gate: Gate::default(),
            agc: Agc::default(),
            strong_gain: 1.0,
            transmitting: false,
            silence_left: 0,
            level_frames: 0,
            mic_resampler: Resampler::new(SAMPLE_RATE, SAMPLE_RATE, 1),
            mic: Vec::with_capacity(FRAME * 4),
            out_resampler: Resampler::new(SAMPLE_RATE, SAMPLE_RATE, 2),
            speakers: HashMap::new(),
        }
    }

    fn make_encoder(&self) -> Option<opus::Encoder> {
        let mut enc =
            match opus::Encoder::new(SAMPLE_RATE, opus::Channels::Mono, opus::Application::Voip) {
                Ok(e) => e,
                Err(e) => {
                    tracing::error!("Opus encoder: {e}");
                    return None;
                }
            };
        let bits = self.settings.bitrate_kbps.clamp(8, 510) as i32 * 1000;
        let _ = enc.set_bitrate(opus::Bitrate::Bits(bits));
        let _ = enc.set_inband_fec(true);
        let _ = enc.set_packet_loss_perc(10);
        let _ = enc.set_complexity(10);
        let _ = enc.set_signal(opus::Signal::Voice);
        Some(enc)
    }

    fn open_devices(&mut self) -> Devices {
        let (dev, errors) = Devices::open(&self.settings);
        for e in errors {
            tracing::warn!("{e}");
            let _ = self.events.try_send(AudioEvent::DeviceError(e));
        }
        self.mic_resampler = Resampler::new(dev.in_rate, SAMPLE_RATE, 1);
        self.out_resampler = Resampler::new(SAMPLE_RATE, dev.out_rate, 2);
        self.mic.clear();
        dev
    }

    fn apply_settings(&mut self) {
        self.settings = self.shared.settings.lock().clone();
        self.encoder = self.make_encoder();
        self.denoise =
            (self.settings.noise_suppression != Denoise::Off).then(nnnoiseless::DenoiseState::new);
    }

    fn run(mut self) {
        self.apply_settings();
        let mut dev = self.open_devices();
        let mut device_retry: Option<Instant> = None;
        let tick = Duration::from_millis(5);
        while !self.shared.stop.load(Ordering::Relaxed) {
            if self.shared.settings_changed.swap(false, Ordering::Relaxed) {
                self.apply_settings();
            }
            let lost = dev.lost.load(Ordering::Relaxed);
            if self.shared.devices_changed.swap(false, Ordering::Relaxed)
                || (lost && device_retry.is_none_or(|t| t.elapsed() > Duration::from_secs(2)))
            {
                drop(dev);
                dev = self.open_devices();
                device_retry = Some(Instant::now());
            }

            while let Ok(p) = self.inbound.try_recv() {
                self.receive(p);
            }
            self.capture(&dev);
            self.playback(&dev);
            std::thread::sleep(tick);
        }
    }

    fn receive(&mut self, p: Inbound) {
        let speaker = match self.speakers.entry(p.ssrc) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let Ok(decoder) = opus::Decoder::new(SAMPLE_RATE, opus::Channels::Stereo) else {
                    return;
                };
                e.insert(Speaker {
                    user_id: p.user_id,
                    stream: p.stream,
                    jitter: JitterBuffer::default(),
                    decoder,
                    speaking: false,
                    quiet_frames: 0,
                    last_heard: Instant::now(),
                })
            }
        };
        speaker.user_id = p.user_id;
        speaker.last_heard = Instant::now();
        speaker.jitter.push(p.sequence, p.opus);
    }

    fn capture(&mut self, dev: &Devices) {
        let raw: Vec<f32> = {
            let mut q = dev.capture.lock();
            let whole = q.len() - q.len() % dev.in_channels.max(1);
            q.drain(..whole).collect()
        };
        if !raw.is_empty() {
            let ch = dev.in_channels.max(1);
            let mono: Vec<f32> = raw
                .chunks_exact(ch)
                .map(|f| f.iter().sum::<f32>() / ch as f32)
                .collect();
            self.mic_resampler.process(&mono, &mut self.mic);
        }
        let muted = self.shared.muted.load(Ordering::Relaxed)
            || self.shared.deafened.load(Ordering::Relaxed);
        while self.mic.len() >= FRAME {
            let mut frame: Vec<f32> = self.mic.drain(..FRAME).collect();
            let gain = self.settings.input_gain;
            for s in &mut frame {
                *s *= gain;
            }
            let mut vad = None;
            if let Some(ns) = &mut self.denoise {
                let mut p = 0.0f32;
                let mut out = [0.0f32; nnnoiseless::FRAME_SIZE];
                let mut scaled = [0.0f32; nnnoiseless::FRAME_SIZE];
                for half in frame.as_chunks_mut::<{ nnnoiseless::FRAME_SIZE }>().0 {
                    for (d, s) in scaled.iter_mut().zip(half.iter()) {
                        *d = s * 32_768.0;
                    }
                    p = p.max(ns.process_frame(&mut out, &scaled));
                    for (d, s) in half.iter_mut().zip(out.iter()) {
                        *d = s / 32_768.0;
                    }
                }
                vad = Some(p);
                if self.settings.noise_suppression == Denoise::Strong {
                    // Ease toward the target gain so words are not chopped.
                    let target = ((p - 0.3) / 0.4).clamp(0.05, 1.0);
                    for s in &mut frame {
                        self.strong_gain += (target - self.strong_gain) * 0.02;
                        *s *= self.strong_gain;
                    }
                }
            }
            if self.settings.automatic_gain {
                self.agc.process(&mut frame, vad);
            }
            let level = level_db(&frame);
            self.level_frames += 1;
            if self.level_frames >= 5 {
                self.level_frames = 0;
                let _ = self.events.try_send(AudioEvent::InputLevel(level));
            }
            let open = self.gate.update(
                level,
                vad,
                self.settings.transmit,
                self.shared.push_to_talk.load(Ordering::Relaxed),
            ) && !muted;
            self.send_frame(open, &frame);
        }
    }

    fn send_frame(&mut self, open: bool, frame: &[f32]) {
        if open != self.transmitting {
            self.transmitting = open;
            if open {
                let _ = self.outbound.send(Outbound::Speaking(true));
            } else {
                self.silence_left = 5;
            }
            let _ = self.events.try_send(AudioEvent::Speaking {
                user_id: 0,
                speaking: open,
            });
        }
        if open {
            let mut buf = [0u8; 1275];
            if let Some(enc) = &mut self.encoder
                && let Ok(n) = enc.encode_float(frame, &mut buf)
            {
                let _ = self.outbound.send(Outbound::Opus(buf[..n].to_vec()));
            }
        } else if self.silence_left > 0 {
            self.silence_left -= 1;
            let _ = self.outbound.send(Outbound::Opus(SILENCE.to_vec()));
            if self.silence_left == 0 {
                let _ = self.outbound.send(Outbound::Speaking(false));
            }
        }
    }

    fn playback(&mut self, dev: &Devices) {
        let deaf = self.shared.deafened.load(Ordering::Relaxed);
        // Keep about 60 ms queued at the device; produce frames until it is there.
        let target = dev.out_rate as usize * dev.out_channels * 60 / 1000;
        let mut produced = 0;
        while dev.playback.lock().len() < target && produced < 4 {
            produced += 1;
            let mixed = self.mix(deaf);
            let mut out = Vec::with_capacity(FRAME * 2 + 16);
            self.out_resampler.process(&mixed, &mut out);
            let gain = self.settings.output_gain;
            let mut q = dev.playback.lock();
            match dev.out_channels {
                2 => q.extend(out.iter().map(|s| soft_clip(s * gain))),
                n => {
                    for f in out.as_chunks::<2>().0 {
                        let m = soft_clip((f[0] + f[1]) * 0.5 * gain);
                        let (l, r) = (soft_clip(f[0] * gain), soft_clip(f[1] * gain));
                        for c in 0..n {
                            q.push_back(match (n, c) {
                                (1, _) => m,
                                (_, 0) => l,
                                (_, 1) => r,
                                _ => 0.0,
                            });
                        }
                    }
                }
            }
        }
        // Forget speakers silent for a while; a new SSRC gets a fresh decoder.
        self.speakers
            .retain(|_, s| s.last_heard.elapsed() < Duration::from_secs(60));
    }

    /// One 20 ms stereo frame of everyone speaking.
    fn mix(&mut self, deaf: bool) -> Vec<f32> {
        let mut mixed = vec![0.0f32; FRAME * 2];
        let mut pcm = vec![0.0f32; FRAME * 2];
        let volumes = self.shared.volumes.lock().clone();
        let stream_volumes = self.shared.stream_volumes.lock().clone();
        for speaker in self.speakers.values_mut() {
            let n = match speaker.jitter.pop() {
                Playout::Frame(packet) => speaker.decoder.decode_float(&packet, &mut pcm, false),
                Playout::Lost(Some(next)) => speaker.decoder.decode_float(&next, &mut pcm, true),
                Playout::Lost(None) => speaker.decoder.decode_float(&[], &mut pcm, false),
                Playout::Idle => Ok(0),
            }
            .unwrap_or(0);
            let samples = &pcm[..(n * 2).min(pcm.len())];
            let loud = n > 0 && level_db(samples) > -50.0;
            if loud {
                speaker.quiet_frames = 0;
            } else {
                speaker.quiet_frames += 1;
            }
            let speaking = !speaker.stream
                && (loud || (speaker.speaking && speaker.quiet_frames < HANGOVER_FRAMES));
            if speaking != speaker.speaking {
                speaker.speaking = speaking;
                let _ = self.events.try_send(AudioEvent::Speaking {
                    user_id: speaker.user_id,
                    speaking,
                });
            }
            if deaf || n == 0 {
                continue;
            }
            let v = if speaker.stream {
                &stream_volumes
            } else {
                &volumes
            }
            .get(&speaker.user_id)
            .copied()
            .unwrap_or(1.0);
            for (m, s) in mixed.iter_mut().zip(samples) {
                *m += s * v;
            }
        }
        mixed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels() {
        assert_eq!(level_db(&[]), -100.0);
        assert!((level_db(&[1.0, -1.0]) - 0.0).abs() < 1e-4);
        assert!((level_db(&[0.1; 10]) + 20.0).abs() < 1e-3);
    }

    #[test]
    fn limiter_never_exceeds_full_scale() {
        for x in [-4.0, -1.0, -0.5, 0.0, 0.79, 0.9, 1.5, 10.0] {
            let y = soft_clip(x);
            assert!(y.abs() <= 1.0);
            assert_eq!(y.signum(), if x == 0.0 { 1.0 } else { x.signum() });
        }
        assert_eq!(soft_clip(0.5), 0.5);
    }

    #[test]
    fn gate_opens_on_voice_and_holds() {
        let mut g = Gate::default();
        let vad = Transmit::VoiceActivity { threshold_db: None };
        for _ in 0..50 {
            assert!(!g.update(-70.0, Some(0.0), vad, false));
        }
        assert!(g.update(-20.0, Some(0.9), vad, false));
        for _ in 0..HANGOVER_FRAMES {
            assert!(g.update(-70.0, Some(0.0), vad, false));
        }
        assert!(!g.update(-70.0, Some(0.0), vad, false));
        // Loud but not voice, according to RNNoise.
        assert!(!g.update(-20.0, Some(0.1), vad, false));
    }

    #[test]
    fn agc_raises_quiet_speech_and_ignores_silence() {
        let mut agc = Agc::default();
        let quiet: Vec<f32> = (0..FRAME).map(|n| 0.01 * (n as f32 * 0.05).sin()).collect();
        for _ in 0..500 {
            let mut f = quiet.clone();
            agc.process(&mut f, Some(0.9));
        }
        assert!(agc.gain_db() > 15.0, "{}", agc.gain_db());
        let mut silent = Agc::default();
        for _ in 0..500 {
            let mut f = quiet.clone();
            silent.process(&mut f, Some(0.0));
        }
        assert_eq!(silent.gain_db(), 0.0);
        // Loud speech is brought down quickly.
        let loud: Vec<f32> = (0..FRAME).map(|n| 0.9 * (n as f32 * 0.05).sin()).collect();
        let mut down = Agc::default();
        for _ in 0..10 {
            let mut f = loud.clone();
            down.process(&mut f, Some(0.9));
        }
        assert!(down.gain_db() < -8.0, "{}", down.gain_db());
    }

    #[test]
    fn gate_manual_threshold_and_push_to_talk() {
        let mut g = Gate::default();
        let manual = Transmit::VoiceActivity {
            threshold_db: Some(-30.0),
        };
        assert!(!g.update(-35.0, None, manual, false));
        assert!(g.update(-25.0, None, manual, false));

        let mut g = Gate::default();
        let ptt = Transmit::PushToTalk { release_ms: 40 };
        assert!(!g.update(-10.0, None, ptt, false));
        assert!(g.update(-90.0, None, ptt, true));
        assert!(g.update(-90.0, None, ptt, false));
        assert!(g.update(-90.0, None, ptt, false));
        assert!(!g.update(-90.0, None, ptt, false));
    }

    #[test]
    fn opus_round_trip_with_concealment() {
        let mut enc =
            opus::Encoder::new(SAMPLE_RATE, opus::Channels::Mono, opus::Application::Voip).unwrap();
        enc.set_inband_fec(true).unwrap();
        let mut dec = opus::Decoder::new(SAMPLE_RATE, opus::Channels::Stereo).unwrap();
        let tone: Vec<f32> = (0..FRAME)
            .map(|n| 0.3 * (n as f32 * 300.0 * std::f32::consts::TAU / 48_000.0).sin())
            .collect();
        let mut pcm = vec![0.0f32; FRAME * 2];
        let mut buf = [0u8; 1275];
        for _ in 0..5 {
            let n = enc.encode_float(&tone, &mut buf).unwrap();
            assert_eq!(dec.decode_float(&buf[..n], &mut pcm, false).unwrap(), FRAME);
        }
        assert!(level_db(&pcm) > -20.0);
        assert_eq!(dec.decode_float(&[], &mut pcm, false).unwrap(), FRAME);
        assert_eq!(dec.decode_float(&SILENCE, &mut pcm, false).unwrap(), FRAME);
    }
}
