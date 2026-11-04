//! One voice connection: the voice gateway WebSocket, the UDP media socket, transport and
//! end-to-end encryption, and the hand-off to the media loop.
//!
//! Joining goes: Hello, Identify, Ready (our SSRC and the media server's address), IP
//! discovery over UDP, Select Protocol with the best encryption mode both sides support, then
//! Session Description with the transport key and the DAVE protocol version. After that,
//! microphone frames go out and everyone else's come in. A dropped socket resumes with the
//! last acknowledged sequence number, keeping the UDP socket and keys.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::{MissedTickBehavior, interval_at, sleep, timeout};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::audio::{AudioEngine, AudioEvent, AudioSettings, Inbound, Outbound, SILENCE};
use crate::crypto::TransportCipher;
use crate::dave::{self, Dave, Reply};
use crate::protocol::{
    self, ClientDisconnect, ClientsConnect, DaveExecuteTransition, DavePrepareEpoch,
    DavePrepareTransition, EncryptionMode, HeartbeatAck, Hello, Identify, Incoming, Outgoing,
    Ready, Resume, SelectProtocol, SelectProtocolData, SessionDescription, SpeakingIn, SpeakingOut,
    VOICE_GATEWAY_VERSION, VoiceOp, close,
};
use crate::rtp::{IpDiscovery, RtpHeader};

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
        let mut conn = Connection::new(config, sink, event_tx.clone());
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

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// State that survives a resume.
struct Media {
    udp: Arc<UdpSocket>,
    ssrc: u32,
    cipher: Option<TransportCipher>,
}

struct Connection {
    config: VoiceConfig,
    sink: std::sync::mpsc::Sender<Inbound>,
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
        config: VoiceConfig,
        sink: std::sync::mpsc::Sender<Inbound>,
        events: async_channel::Sender<VoiceEvent>,
    ) -> Self {
        let dave = Dave::new(config.user_id, config.channel_id);
        Self {
            config,
            sink,
            events,
            media: None,
            dave,
            users: HashMap::new(),
            seq_ack: -1,
            rtp: RtpHeader {
                payload_type: RtpHeader::OPUS,
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
                    video: false,
                    streams: Vec::new(),
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
                received = recv_udp(udp.as_deref(), &mut buf) => {
                    if let Ok(n) = received {
                        self.receive_media(n, &buf);
                    }
                }
                out = audio.recv() => {
                    match out {
                        Some(Outbound::Opus(frame)) => self.send_media(&frame).await,
                        Some(Outbound::Speaking(on)) => {
                            if let Some(m) = &self.media {
                                let msg = json(VoiceOp::Speaking, &SpeakingOut {
                                    speaking: if on { protocol::speaking::MICROPHONE } else { 0 },
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
                self.connected();
            }
            VoiceOp::Resumed => self.connected(),
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
        // RTCP shares the socket; its packet types are 200 to 204.
        if packet.len() < RtpHeader::LEN || (200..=204).contains(&packet[1]) {
            return;
        }
        let Some(cipher) = self.media.as_ref().and_then(|m| m.cipher.as_ref()) else {
            return;
        };
        let Some((header, payload)) = cipher.open(packet) else {
            return;
        };
        if header.payload_type != RtpHeader::OPUS {
            return;
        }
        let user_id = self.users.get(&header.ssrc).copied().unwrap_or(0);
        if user_id == 0 && self.dave.protocol_version() > 0 {
            return; // Cannot pick the sender's key yet.
        }
        let Some(opus) = self.dave.decrypt_opus(user_id, &payload) else {
            return;
        };
        let _ = self.sink.send(Inbound {
            user_id,
            ssrc: header.ssrc,
            sequence: header.sequence,
            opus: opus.into_owned(),
        });
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
