//! Go Live from the app's side: sharing this client's screen into its voice channel, and
//! watching someone else's stream.
//!
//! Both start on the main gateway. Sharing captures first (on Wayland the desktop asks
//! which screen or window), then asks Discord to create the stream; watching asks to watch
//! it. Discord answers with the stream's media server and a token, and the stream connection
//! starts on the network runtime. A captured frame goes from the capture thread straight to
//! the encoder; a decoded frame comes back to the main thread as an image to draw.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui::{AppContext, Context, RenderImage, Task};
use rcord_capture::{Capture, PixelFormat, SourceId};
use rcord_gateway::{Command, StreamCreate, StreamKey};
use rcord_model::Snowflake;
use rcord_store::Update;
use rcord_voice::{
    ConnectionState, StreamConfig, StreamHandle, StreamRole, VideoQuality, VoiceEvent,
};

use crate::net::Net;
use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamStatus {
    /// Waiting for Discord to assign a stream server.
    Starting,
    Connecting,
    Live,
    Reconnecting,
}

/// Stream quality, chosen in the Go Live dialog the way Discord offers it: a resolution and
/// a frame rate. The dialog starts from the screen share settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamPreset {
    /// Vertical resolution; 0 streams at the source's resolution.
    pub height: u32,
    pub fps: u32,
}

impl StreamPreset {
    pub const RESOLUTIONS: [(&'static str, u32); 4] = [
        ("720p", 720),
        ("1080p", 1080),
        ("1440p", 1440),
        ("Source", 0),
    ];
    pub const FRAME_RATES: [u32; 3] = [15, 30, 60];

    pub fn from_settings(s: &rcord_settings::Stream) -> Self {
        let height = Self::RESOLUTIONS
            .iter()
            .map(|r| r.1)
            .find(|h| *h == s.resolution)
            .unwrap_or(720);
        let fps = Self::FRAME_RATES
            .into_iter()
            .find(|f| *f == s.fps)
            .unwrap_or(30);
        Self { height, fps }
    }

    pub fn label(self) -> String {
        let res = match self.height {
            0 => "Source".to_owned(),
            h => format!("{h}p"),
        };
        format!("{res} · {} FPS", self.fps)
    }

    pub fn quality(self) -> VideoQuality {
        // "Source" still caps at 4K, which is as large as Discord streams.
        let (max_width, max_height) = match self.height {
            0 => (3840, 2160),
            h => (h * 16 / 9, h),
        };
        // Bits for each pixel and frame, tuned for screen content: about 2.5 Mbps at 720p30,
        // 5 Mbps at 1080p30, and capped at 8 Mbps.
        let pixels = u64::from(max_width) * u64::from(max_height) * u64::from(self.fps);
        let bitrate_kbps = (pixels * 9 / 100_000).clamp(1000, 8000) as u32;
        VideoQuality {
            max_width,
            max_height,
            fps: self.fps,
            bitrate_kbps,
        }
    }
}

/// The stream key Discord uses for `user`'s stream in a channel.
pub fn stream_key(guild: Option<Snowflake>, channel: Snowflake, user: Snowflake) -> String {
    match guild {
        Some(g) => format!("guild:{g}:{channel}:{user}"),
        None => format!("call:{channel}:{user}"),
    }
}

/// Connection details that arrive in pieces from the main gateway.
#[derive(Default)]
struct Pending {
    rtc_server_id: Option<u64>,
    server: Option<(String, String)>,
}

/// This client's own stream.
pub struct GoLive {
    pub key: String,
    pub status: StreamStatus,
    pub preset: StreamPreset,
    pub viewers: Vec<Snowflake>,
    pending: Pending,
    capture: Option<Capture>,
    handle: Option<Arc<StreamHandle>>,
    pump_stop: Arc<AtomicBool>,
    _events: Option<Task<()>>,
}

impl Drop for GoLive {
    fn drop(&mut self) {
        self.pump_stop.store(true, Ordering::Relaxed);
    }
}

/// A stream this client is watching.
pub struct Watching {
    pub key: String,
    pub streamer: Snowflake,
    pub status: StreamStatus,
    pub viewers: Vec<Snowflake>,
    /// The newest picture, ready to draw.
    pub frame: Option<Arc<RenderImage>>,
    /// The picture's size and the frames shown in the last second, for the quality badge.
    pub size: (u32, u32),
    pub fps: u32,
    frames: u32,
    counted_since: std::time::Instant,
    pending: Pending,
    handle: Option<StreamHandle>,
    _events: Option<Task<()>>,
}

impl AppState {
    /// Starts sharing `source` in the current call. Capture starts first, since on Wayland
    /// the desktop asks the user what to share and they may cancel.
    pub fn start_go_live(
        &mut self,
        source: SourceId,
        preset: StreamPreset,
        cx: &mut Context<Self>,
    ) {
        let Some(call) = &self.call else {
            self.toast("Join a voice channel to share your screen.", cx);
            return;
        };
        let (guild, channel) = (call.guild, call.channel);
        let Some(me) = self.store.me_id() else {
            return;
        };
        self.stop_go_live(cx);
        let fps = preset.fps;
        let cursor = self.settings.stream.capture_cursor;
        // The dialog's choice becomes the default for the next stream.
        self.update_settings(cx, |s| {
            s.stream.resolution = preset.height;
            s.stream.fps = preset.fps;
        });
        let started = cx.background_spawn(async move { Capture::start(&source, fps, cursor) });
        cx.spawn(async move |this, cx| {
            let capture = started.await;
            let _ = this.update(cx, |state, cx| {
                let capture = match capture {
                    Ok(c) => c,
                    Err(rcord_capture::CaptureError::Cancelled) => return,
                    Err(e) => {
                        state.toast(format!("Screen share could not start: {e}"), cx);
                        return;
                    }
                };
                // The call may have ended while the picker was open.
                if state.call.as_ref().is_none_or(|c| c.channel != channel) {
                    return;
                }
                let key = stream_key(guild, channel, me);
                state.go_live = Some(GoLive {
                    key,
                    status: StreamStatus::Starting,
                    preset,
                    viewers: Vec::new(),
                    pending: Pending::default(),
                    capture: Some(capture),
                    handle: None,
                    pump_stop: Arc::new(AtomicBool::new(false)),
                    _events: None,
                });
                if let Some(gw) = &state.gateway {
                    gw.send(Command::StreamCreate(StreamCreate {
                        kind: if guild.is_some() { "guild" } else { "call" },
                        guild_id: guild,
                        channel_id: channel,
                        preferred_region: None,
                    }));
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn stop_go_live(&mut self, cx: &mut Context<Self>) {
        let Some(live) = self.go_live.take() else {
            return;
        };
        if let Some(gw) = &self.gateway {
            gw.send(Command::StreamDelete(StreamKey {
                stream_key: live.key.clone(),
            }));
        }
        cx.notify();
    }

    /// Watches `streamer`'s stream in the current call's channel, joining it first if
    /// needed.
    pub fn watch_stream(
        &mut self,
        channel: Snowflake,
        streamer: Snowflake,
        cx: &mut Context<Self>,
    ) {
        if Some(streamer) == self.store.me_id() {
            return;
        }
        let guild = self.store.channel(channel).and_then(|c| c.guild_id);
        let key = stream_key(guild, channel, streamer);
        if self.watching.iter().any(|w| w.key == key) {
            self.focused_stream = Some(streamer);
            cx.notify();
            return;
        }
        if self.call.as_ref().is_none_or(|c| c.channel != channel) {
            // Streams belong to a channel; joining another one ends the ones watched here.
            self.stop_all_watching(cx);
            self.join_voice(channel, cx);
        }
        self.focused_stream = Some(streamer);
        self.watching.push(Watching {
            key: key.clone(),
            streamer,
            status: StreamStatus::Starting,
            viewers: Vec::new(),
            frame: None,
            size: (0, 0),
            fps: 0,
            frames: 0,
            counted_since: std::time::Instant::now(),
            pending: Pending::default(),
            handle: None,
            _events: None,
        });
        if let Some(gw) = &self.gateway {
            gw.send(Command::StreamWatch(StreamKey { stream_key: key }));
        }
        cx.notify();
    }

    /// Sample streams for `RCORD_DEMO=streams`: tiles with generated pictures, so the stream
    /// views can be seen and screenshotted without a call.
    pub(crate) fn demo_streams(&mut self, cx: &mut Context<Self>) {
        let people = crate::demo::streamers();
        for (i, (streamer, hue)) in people.into_iter().enumerate() {
            let (w, h) = (1280u32, 720u32);
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                for x in 0..w {
                    let fx = x as f32 / w as f32;
                    let fy = y as f32 / h as f32;
                    let band = ((fx * 8.0).floor() as u32 + (fy * 6.0).floor() as u32) % 2;
                    let shade = 0.55 + 0.35 * (1.0 - fy) + 0.05 * band as f32;
                    let [r, g, b] = hue;
                    let px = |c: u8| (c as f32 * shade).min(255.0) as u8;
                    rgba.extend_from_slice(&[px(b), px(g), px(r), 255]);
                }
            }
            let Some(buffer) = image::RgbaImage::from_raw(w, h, rgba) else {
                continue;
            };
            self.watching.push(Watching {
                key: format!("demo:{i}"),
                streamer,
                status: StreamStatus::Live,
                viewers: vec![streamer; i + 2],
                frame: Some(Arc::new(RenderImage::new([image::Frame::new(buffer)]))),
                size: (w, h),
                fps: [60, 30, 60][i % 3],
                frames: 0,
                counted_since: std::time::Instant::now(),
                pending: Pending::default(),
                handle: None,
                _events: None,
            });
        }
        self.focused_stream = self.watching.first().map(|w| w.streamer);
        cx.notify();
    }

    /// Opens the card for `streamer`'s stream in `channel`, fetching its thumbnail.
    pub fn open_stream_preview(
        &mut self,
        channel: Snowflake,
        streamer: Snowflake,
        at: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.set_overlay(
            Some(crate::state::Overlay::StreamPreview {
                channel,
                streamer,
                at,
            }),
            cx,
        );
        let guild = self.store.channel(channel).and_then(|c| c.guild_id);
        let key = stream_key(guild, channel, streamer);
        // Discord refreshes thumbnails every few minutes, so a recent one is current.
        if self
            .stream_previews
            .get(&key)
            .is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(60))
        {
            return;
        }
        let Some(rest) = cx.global::<Net>().rest() else {
            return;
        };
        let fetch_key = key.clone();
        let task = cx
            .global::<Net>()
            .run(async move { rest.stream_preview(&fetch_key).await });
        cx.spawn(async move |this, cx| {
            let url = task.await.and_then(Result::ok).flatten();
            this.update(cx, |s, cx| {
                s.stream_previews
                    .insert(key, (url, std::time::Instant::now()));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Stops watching `streamer`'s stream.
    pub fn stop_watching(&mut self, streamer: Snowflake, cx: &mut Context<Self>) {
        let Some(i) = self.watching.iter().position(|w| w.streamer == streamer) else {
            return;
        };
        let w = self.watching.remove(i);
        self.close_watch(w, true, cx);
    }

    pub fn stop_all_watching(&mut self, cx: &mut Context<Self>) {
        for w in std::mem::take(&mut self.watching) {
            self.close_watch(w, true, cx);
        }
    }

    pub fn is_watching(&self, streamer: Snowflake) -> bool {
        self.watching.iter().any(|w| w.streamer == streamer)
    }

    /// The stream drawn large: the focused one while it is watched, else the first.
    pub fn focused_watch(&self) -> Option<&Watching> {
        self.watching
            .iter()
            .find(|w| Some(w.streamer) == self.focused_stream)
            .or_else(|| self.watching.first())
    }

    /// How loud `streamer`'s stream plays, 0 to 2.
    pub fn stream_volume(&self, streamer: Snowflake) -> f32 {
        self.settings
            .voice
            .stream_volumes
            .get(&streamer.to_string())
            .copied()
            .unwrap_or(1.0)
    }

    pub fn set_stream_volume(&mut self, streamer: Snowflake, volume: f32, cx: &mut Context<Self>) {
        let volume = volume.clamp(0.0, 2.0);
        let id = streamer.to_string();
        let v = &mut self.settings.voice.stream_volumes;
        if (volume - 1.0).abs() > 0.005 {
            v.insert(id, volume);
        } else {
            v.remove(&id);
        }
        if let Some(h) = self.call.as_ref().and_then(|c| c.voice_handle()) {
            h.set_stream_volume(streamer.get(), volume);
        }
        self.save_settings(cx);
        cx.notify();
    }

    /// Ends a watch, telling Discord unless the stream is already gone.
    fn close_watch(&mut self, mut w: Watching, tell: bool, cx: &mut Context<Self>) {
        if tell && let Some(gw) = &self.gateway {
            gw.send(Command::StreamDelete(StreamKey {
                stream_key: w.key.clone(),
            }));
        }
        if let Some(old) = w.frame.take() {
            cx.drop_image(old, None);
        }
        if self.focused_stream == Some(w.streamer) {
            self.focused_stream = self.watching.first().map(|w| w.streamer);
        }
        if self.watching.is_empty()
            && matches!(self.overlay, Some(crate::state::Overlay::StreamFullscreen))
        {
            self.set_overlay(None, cx);
        }
        cx.notify();
    }

    /// Opens the Go Live picker, or stops the stream if one is running.
    pub fn toggle_go_live_picker(&mut self, cx: &mut Context<Self>) {
        if self.go_live.is_some() {
            self.stop_go_live(cx);
            return;
        }
        let sources = rcord_capture::sources();
        if sources.is_empty() {
            self.toast("No screens were found to share.", cx);
            return;
        }
        self.set_overlay(
            Some(crate::state::Overlay::GoLive {
                sources,
                selected: 0,
                preset: StreamPreset::from_settings(&self.settings.stream),
            }),
            cx,
        );
    }

    /// Ends both directions, for example when the call ends.
    pub(crate) fn end_streams(&mut self, cx: &mut Context<Self>) {
        self.stop_go_live(cx);
        self.stop_all_watching(cx);
    }

    /// Looks at the main gateway's stream events.
    pub(crate) fn stream_gateway_update(&mut self, update: &Update, cx: &mut Context<Self>) {
        let key = match update {
            Update::StreamCreate(s) | Update::StreamUpdate(s) => {
                let rtc = s.rtc_server_id;
                let viewers = s.viewer_ids.clone();
                self.with_pending(&s.stream_key, |p, list| {
                    if rtc.is_some() {
                        p.rtc_server_id = rtc;
                    }
                    *list = viewers;
                });
                s.stream_key.clone()
            }
            Update::StreamServerUpdate(s) => {
                let server = s.endpoint.clone().map(|e| (e, s.token.clone()));
                self.with_pending(&s.stream_key, |p, _| p.server = server);
                s.stream_key.clone()
            }
            Update::StreamDelete(d) => {
                self.stream_deleted(&d.stream_key, d.reason.as_deref(), cx);
                return;
            }
            _ => return,
        };
        self.connect_stream(&key, cx);
        cx.notify();
    }

    fn with_pending(&mut self, key: &str, f: impl FnOnce(&mut Pending, &mut Vec<Snowflake>)) {
        if let Some(live) = self.go_live.as_mut().filter(|l| l.key == key) {
            f(&mut live.pending, &mut live.viewers);
        } else if let Some(w) = self.watching.iter_mut().find(|w| w.key == key) {
            f(&mut w.pending, &mut w.viewers);
        }
    }

    fn stream_deleted(&mut self, key: &str, reason: Option<&str>, cx: &mut Context<Self>) {
        let message = match reason {
            Some("stream_full") => Some("That stream is full."),
            Some("unauthorized") => Some("You can't watch or share streams here."),
            _ => None,
        };
        if self.go_live.as_ref().is_some_and(|l| l.key == key) {
            self.go_live = None;
        } else if let Some(i) = self.watching.iter().position(|w| w.key == key) {
            let w = self.watching.remove(i);
            self.close_watch(w, false, cx);
        } else {
            return;
        }
        if let Some(m) = message {
            self.toast(m, cx);
        }
        cx.notify();
    }

    /// Starts the media connection once the server, token, and stream server ID are known.
    fn connect_stream(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(me) = self.store.me_id() else {
            return;
        };
        let Some(session_id) = self.call.as_ref().and_then(|c| c.session_id()) else {
            return;
        };
        let audio = self.call.as_ref().and_then(|c| c.audio_sink());
        let (pending, role) = if let Some(live) = self.go_live.as_mut().filter(|l| l.key == key) {
            if live.handle.is_some() {
                return;
            }
            (
                &live.pending,
                StreamRole::Send {
                    quality: live.preset.quality(),
                },
            )
        } else if let Some(w) = self.watching.iter_mut().find(|w| w.key == key) {
            if w.handle.is_some() {
                return;
            }
            (
                &w.pending,
                StreamRole::Watch {
                    streamer: w.streamer.get(),
                    audio,
                },
            )
        } else {
            return;
        };
        let (Some(rtc), Some((endpoint, token))) = (pending.rtc_server_id, pending.server.clone())
        else {
            return;
        };
        let config = StreamConfig {
            endpoint,
            server_id: rtc,
            user_id: me.get(),
            session_id,
            token,
            role,
        };
        let net = cx.global::<Net>();
        let started = {
            let _guard = net.handle().enter();
            rcord_voice::connect_stream(config)
        };
        let (handle, events) = match started {
            Ok(v) => v,
            Err(e) => {
                self.toast(format!("The stream could not start: {e}"), cx);
                return;
            }
        };
        let key_owned = key.to_owned();
        let task = cx.spawn(async move |this, cx| {
            while let Ok(first) = events.recv().await {
                let ok = this.update(cx, |state, cx| {
                    state.stream_event(&key_owned, first, cx);
                    while let Ok(next) = events.try_recv() {
                        state.stream_event(&key_owned, next, cx);
                    }
                });
                if ok.is_err() {
                    break;
                }
            }
        });

        if let Some(live) = self.go_live.as_mut().filter(|l| l.key == key) {
            let handle = Arc::new(handle);
            if let Some(capture) = live.capture.take() {
                spawn_pump(capture, handle.clone(), live.pump_stop.clone());
            }
            live.handle = Some(handle);
            live.status = StreamStatus::Connecting;
            live._events = Some(task);
        } else if let Some(w) = self.watching.iter_mut().find(|w| w.key == key) {
            w.handle = Some(handle);
            w.status = StreamStatus::Connecting;
            w._events = Some(task);
        }
    }

    fn stream_event(&mut self, key: &str, event: VoiceEvent, cx: &mut Context<Self>) {
        let status = match &event {
            VoiceEvent::State(ConnectionState::Connecting) => Some(StreamStatus::Connecting),
            VoiceEvent::State(ConnectionState::Connected { .. }) => Some(StreamStatus::Live),
            VoiceEvent::State(ConnectionState::Reconnecting) => Some(StreamStatus::Reconnecting),
            _ => None,
        };
        if let Some(live) = self.go_live.as_mut().filter(|l| l.key == key) {
            match event {
                VoiceEvent::Closed { reason, .. } => {
                    self.stop_go_live(cx);
                    if reason != "Disconnected" {
                        self.toast(format!("Your stream ended: {reason}"), cx);
                    }
                }
                _ => {
                    if let Some(s) = status {
                        live.status = s;
                        cx.notify();
                    }
                }
            }
            return;
        }
        let Some(w) = self.watching.iter_mut().find(|w| w.key == key) else {
            return;
        };
        match event {
            VoiceEvent::VideoFrame => {
                let Some(frame) = w.handle.as_ref().and_then(|h| h.take_frame()) else {
                    return;
                };
                let Some(buffer) =
                    image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra.clone())
                else {
                    return;
                };
                let image = Arc::new(RenderImage::new([image::Frame::new(buffer)]));
                w.size = (frame.width, frame.height);
                w.frames += 1;
                let elapsed = w.counted_since.elapsed();
                if elapsed >= Duration::from_secs(1) {
                    w.fps = (w.frames as f32 / elapsed.as_secs_f32()).round() as u32;
                    w.frames = 0;
                    w.counted_since = std::time::Instant::now();
                }
                if let Some(old) = w.frame.replace(image) {
                    cx.drop_image(old, None);
                }
                cx.notify();
            }
            VoiceEvent::Closed { reason, .. } => {
                let streamer = w.streamer;
                self.stop_watching(streamer, cx);
                if reason != "Disconnected" {
                    self.toast(format!("The stream ended: {reason}"), cx);
                }
            }
            _ => {
                if let Some(s) = status {
                    w.status = s;
                    cx.notify();
                }
            }
        }
    }
}

/// Moves captured frames to the encoder on a thread of their own, so neither waits on the
/// other. Ends when the stream stops or capture ends (for example from the desktop's own
/// stop-sharing control).
fn spawn_pump(capture: Capture, handle: Arc<StreamHandle>, stop: Arc<AtomicBool>) {
    let result = std::thread::Builder::new()
        .name("rcord-go-live".into())
        .spawn(move || {
            use std::sync::mpsc::RecvTimeoutError;
            while !stop.load(Ordering::Relaxed) {
                match capture.frames().recv_timeout(Duration::from_millis(250)) {
                    Ok(f) => {
                        let bgra = f.format == PixelFormat::Bgra;
                        if !handle.offer_frame(f.width, f.height, bgra, f.data) {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
    if let Err(e) = result {
        tracing::warn!("could not start the screen share thread: {e}");
    }
}
