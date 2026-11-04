//! One voice connection: the voice gateway WebSocket, the UDP media socket, transport and
//! end-to-end encryption, and the hand-off to the media loop.
//!
//! Joining goes: Hello, Identify, Ready (our SSRC and the media server's address), IP
//! discovery over UDP, Select Protocol with the best encryption mode both sides support, then
//! Session Description with the transport key and the DAVE protocol version. After that,
//! microphone frames go out and everyone else's come in. A dropped socket resumes with the
//! last acknowledged sequence number, keeping the UDP socket and keys.
//!
//! Go Live streams use the same protocol on a server of their own: the client identifies
//! with video, offers H.264, and either announces its stream SSRCs and sends frames, or
//! receives the streamer's frames, reassembles and decrypts them, and hands them to a
//! decoder thread. Lost frames are recovered by asking the sender for a keyframe.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::{MissedTickBehavior, interval_at, sleep, timeout};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::audio::{AudioEngine, AudioEvent, AudioSettings, Inbound, Outbound, SILENCE};
use crate::crypto::TransportCipher;
use crate::dave::{self, Dave, Reply};
use crate::h264::{self, Depacketized, Depacketizer};
use crate::protocol::{
    self, ClientDisconnect, ClientsConnect, Codec, DaveExecuteTransition, DavePrepareEpoch,
    DavePrepareTransition, EncryptionMode, HeartbeatAck, Hello, Identify, Incoming, Outgoing,
    Ready, Resolution, Resume, SelectProtocol, SelectProtocolData, SessionDescription, SpeakingIn,
    SpeakingOut, StreamDescriptor, VOICE_GATEWAY_VERSION, VideoIn, VideoOut, VideoStreamOut,
    VoiceOp, close,
};
use crate::rtp::{self, IpDiscovery, RtpHeader};
use crate::video::{
    DecodedFrame, EncodedFrame, RawFrame, VideoDecoder, VideoEncoder, VideoQuality,
};

/// Where and as whom to connect, from the main gateway's voice state and voice server
/// updates.
#[derive(Clone)]
pub struct VoiceConfig {
    /// `host[:port]` from VOICE_SERVER_UPDATE.
    pub endpoint: String,
    /// The guild ID, or the channel ID for a direct call.
    pub server_id: u64,
    pub channel_id: u64,
    pub user_id: u64,
    pub session_id: String,
    pub token: String,
    pub audio: AudioSettings,
    pub self_mute: bool,
    pub self_deaf: bool,
}

impl std::fmt::Debug for VoiceConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceConfig")
            .field("endpoint", &self.endpoint)
            .field("server_id", &self.server_id)
            .field("channel_id", &self.channel_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Opening the voice gateway and finding a route for media.
    Connecting,
    /// Media flows. `encrypted` is true when the call is end-to-end encrypted.
    Connected { encrypted: bool },
    /// The voice gateway dropped and is resuming.
    Reconnecting,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VoiceEvent {
    State(ConnectionState),
    /// A participant started or stopped speaking. `0` is this client.
    Speaking {
        user_id: u64,
        speaking: bool,
    },
    Latency(Duration),
    /// The call's privacy code changed; `None` when it is not end-to-end encrypted.
    PrivacyCode(Option<String>),
    InputLevel(f32),
    DeviceError(String),
    /// A decoded stream frame is ready; take it with [`StreamHandle::take_frame`].
    VideoFrame,
    /// The connection ended for good. `code` is the voice gateway's close code, if any.
    Closed {
        code: Option<u16>,
        reason: String,
    },
}

enum Control {
    Disconnect,
}

/// A running voice connection. Dropping it disconnects.
pub struct VoiceHandle {
    control: mpsc::UnboundedSender<Control>,
    engine: AudioEngine,
}

impl std::fmt::Debug for VoiceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceHandle").finish_non_exhaustive()
    }
}

impl Drop for VoiceHandle {
    fn drop(&mut self) {
        let _ = self.control.send(Control::Disconnect);
    }
}

impl VoiceHandle {
    pub fn set_muted(&self, muted: bool) {
        self.engine.set_muted(muted);
    }

    pub fn set_deafened(&self, deafened: bool) {
        self.engine.set_deafened(deafened);
    }

    pub fn set_push_to_talk(&self, held: bool) {
        self.engine.set_push_to_talk(held);
    }

    pub fn set_user_volume(&self, user_id: u64, volume: f32) {
        self.engine.set_user_volume(user_id, volume);
    }

    pub fn update_audio(&self, settings: AudioSettings) {
        self.engine.update_settings(settings);
    }

    /// Where a watched stream's sound goes, so it plays and mixes with the call.
    pub fn audio_sink(&self) -> std::sync::mpsc::Sender<Inbound> {
        self.engine.sink()
    }
}

/// Connects on the current Tokio runtime. Events stop after [`VoiceEvent::Closed`].
pub fn connect(
    config: VoiceConfig,
) -> std::io::Result<(VoiceHandle, async_channel::Receiver<VoiceEvent>)> {
    let (event_tx, event_rx) = async_channel::unbounded();
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let (audio_tx, audio_rx) = async_channel::unbounded();
    let engine = AudioEngine::start(config.audio.clone(), out_tx, audio_tx)?;
    engine.set_muted(config.self_mute);
    engine.set_deafened(config.self_deaf);
    let sink = engine.sink();
    let (control_tx, control_rx) = mpsc::unbounded_channel();

    // Device and speaking events from the media thread pass straight through.
    let forward = event_tx.clone();
    tokio::spawn(async move {
        while let Ok(e) = audio_rx.recv().await {
            let e = match e {
                AudioEvent::Speaking { user_id, speaking } => {
                    VoiceEvent::Speaking { user_id, speaking }
                }
                AudioEvent::InputLevel(l) => VoiceEvent::InputLevel(l),
                AudioEvent::DeviceError(e) => VoiceEvent::DeviceError(e),
            };
            if forward.send(e).await.is_err() {
                break;
            }
        }
    });

    tokio::spawn(async move {
        let target = Target::voice(&config);
        let mut conn = Connection::new(target, Some(sink), None, event_tx.clone());
        let (code, reason) = conn.run(out_rx, control_rx).await;
        let _ = event_tx.send(VoiceEvent::Closed { code, reason }).await;
    });

    Ok((
        VoiceHandle {
            control: control_tx,
            engine,
        },
        event_rx,
    ))
}

/// Go Live: sending this client's stream or watching someone else's. Streams have media
/// servers of their own, named by the main gateway's stream events.
#[derive(Clone)]
pub struct StreamConfig {
    pub endpoint: String,
    /// The stream's `rtc_server_id`.
    pub server_id: u64,
    pub user_id: u64,
    /// The voice session of the call the stream belongs to.
    pub session_id: String,
    pub token: String,
    pub role: StreamRole,
}

impl std::fmt::Debug for StreamConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamConfig")
            .field("endpoint", &self.endpoint)
            .field("server_id", &self.server_id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub enum StreamRole {
    /// Send frames given to [`StreamHandle::offer_frame`].
    Send { quality: VideoQuality },
    /// Receive `streamer`'s frames; their sound goes to `audio`, normally the call's mixer.
    Watch {
        streamer: u64,
        audio: Option<std::sync::mpsc::Sender<Inbound>>,
    },
}

/// A running stream connection. Dropping it disconnects and stops its codec thread.
pub struct StreamHandle {
    control: mpsc::UnboundedSender<Control>,
    frames: Option<std::sync::mpsc::SyncSender<OwnedFrame>>,
    latest: Arc<parking_lot::Mutex<Option<Arc<DecodedFrame>>>>,
    pending: Arc<AtomicBool>,
    /// Keeps a viewer's (unused) outbound channel open; the connection ends when it closes.
    _outbound: Option<mpsc::UnboundedSender<Outbound>>,
}

impl std::fmt::Debug for StreamHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamHandle").finish_non_exhaustive()
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        let _ = self.control.send(Control::Disconnect);
    }
}

struct OwnedFrame {
    width: u32,
    height: u32,
    bgra: bool,
    data: Vec<u8>,
}

impl StreamHandle {
    /// Hands a captured frame to the encoder. A busy encoder skips it. Returns false once
    /// the stream has ended.
    pub fn offer_frame(&self, width: u32, height: u32, bgra: bool, data: Vec<u8>) -> bool {
        let Some(tx) = &self.frames else {
            return false;
        };
        !matches!(
            tx.try_send(OwnedFrame {
                width,
                height,
                bgra,
                data
            }),
            Err(std::sync::mpsc::TrySendError::Disconnected(_))
        )
    }

    /// The newest decoded frame of a watched stream, once per [`VoiceEvent::VideoFrame`].
    pub fn take_frame(&self) -> Option<Arc<DecodedFrame>> {
        self.pending.store(false, Ordering::Relaxed);
        self.latest.lock().clone()
    }
}

/// Connects a Go Live stream on the current Tokio runtime.
pub fn connect_stream(
    config: StreamConfig,
) -> std::io::Result<(StreamHandle, async_channel::Receiver<VoiceEvent>)> {
    let (event_tx, event_rx) = async_channel::unbounded();
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let (control_tx, control_rx) = mpsc::unbounded_channel();
    let want_keyframe = Arc::new(AtomicBool::new(true));
    let keyframe_asked = Arc::new(AtomicBool::new(false));
    let latest = Arc::new(parking_lot::Mutex::new(None));
    let pending = Arc::new(AtomicBool::new(false));

    let (sending, streamer, audio, frames, decoder, outbound) = match config.role.clone() {
        StreamRole::Send { quality } => {
            let (tx, rx) = std::sync::mpsc::sync_channel::<OwnedFrame>(1);
            let asked = keyframe_asked.clone();
            std::thread::Builder::new()
                .name("rcord-video-encode".into())
                .spawn(move || encode_loop(quality, rx, out_tx, asked))?;
            (Some(quality), config.user_id, None, Some(tx), None, None)
        }
        StreamRole::Watch { streamer, audio } => {
            let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(4);
            let (latest, pending, want, events) = (
                latest.clone(),
                pending.clone(),
                want_keyframe.clone(),
                event_tx.clone(),
            );
            std::thread::Builder::new()
                .name("rcord-video-decode".into())
                .spawn(move || decode_loop(rx, latest, pending, want, events))?;
            (None, streamer, audio, None, Some(tx), Some(out_tx))
        }
    };

    let video = Video {
        sending,
        streamer,
        ssrc: 0,
        rtx_ssrc: 0,
        rtp: RtpHeader {
            payload_type: RtpHeader::H264,
            marker: false,
            sequence: rand::random(),
            timestamp: rand::random(),
            ssrc: 0,
        },
        clock: Instant::now(),
        senders: HashMap::new(),
        reassembly: HashMap::new(),
        decoder,
        want_keyframe,
        keyframe_asked,
        last_pli: None,
    };
    let target = Target {
        endpoint: config.endpoint.clone(),
        server_id: config.server_id,
        user_id: config.user_id,
        session_id: config.session_id.clone(),
        token: config.token.clone(),
        dave_channel: config.server_id.saturating_sub(1),
    };
    tokio::spawn(async move {
        let mut conn = Connection::new(target, audio, Some(video), event_tx.clone());
        let (code, reason) = conn.run(out_rx, control_rx).await;
        let _ = event_tx.send(VoiceEvent::Closed { code, reason }).await;
    });

    Ok((
        StreamHandle {
            control: control_tx,
            frames,
            latest,
            pending,
            _outbound: outbound,
        },
        event_rx,
    ))
}

/// Encodes captured frames for the connection. A screen that does not change sends no new
/// frames on some systems, so the last one is repeated each second to keep viewers' pictures
/// alive and let late joiners get a keyframe.
fn encode_loop(
    quality: VideoQuality,
    frames: std::sync::mpsc::Receiver<OwnedFrame>,
    out: mpsc::UnboundedSender<Outbound>,
    keyframe_asked: Arc<AtomicBool>,
) {
    use std::sync::mpsc::RecvTimeoutError;
    let mut encoder = VideoEncoder::new(quality);
    let mut last: Option<OwnedFrame> = None;
    loop {
        match frames.recv_timeout(Duration::from_secs(1)) {
            Ok(f) => last = Some(f),
            Err(RecvTimeoutError::Timeout) if last.is_some() => {}
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let Some(f) = &last else {
            continue;
        };
        if keyframe_asked.swap(false, Ordering::Relaxed) {
            encoder.request_keyframe();
        }
        let raw = RawFrame {
            width: f.width,
            height: f.height,
            bgra: f.bgra,
            data: &f.data,
        };
        match encoder.encode(raw) {
            Ok(Some(encoded)) => {
                if out.send(Outbound::Video(encoded)).is_err() {
                    return;
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("video encode failed: {e}"),
        }
    }
}

fn decode_loop(
    units: std::sync::mpsc::Receiver<Vec<u8>>,
    latest: Arc<parking_lot::Mutex<Option<Arc<DecodedFrame>>>>,
    pending: Arc<AtomicBool>,
    want_keyframe: Arc<AtomicBool>,
    events: async_channel::Sender<VoiceEvent>,
) {
    let mut decoder = match VideoDecoder::new() {
        Ok(d) => d,
        Err(e) => {
            let _ = events.try_send(VoiceEvent::DeviceError(e.to_string()));
            return;
        }
    };
    let mut synced = false;
    while let Ok(unit) = units.recv() {
        // Until a keyframe arrives there is nothing to decode against.
        if !synced && !h264::is_keyframe(&unit) {
            want_keyframe.store(true, Ordering::Relaxed);
            continue;
        }
        match decoder.decode(&unit) {
            Ok(Some(frame)) => {
                synced = true;
                *latest.lock() = Some(Arc::new(frame));
                if !pending.swap(true, Ordering::Relaxed)
                    && events.try_send(VoiceEvent::VideoFrame).is_err()
                {
                    return;
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::debug!("video decode failed: {e}");
                synced = false;
                want_keyframe.store(true, Ordering::Relaxed);
            }
        }
        if events.is_closed() {
            return;
        }
    }
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// State that survives a resume.
struct Media {
    udp: Arc<UdpSocket>,
    ssrc: u32,
    cipher: Option<TransportCipher>,
}

/// Where to connect and as whom; shared by calls and streams.
struct Target {
    endpoint: String,
    server_id: u64,
    user_id: u64,
    session_id: String,
    token: String,
    /// The DAVE group: the voice channel for calls, the stream server ID minus one for
    /// streams.
    dave_channel: u64,
}

impl Target {
    fn voice(c: &VoiceConfig) -> Self {
        Self {
            endpoint: c.endpoint.clone(),
            server_id: c.server_id,
            user_id: c.user_id,
            session_id: c.session_id.clone(),
            token: c.token.clone(),
            dave_channel: c.channel_id,
        }
    }
}

/// Video state for a Go Live connection.
struct Video {
    /// Set when sending: the encoder's limits, announced to the server.
    sending: Option<VideoQuality>,
    /// The streamer, when watching; frames from unknown SSRCs are theirs.
    streamer: u64,
    ssrc: u32,
    rtx_ssrc: u32,
    rtp: RtpHeader,
    clock: Instant,
    /// Video SSRCs to their senders, from the server's video updates.
    senders: HashMap<u32, u64>,
    reassembly: HashMap<u32, Depacketizer>,
    /// Reassembled, decrypted access units on their way to the decoder.
    decoder: Option<std::sync::mpsc::SyncSender<Vec<u8>>>,
    /// Set by the decoder or a lost frame: ask the sender for a keyframe.
    want_keyframe: Arc<AtomicBool>,
    /// Set when a viewer asked us for a keyframe.
    keyframe_asked: Arc<AtomicBool>,
    last_pli: Option<Instant>,
}

struct Connection {
    config: Target,
    sink: Option<std::sync::mpsc::Sender<Inbound>>,
    video: Option<Video>,
    events: async_channel::Sender<VoiceEvent>,
    media: Option<Media>,
    dave: Dave,
    /// Who sends on each SSRC, from the server's speaking updates.
    users: HashMap<u32, u64>,
    seq_ack: i64,
    rtp: RtpHeader,
    last_sent: Option<Instant>,
    packet: Vec<u8>,
    privacy_code: Option<String>,
}

enum End {
    /// Try to resume.
    Resume { code: Option<u16>, reason: String },
    /// Stop for good.
    Close { code: Option<u16>, reason: String },
}

impl Connection {
    fn new(
        config: Target,
        sink: Option<std::sync::mpsc::Sender<Inbound>>,
        video: Option<Video>,
        events: async_channel::Sender<VoiceEvent>,
    ) -> Self {
        let dave = Dave::new(config.user_id, config.dave_channel);
        Self {
            config,
            sink,
            video,
            events,
            media: None,
            dave,
            users: HashMap::new(),
            seq_ack: -1,
            rtp: RtpHeader {
                payload_type: RtpHeader::OPUS,
                marker: false,
                sequence: rand::random(),
                timestamp: rand::random(),
                ssrc: 0,
            },
            last_sent: None,
            packet: Vec::with_capacity(1500),
            privacy_code: None,
        }
    }

    fn emit(&self, e: VoiceEvent) {
        let _ = self.events.try_send(e);
    }

    async fn run(
        &mut self,
        mut audio: mpsc::UnboundedReceiver<Outbound>,
        mut control: mpsc::UnboundedReceiver<Control>,
    ) -> (Option<u16>, String) {
        let mut attempts = 0u32;
        loop {
            let resuming = self.media.is_some();
            self.emit(VoiceEvent::State(if resuming {
                ConnectionState::Reconnecting
            } else {
                ConnectionState::Connecting
            }));
            match self.session(resuming, &mut audio, &mut control).await {
                End::Close { code, reason } => return (code, reason),
                End::Resume { code, reason } => {
                    attempts += 1;
                    tracing::info!("voice connection dropped ({code:?}): {reason}");
                    if attempts > 5 {
                        return (code, reason);
                    }
                    let wait = Duration::from_millis(500 * u64::from(attempts));
                    tokio::select! {
                        _ = sleep(wait) => {}
                        _ = control.recv() => return (None, "Disconnected".into()),
                    }
                }
            }
        }
    }

    async fn session(
        &mut self,
        resuming: bool,
        audio: &mut mpsc::UnboundedReceiver<Outbound>,
        control: &mut mpsc::UnboundedReceiver<Control>,
    ) -> End {
        let host = self
            .config
            .endpoint
            .trim_start_matches("wss://")
            .trim_end_matches('/');
        let url = format!("wss://{host}/?v={VOICE_GATEWAY_VERSION}");
        let mut ws = match timeout(
            Duration::from_secs(10),
            tokio_tungstenite::connect_async(url),
        )
        .await
        {
            Ok(Ok((ws, _))) => ws,
            Ok(Err(e)) => {
                return End::Resume {
                    code: None,
                    reason: e.to_string(),
                };
            }
            Err(_) => {
                return End::Resume {
                    code: None,
                    reason: "voice server timed out".into(),
                };
            }
        };

        let server_id = self.config.server_id.to_string();
        let first = if resuming {
            json(
                VoiceOp::Resume,
                &Resume {
                    server_id: &server_id,
                    session_id: &self.config.session_id,
                    token: &self.config.token,
                    seq_ack: self.seq_ack,
                },
            )
        } else {
            let user_id = self.config.user_id.to_string();
            json(
                VoiceOp::Identify,
                &Identify {
                    server_id: &server_id,
                    user_id: &user_id,
                    session_id: &self.config.session_id,
                    token: &self.config.token,
                    video: self.video.is_some(),
                    streams: if self.video.is_some() {
                        vec![StreamDescriptor::SCREEN]
                    } else {
                        Vec::new()
                    },
                    max_dave_protocol_version: dave::MAX_PROTOCOL_VERSION,
                },
            )
        };
        if let Err(e) = ws.send(WsMessage::Text(first.into())).await {
            return End::Resume {
                code: None,
                reason: e.to_string(),
            };
        }

        // Heartbeats start once Hello says how often.
        let idle = Duration::from_secs(3600);
        let mut heartbeat = interval_at(tokio::time::Instant::now() + idle, idle);
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut awaiting_ack = 0u32;
        let mut keepalive = tokio::time::interval(Duration::from_secs(5));
        let mut keepalive_n = 0u32;
        let mut video_tick = tokio::time::interval(Duration::from_millis(200));
        video_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let has_video = self.video.is_some();
        let mut buf = vec![0u8; 2048];

        loop {
            let udp = self.media.as_ref().map(|m| m.udp.clone());
            tokio::select! {
                frame = ws.next() => {
                    let frame = match frame {
                        Some(Ok(f)) => f,
                        Some(Err(e)) => return End::Resume { code: None, reason: e.to_string() },
                        None => return End::Resume { code: None, reason: "voice server closed the connection".into() },
                    };
                    match frame {
                        WsMessage::Text(text) => {
                            let Ok(msg) = serde_json::from_str::<Incoming>(text.as_str()) else {
                                continue;
                            };
                            if let Some(seq) = msg.seq {
                                self.seq_ack = seq as i64;
                            }
                            if msg.op == VoiceOp::HeartbeatAck as u8 {
                                awaiting_ack = 0;
                            }
                            match self.handle_json(msg, &mut ws).await {
                                Ok(Some(interval)) => {
                                    let start = tokio::time::Instant::now()
                                        + interval.mul_f64(rand::random::<f64>());
                                    heartbeat = interval_at(start, interval);
                                    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
                                }
                                Ok(None) => {}
                                Err(end) => return end,
                            }
                        }
                        WsMessage::Binary(bytes) => {
                            if let Some((seq, op, payload)) = protocol::parse_binary(&bytes) {
                                self.seq_ack = i64::from(seq);
                                let replies = self.handle_binary(op, payload);
                                if let Err(end) = send_replies(&mut ws, replies).await {
                                    return end;
                                }
                            }
                        }
                        WsMessage::Close(frame) => {
                            let code = frame.as_ref().map(|f| u16::from(f.code));
                            let reason = frame
                                .map(|f| f.reason.to_string())
                                .filter(|r| !r.is_empty())
                                .unwrap_or_else(|| "voice server closed the connection".into());
                            return match code {
                                Some(c) if !close::can_resume(c) => End::Close { code, reason: describe_close(c, reason) },
                                _ => End::Resume { code, reason },
                            };
                        }
                        _ => {}
                    }
                }
                _ = heartbeat.tick() => {
                    if awaiting_ack >= 2 {
                        return End::Resume { code: None, reason: "voice heartbeat not acknowledged".into() };
                    }
                    awaiting_ack += 1;
                    let beat = json(VoiceOp::Heartbeat, &protocol::Heartbeat {
                        t: unix_ms(),
                        seq_ack: self.seq_ack,
                    });
                    if let Err(e) = ws.send(WsMessage::Text(beat.into())).await {
                        return End::Resume { code: None, reason: e.to_string() };
                    }
                }
                _ = keepalive.tick(), if udp.is_some() => {
                    if let Some(udp) = &udp {
                        keepalive_n = keepalive_n.wrapping_add(1);
                        let mut p = [0u8; 8];
                        p[..4].copy_from_slice(&keepalive_n.to_le_bytes());
                        let _ = udp.send(&p).await;
                    }
                }
                _ = video_tick.tick(), if has_video => self.request_keyframe().await,
                received = recv_udp(udp.as_deref(), &mut buf) => {
                    if let Ok(n) = received {
                        self.receive_media(n, &buf);
                    }
                }
                out = audio.recv() => {
                    match out {
                        Some(Outbound::Opus(frame)) => self.send_media(&frame).await,
                        Some(Outbound::Video(frame)) => self.send_video(frame).await,
                        Some(Outbound::Speaking(on)) => {
                            if let Some(m) = &self.media {
                                let flag = if self.video.is_some() {
                                    protocol::speaking::SOUNDSHARE
                                } else {
                                    protocol::speaking::MICROPHONE
                                };
                                let msg = json(VoiceOp::Speaking, &SpeakingOut {
                                    speaking: if on { flag } else { 0 },
                                    delay: 0,
                                    ssrc: m.ssrc,
                                });
                                if let Err(e) = ws.send(WsMessage::Text(msg.into())).await {
                                    return End::Resume { code: None, reason: e.to_string() };
                                }
                            }
                        }
                        None => return End::Close { code: None, reason: "audio stopped".into() },
                    }
                }
                _ = control.recv() => {
                    let _ = ws.close(None).await;
                    return End::Close { code: None, reason: "Disconnected".into() };
                }
            }
        }
    }

    /// Handles a JSON frame. Returns the heartbeat interval when it is Hello.
    async fn handle_json(&mut self, msg: Incoming, ws: &mut Ws) -> Result<Option<Duration>, End> {
        let Some(op) = VoiceOp::from_u8(msg.op) else {
            return Ok(None);
        };
        let d = msg.d;
        let parse_err = |e: serde_json::Error| End::Close {
            code: None,
            reason: format!("unexpected voice payload: {e}"),
        };
        match op {
            VoiceOp::Hello => {
                let h: Hello = serde_json::from_value(d).map_err(parse_err)?;
                return Ok(Some(Duration::from_secs_f64(
                    (h.heartbeat_interval / 1000.0).max(1.0),
                )));
            }
            VoiceOp::HeartbeatAck => {
                if let Ok(ack) = serde_json::from_value::<HeartbeatAck>(d) {
                    let rtt = unix_ms().saturating_sub(ack.t);
                    if rtt < 60_000 {
                        self.emit(VoiceEvent::Latency(Duration::from_millis(rtt)));
                    }
                }
            }
            VoiceOp::Ready => {
                let r: Ready = serde_json::from_value(d).map_err(parse_err)?;
                let mode = EncryptionMode::choose(&r.modes).ok_or_else(|| End::Close {
                    code: None,
                    reason: "the voice server offered no supported encryption".into(),
                })?;
                let (udp, me) = discover(&r).await.map_err(|e| End::Resume {
                    code: None,
                    reason: format!("could not reach the voice server: {e}"),
                })?;
                self.rtp.ssrc = r.ssrc;
                if let (Some(v), Some(stream)) = (&mut self.video, r.streams.first()) {
                    v.ssrc = stream.ssrc;
                    v.rtx_ssrc = stream.rtx_ssrc;
                    v.rtp.ssrc = stream.ssrc;
                }
                self.media = Some(Media {
                    udp: Arc::new(udp),
                    ssrc: r.ssrc,
                    cipher: None,
                });
                let select = json(
                    VoiceOp::SelectProtocol,
                    &SelectProtocol {
                        protocol: "udp",
                        data: SelectProtocolData {
                            address: me.address,
                            port: me.port,
                            mode,
                        },
                        codecs: if self.video.is_some() {
                            vec![Codec::OPUS, Codec::H264]
                        } else {
                            Vec::new()
                        },
                    },
                );
                ws.send(WsMessage::Text(select.into()))
                    .await
                    .map_err(|e| End::Resume {
                        code: None,
                        reason: e.to_string(),
                    })?;
            }
            VoiceOp::SessionDescription => {
                let s: SessionDescription = serde_json::from_value(d).map_err(parse_err)?;
                let cipher =
                    TransportCipher::new(s.mode, &s.secret_key).map_err(|e| End::Close {
                        code: None,
                        reason: e.to_string(),
                    })?;
                if let Some(m) = &mut self.media {
                    m.cipher = Some(cipher);
                }
                let replies = self.dave.start(s.dave_protocol_version);
                send_replies(ws, replies).await?;
                self.announce_video(ws).await?;
                self.connected();
            }
            VoiceOp::Resumed => self.connected(),
            VoiceOp::Video => {
                if let (Some(v), Ok(info)) = (&mut self.video, serde_json::from_value::<VideoIn>(d))
                {
                    v.senders.retain(|_, u| *u != info.user_id);
                    let ssrcs = info.streams.iter().map(|s| s.ssrc).chain([info.video_ssrc]);
                    for ssrc in ssrcs.filter(|s| *s != 0) {
                        v.senders.insert(ssrc, info.user_id);
                    }
                    if info.audio_ssrc != 0 {
                        self.users.insert(info.audio_ssrc, info.user_id);
                    }
                    self.dave.add_members(&[info.user_id]);
                    v.want_keyframe.store(true, Ordering::Relaxed);
                }
            }
            VoiceOp::Speaking => {
                if let Ok(s) = serde_json::from_value::<SpeakingIn>(d) {
                    self.users.insert(s.ssrc, s.user_id);
                    self.dave.add_members(&[s.user_id]);
                }
            }
            VoiceOp::ClientsConnect => {
                if let Ok(c) = serde_json::from_value::<ClientsConnect>(d) {
                    self.dave.add_members(&c.user_ids);
                }
            }
            VoiceOp::ClientDisconnect => {
                if let Ok(c) = serde_json::from_value::<ClientDisconnect>(d) {
                    self.users.retain(|_, u| *u != c.user_id);
                    self.dave.remove_member(c.user_id);
                    self.emit(VoiceEvent::Speaking {
                        user_id: c.user_id,
                        speaking: false,
                    });
                }
            }
            VoiceOp::DavePrepareTransition => {
                if let Ok(t) = serde_json::from_value::<DavePrepareTransition>(d) {
                    let replies = self
                        .dave
                        .prepare_transition(t.transition_id, t.protocol_version);
                    send_replies(ws, replies).await?;
                    self.update_privacy();
                }
            }
            VoiceOp::DaveExecuteTransition => {
                if let Ok(t) = serde_json::from_value::<DaveExecuteTransition>(d) {
                    self.dave.execute_transition(t.transition_id);
                    self.update_privacy();
                }
            }
            VoiceOp::DavePrepareEpoch => {
                if let Ok(e) = serde_json::from_value::<DavePrepareEpoch>(d) {
                    let replies = self.dave.prepare_epoch(e.epoch, e.protocol_version);
                    send_replies(ws, replies).await?;
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_binary(&mut self, op: VoiceOp, payload: &[u8]) -> Vec<Reply> {
        let replies = match op {
            VoiceOp::DaveMlsExternalSender => self.dave.external_sender(payload),
            VoiceOp::DaveMlsProposals => self.dave.proposals(payload),
            VoiceOp::DaveMlsAnnounceCommitTransition => self.dave.commit(payload),
            VoiceOp::DaveMlsWelcome => self.dave.welcome(payload),
            _ => Vec::new(),
        };
        self.update_privacy();
        replies
    }

    fn connected(&mut self) {
        self.emit(VoiceEvent::State(ConnectionState::Connected {
            encrypted: self.dave.protocol_version() > 0,
        }));
        self.update_privacy();
    }

    fn update_privacy(&mut self) {
        let code = if self.dave.is_active() {
            self.dave.privacy_code()
        } else {
            None
        };
        if code != self.privacy_code {
            self.privacy_code.clone_from(&code);
            self.emit(VoiceEvent::PrivacyCode(code));
            self.emit(VoiceEvent::State(ConnectionState::Connected {
                encrypted: self.dave.protocol_version() > 0,
            }));
        }
    }

    async fn send_media(&mut self, opus: &[u8]) {
        let Some(media) = &mut self.media else {
            return;
        };
        let Some(cipher) = &mut media.cipher else {
            return;
        };
        let frame = if opus == SILENCE {
            std::borrow::Cow::Borrowed(opus)
        } else {
            match self.dave.encrypt_opus(opus) {
                Some(f) => f,
                None => return, // Keys not ready; others could not play it.
            }
        };
        // After a pause, move the timestamp on by the time that passed.
        let now = Instant::now();
        match self.last_sent {
            Some(t) if now.duration_since(t) > Duration::from_millis(60) => {
                let frames = (now.duration_since(t).as_millis() / 20) as u32;
                self.rtp.sequence = self.rtp.sequence.wrapping_add(1);
                self.rtp.timestamp = self.rtp.timestamp.wrapping_add(frames * 960);
            }
            Some(_) => self.rtp.next_frame(),
            None => {}
        }
        self.last_sent = Some(now);
        let header = self.rtp.encode();
        if cipher.seal(&header, &frame, &mut self.packet).is_ok() {
            let _ = media.udp.send(&self.packet).await;
        }
    }

    fn receive_media(&mut self, n: usize, buf: &[u8]) {
        let packet = &buf[..n];
        if packet.len() < 8 {
            return;
        }
        // RTCP shares the socket; its packet types are 200 to 206.
        if (200..=206).contains(&packet[1]) {
            self.receive_rtcp(packet);
            return;
        }
        if packet.len() < RtpHeader::LEN {
            return;
        }
        let Some(cipher) = self.media.as_ref().and_then(|m| m.cipher.as_ref()) else {
            return;
        };
        let Some((header, payload)) = cipher.open(packet) else {
            return;
        };
        match header.payload_type {
            RtpHeader::OPUS => self.receive_audio(header, &payload),
            RtpHeader::H264 => self.receive_video(header, &payload),
            _ => {}
        }
    }

    fn receive_audio(&mut self, header: RtpHeader, payload: &[u8]) {
        let Some(sink) = &self.sink else {
            return;
        };
        let streamer = self.video.as_ref().map_or(0, |v| v.streamer);
        let user_id = self.users.get(&header.ssrc).copied().unwrap_or(streamer);
        if user_id == 0 && self.dave.protocol_version() > 0 {
            return; // Cannot pick the sender's key yet.
        }
        let Some(opus) = self.dave.decrypt_opus(user_id, payload) else {
            return;
        };
        let _ = sink.send(Inbound {
            user_id,
            ssrc: header.ssrc,
            sequence: header.sequence,
            opus: opus.into_owned(),
        });
    }

    fn receive_video(&mut self, header: RtpHeader, payload: &[u8]) {
        let Some(v) = &mut self.video else {
            return;
        };
        let Some(decoder) = &v.decoder else {
            return; // Sending only.
        };
        let user_id = v.senders.get(&header.ssrc).copied().unwrap_or(v.streamer);
        let assembled = v.reassembly.entry(header.ssrc).or_default().push(
            header.sequence,
            header.timestamp,
            header.marker,
            payload,
        );
        let frame = match assembled {
            Depacketized::Pending => return,
            Depacketized::Lost => {
                v.want_keyframe.store(true, Ordering::Relaxed);
                return;
            }
            Depacketized::Frame(f) => f,
        };
        let decoder = decoder.clone();
        let want_keyframe = v.want_keyframe.clone();
        match self.dave.decrypt_video(user_id, &frame) {
            Some(plain) => {
                // A busy decoder drops frames; the next keyframe recovers.
                if decoder.try_send(plain.into_owned()).is_err() {
                    want_keyframe.store(true, Ordering::Relaxed);
                }
            }
            None => want_keyframe.store(true, Ordering::Relaxed),
        }
    }

    /// Viewers ask for a keyframe with a picture loss indication or a full intra request.
    fn receive_rtcp(&mut self, packet: &[u8]) {
        let Some(v) = &self.video else {
            return;
        };
        if v.sending.is_none() || packet[1] != 206 {
            return;
        }
        let Some(cipher) = self.media.as_ref().and_then(|m| m.cipher.as_ref()) else {
            return;
        };
        if let Some(plain) = cipher.open_rtcp(packet)
            && matches!(plain[0] & 0x1f, 1 | 4)
        {
            v.keyframe_asked.store(true, Ordering::Relaxed);
        }
    }

    /// Asks the streamer for a keyframe when the picture broke, at most once a second.
    async fn request_keyframe(&mut self) {
        let Some(v) = &mut self.video else {
            return;
        };
        if v.decoder.is_none() || !v.want_keyframe.load(Ordering::Relaxed) {
            return;
        }
        if v.last_pli
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        let Some(media) = &mut self.media else {
            return;
        };
        let Some(cipher) = &mut media.cipher else {
            return;
        };
        let targets: Vec<u32> = v
            .senders
            .iter()
            .filter(|(_, u)| **u == v.streamer)
            .map(|(s, _)| *s)
            .collect();
        if targets.is_empty() {
            return; // The streamer's SSRC is not known yet.
        }
        for ssrc in targets {
            let (head, body) = rtp::rtcp_pli(media.ssrc, ssrc);
            if cipher.seal(&head, &body, &mut self.packet).is_ok() {
                let _ = media.udp.send(&self.packet).await;
            }
        }
        v.last_pli = Some(Instant::now());
        v.want_keyframe.store(false, Ordering::Relaxed);
    }

    /// After the session description: a sender announces its stream, a viewer asks for
    /// video.
    async fn announce_video(&mut self, ws: &mut Ws) -> Result<(), End> {
        let Some(v) = &self.video else {
            return Ok(());
        };
        let Some(media) = &self.media else {
            return Ok(());
        };
        if v.sending.is_some() {
            let speaking = json(
                VoiceOp::Speaking,
                &SpeakingOut {
                    speaking: protocol::speaking::SOUNDSHARE,
                    delay: 0,
                    ssrc: media.ssrc,
                },
            );
            ws.send(WsMessage::Text(speaking.into()))
                .await
                .map_err(|e| End::Resume {
                    code: None,
                    reason: e.to_string(),
                })?;
        }
        let msg = match v.sending {
            Some(q) => json(
                VoiceOp::Video,
                &VideoOut {
                    audio_ssrc: media.ssrc,
                    video_ssrc: v.ssrc,
                    rtx_ssrc: v.rtx_ssrc,
                    streams: vec![VideoStreamOut {
                        kind: "video",
                        rid: "100",
                        ssrc: v.ssrc,
                        active: true,
                        quality: 100,
                        rtx_ssrc: v.rtx_ssrc,
                        max_bitrate: q.bitrate_kbps * 1000,
                        max_framerate: q.fps,
                        max_resolution: Resolution {
                            kind: "fixed",
                            width: q.max_width,
                            height: q.max_height,
                        },
                    }],
                },
            ),
            None => json(VoiceOp::MediaSinkWants, &serde_json::json!({ "any": 100 })),
        };
        ws.send(WsMessage::Text(msg.into()))
            .await
            .map_err(|e| End::Resume {
                code: None,
                reason: e.to_string(),
            })
    }

    async fn send_video(&mut self, frame: EncodedFrame) {
        let Some(v) = &mut self.video else {
            return;
        };
        let Some(media) = &mut self.media else {
            return;
        };
        let Some(cipher) = &mut media.cipher else {
            return;
        };
        if v.ssrc == 0 {
            return;
        }
        let Some(data) = self.dave.encrypt_h264(&frame.data) else {
            v.keyframe_asked.store(true, Ordering::Relaxed);
            return; // Keys not ready; send a keyframe once they are.
        };
        // 90 kHz media clock from wall time.
        let elapsed = v.clock.elapsed().as_micros() as u64;
        let timestamp = (elapsed * 9 / 100) as u32;
        let payloads = h264::packetize(&data, h264::MAX_PAYLOAD);
        let last = payloads.len().saturating_sub(1);
        let mut body = Vec::with_capacity(h264::MAX_PAYLOAD + 4);
        for (i, p) in payloads.iter().enumerate() {
            v.rtp.sequence = v.rtp.sequence.wrapping_add(1);
            let header = RtpHeader {
                marker: i == last,
                timestamp: v.rtp.timestamp.wrapping_add(timestamp),
                ..v.rtp
            }
            .encode_video();
            body.clear();
            body.extend_from_slice(&rtp::PLAYOUT_DELAY);
            body.extend_from_slice(p);
            if cipher.seal(&header, &body, &mut self.packet).is_ok() {
                let _ = media.udp.send(&self.packet).await;
            }
        }
    }
}

async fn recv_udp(udp: Option<&UdpSocket>, buf: &mut [u8]) -> std::io::Result<usize> {
    match udp {
        Some(s) => s.recv(buf).await,
        None => std::future::pending().await,
    }
}

/// Opens the media socket and asks the server for our public address, retrying a few times
/// since UDP may drop the first packets.
async fn discover(ready: &Ready) -> std::io::Result<(UdpSocket, IpDiscovery)> {
    let addr: SocketAddr = tokio::net::lookup_host((ready.ip.as_str(), ready.port))
        .await?
        .next()
        .ok_or_else(|| std::io::Error::other("voice server address did not resolve"))?;
    let bind: SocketAddr = if addr.is_ipv4() {
        ([0, 0, 0, 0], 0).into()
    } else {
        ([0u16; 8], 0).into()
    };
    let udp = UdpSocket::bind(bind).await?;
    udp.connect(addr).await?;
    let request = IpDiscovery::request(ready.ssrc);
    let mut buf = [0u8; 128];
    for _ in 0..5 {
        udp.send(&request).await?;
        if let Ok(Ok(n)) = timeout(Duration::from_secs(1), udp.recv(&mut buf)).await
            && let Some(me) = IpDiscovery::parse_response(&buf[..n])
        {
            return Ok((udp, me));
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "no answer to IP discovery",
    ))
}

async fn send_replies(ws: &mut Ws, replies: Vec<Reply>) -> Result<(), End> {
    for r in replies {
        let msg = match r {
            Reply::Json(s) => WsMessage::Text(s.into()),
            Reply::Binary(b) => WsMessage::Binary(b.into()),
        };
        ws.send(msg).await.map_err(|e| End::Resume {
            code: None,
            reason: e.to_string(),
        })?;
    }
    Ok(())
}

fn json<T: serde::Serialize>(op: VoiceOp, d: &T) -> String {
    serde_json::to_string(&Outgoing::new(op, d)).unwrap_or_default()
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn describe_close(code: u16, reason: String) -> String {
    match code {
        close::DISCONNECTED => "Disconnected from the voice channel.".into(),
        close::E2EE_REQUIRED => {
            "This call requires end-to-end encryption that could not be set up.".into()
        }
        close::CALL_TERMINATED => "The call ended.".into(),
        close::SESSION_NO_LONGER_VALID | close::SESSION_TIMEOUT => {
            "The voice session expired. Join again.".into()
        }
        _ => reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_reasons_are_readable() {
        assert_eq!(
            describe_close(close::DISCONNECTED, String::new()),
            "Disconnected from the voice channel."
        );
        assert_eq!(describe_close(4999, "x".into()), "x");
    }
}
