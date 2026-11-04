//! The Discord gateway: one WebSocket per account, zlib-stream compressed, with heartbeats,
//! resume, and reconnect with backoff. Events are handed on with their payload still raw,
//! so the cache parses each one exactly once into its own type.

mod inflate;
pub mod protocol;

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use rand::Rng;
use serde_json::value::RawValue;
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep, timeout};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

pub use inflate::ZlibStream;
pub use protocol::{
    CHROME_VERSION, Capabilities, ClientProperties, Command, GuildSubscription,
    GuildSubscriptionsBulk, PresenceUpdate, RequestGuildMembers, StreamCreate, StreamKey,
    StreamSetPaused, VoiceStateUpdate,
};
use protocol::{ClientState, Hello, Identify, Incoming, Outgoing, ReadySession, Resume, op};

pub const DEFAULT_URL: &str = "wss://gateway.discord.gg";
pub const API_VERSION: u8 = 9;

/// An account token. Its `Debug` form is redacted so it never reaches a log.
#[derive(Clone)]
pub struct Token(Arc<str>);

impl Token {
    pub fn new(token: impl Into<Arc<str>>) -> Self {
        Self(token.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub token: Token,
    pub properties: ClientProperties,
    pub presence: PresenceUpdate,
    pub capabilities: Capabilities,
    pub url: String,
}

/// A dispatch event: its name and its payload as received.
#[derive(Debug)]
pub struct Dispatch {
    pub name: Box<str>,
    pub data: Box<RawValue>,
}

#[derive(Debug)]
pub enum Event {
    /// The socket is open and IDENTIFY or RESUME was sent.
    Connecting {
        resuming: bool,
    },
    Dispatch(Dispatch),
    /// The connection dropped. `reconnecting` is false for fatal close codes, such as an
    /// invalid token, after which the gateway stops.
    Disconnected {
        code: Option<u16>,
        reason: String,
        reconnecting: bool,
    },
    /// Heartbeat round-trip time, measured on each acknowledgement.
    Latency(Duration),
}

/// Sends commands to a running gateway. Dropping every handle shuts it down.
#[derive(Debug, Clone)]
pub struct Handle {
    commands: mpsc::UnboundedSender<Command>,
}

impl Handle {
    /// Queues a command. Commands sent while reconnecting are delivered once connected.
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

/// Starts the gateway on the current Tokio runtime.
pub fn spawn(config: Config) -> (Handle, async_channel::Receiver<Event>) {
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = async_channel::unbounded();
    tokio::spawn(run(config, command_rx, event_tx));
    (
        Handle {
            commands: command_tx,
        },
        event_rx,
    )
}

struct Session {
    id: String,
    resume_url: String,
    seq: u64,
}

enum Exit {
    /// Reconnect, resuming if a session is kept.
    Reconnect { code: Option<u16>, reason: String },
    /// Stop for good.
    Fatal { code: Option<u16>, reason: String },
    /// Every handle was dropped.
    Shutdown,
}

/// Close codes after which reconnecting cannot help.
fn is_fatal(code: u16) -> bool {
    matches!(code, 4004 | 4010 | 4011 | 4012 | 4013 | 4014)
}

/// Close codes that end the session, so the next connection identifies again.
fn invalidates_session(code: u16) -> bool {
    matches!(code, 4007 | 4009)
}

async fn run(
    config: Config,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: async_channel::Sender<Event>,
) {
    let mut session: Option<Session> = None;
    let mut attempt: u32 = 0;
    let mut queued: Vec<Command> = Vec::new();

    loop {
        let base = session
            .as_ref()
            .map_or(config.url.as_str(), |s| s.resume_url.as_str());
        let url = format!(
            "{}/?v={API_VERSION}&encoding=json&compress=zlib-stream",
            base.trim_end_matches('/')
        );
        let started = Instant::now();
        let exit = connect_once(
            &config,
            &url,
            &mut session,
            &mut commands,
            &mut queued,
            &events,
        )
        .await;
        let (code, reason, reconnecting) = match exit {
            Exit::Shutdown => return,
            Exit::Fatal { code, reason } => (code, reason, false),
            Exit::Reconnect { code, reason } => (code, reason, true),
        };
        if code.is_some_and(invalidates_session) {
            session = None;
        }
        if events
            .send(Event::Disconnected {
                code,
                reason,
                reconnecting,
            })
            .await
            .is_err()
            || !reconnecting
        {
            return;
        }
        // A connection that stayed up for a while resets the backoff.
        if started.elapsed() > Duration::from_secs(60) {
            attempt = 0;
        }
        let delay = backoff(attempt);
        attempt = attempt.saturating_add(1);
        tokio::select! {
            () = sleep(delay) => {}
            closed = wait_for_close(&mut commands, &mut queued) => if closed { return },
        }
    }
}

/// Collects commands while waiting to reconnect. Returns true when every handle is gone.
async fn wait_for_close(
    commands: &mut mpsc::UnboundedReceiver<Command>,
    queued: &mut Vec<Command>,
) -> bool {
    loop {
        match commands.recv().await {
            Some(c) => queued.push(c),
            None => return true,
        }
    }
}

fn backoff(attempt: u32) -> Duration {
    let base_ms = 1_000u64.saturating_mul(1 << attempt.min(6)).min(60_000);
    let jitter = rand::rng().random_range(0..=base_ms / 4);
    Duration::from_millis(base_ms + jitter)
}

async fn connect_once(
    config: &Config,
    url: &str,
    session: &mut Option<Session>,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    queued: &mut Vec<Command>,
    events: &async_channel::Sender<Event>,
) -> Exit {
    let reconnect = |reason: String| Exit::Reconnect { code: None, reason };

    let ws = match timeout(
        Duration::from_secs(20),
        tokio_tungstenite::connect_async(url),
    )
    .await
    {
        Ok(Ok((ws, _))) => ws,
        Ok(Err(e)) => return reconnect(format!("connect failed: {e}")),
        Err(_) => return reconnect("connect timed out".into()),
    };
    let (mut sink, mut stream) = ws.split();
    let mut zlib = ZlibStream::new();

    // HELLO comes first and sets the heartbeat interval.
    let interval = loop {
        let frame = match timeout(Duration::from_secs(20), stream.next()).await {
            Ok(Some(Ok(frame))) => frame,
            Ok(Some(Err(e))) => return reconnect(format!("read failed: {e}")),
            Ok(None) => return reconnect("closed before hello".into()),
            Err(_) => return reconnect("no hello".into()),
        };
        let text = match decode(&mut zlib, &frame) {
            Ok(Some(text)) => text,
            Ok(None) => continue,
            Err(e) => return reconnect(e),
        };
        let Ok(payload) = serde_json::from_slice::<Incoming<'_>>(&text) else {
            return reconnect("malformed hello".into());
        };
        if payload.op != op::HELLO {
            continue;
        }
        match payload.d.map(|d| serde_json::from_str::<Hello>(d.get())) {
            Some(Ok(hello)) => break Duration::from_millis(hello.heartbeat_interval),
            _ => return reconnect("malformed hello".into()),
        }
    };

    let resuming = session.is_some();
    let hello_reply = match session {
        Some(s) => serde_json::to_string(&Outgoing {
            op: op::RESUME,
            d: Resume {
                token: config.token.expose(),
                session_id: &s.id,
                seq: s.seq,
            },
        }),
        None => serde_json::to_string(&Outgoing {
            op: op::IDENTIFY,
            d: Identify {
                token: config.token.expose(),
                capabilities: config.capabilities.bits(),
                properties: &config.properties,
                presence: &config.presence,
                compress: false,
                client_state: ClientState::default(),
            },
        }),
    };
    let Ok(hello_reply) = hello_reply else {
        return Exit::Fatal {
            code: None,
            reason: "could not encode identify".into(),
        };
    };
    if let Err(e) = sink.send(WsMessage::Text(hello_reply.into())).await {
        return reconnect(format!("send failed: {e}"));
    }
    if events.send(Event::Connecting { resuming }).await.is_err() {
        return Exit::Shutdown;
    }

    // The first heartbeat goes out after a random fraction of the interval, so clients
    // that reconnect together do not beat together.
    let first = interval.mul_f64(rand::rng().random_range(0.0..1.0));

    let mut heartbeat = interval_at(Instant::now() + first, interval);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut awaiting_ack = false;
    let mut sent_at = Instant::now();
    let mut ready = resuming;
    let mut pending: Vec<Command> = std::mem::take(queued);

    loop {
        tokio::select! {
            biased;

            _ = heartbeat.tick() => {
                if awaiting_ack {
                    // A missed acknowledgement means a zombie connection.
                    let _ = sink.send(WsMessage::Close(Some(CloseFrame {
                        code: CloseCode::Library(4000),
                        reason: "heartbeat timeout".into(),
                    }))).await;
                    return reconnect("heartbeat not acknowledged".into());
                }
                let seq = session.as_ref().map(|s| s.seq);
                let beat = serde_json::to_string(&Outgoing { op: op::HEARTBEAT, d: seq })
                    .unwrap_or_else(|_| r#"{"op":1,"d":null}"#.into());
                if let Err(e) = sink.send(WsMessage::Text(beat.into())).await {
                    return reconnect(format!("send failed: {e}"));
                }
                awaiting_ack = true;
                sent_at = Instant::now();
            }

            frame = stream.next() => {
                let frame = match frame {
                    Some(Ok(frame)) => frame,
                    Some(Err(e)) => return reconnect(format!("read failed: {e}")),
                    None => return reconnect("connection closed".into()),
                };
                if let WsMessage::Close(close) = &frame {
                    let code = close.as_ref().map(|c| u16::from(c.code));
                    let reason = close.as_ref().map(|c| c.reason.to_string()).unwrap_or_default();
                    return match code {
                        Some(c) if is_fatal(c) => Exit::Fatal { code, reason },
                        _ => Exit::Reconnect { code, reason },
                    };
                }
                let text = match decode(&mut zlib, &frame) {
                    Ok(Some(text)) => text,
                    Ok(None) => continue,
                    Err(e) => return reconnect(e),
                };
                let payload = match serde_json::from_slice::<Incoming<'_>>(&text) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!("skipping malformed gateway payload: {e}");
                        continue;
                    }
                };
                match payload.op {
                    op::DISPATCH => {
                        if let (Some(s), Some(seq)) = (session.as_mut(), payload.s) {
                            s.seq = s.seq.max(seq);
                        }
                        let (Some(name), Some(data)) = (payload.t, payload.d) else { continue };
                        let is_ready = name == "READY";
                        if is_ready {
                            match serde_json::from_str::<ReadySession>(data.get()) {
                                Ok(r) => {
                                    *session = Some(Session {
                                        id: r.session_id,
                                        resume_url: r.resume_gateway_url.unwrap_or_else(|| config.url.clone()),
                                        seq: payload.s.unwrap_or(0),
                                    });
                                }
                                Err(e) => tracing::warn!("READY without a session: {e}"),
                            }
                        }
                        if is_ready || name == "RESUMED" {
                            ready = true;
                            for command in pending.drain(..) {
                                if let Ok(text) = command.encode() {
                                    let _ = sink.send(WsMessage::Text(text.into())).await;
                                }
                            }
                        }
                        let event = Event::Dispatch(Dispatch {
                            name: name.into(),
                            data: data.to_owned(),
                        });
                        if events.send(event).await.is_err() {
                            return Exit::Shutdown;
                        }
                        if is_ready {
                            zlib.trim();
                        }
                    }
                    op::HEARTBEAT => {
                        heartbeat.reset_immediately();
                        awaiting_ack = false;
                    }
                    op::HEARTBEAT_ACK => {
                        awaiting_ack = false;
                        let _ = events.try_send(Event::Latency(sent_at.elapsed()));
                    }
                    op::RECONNECT => return reconnect("server asked to reconnect".into()),
                    op::INVALID_SESSION => {
                        let resumable = payload
                            .d
                            .and_then(|d| serde_json::from_str::<bool>(d.get()).ok())
                            .unwrap_or(false);
                        if !resumable {
                            *session = None;
                        }
                        // Discord asks for a short random wait before identifying again.
                        let wait = Duration::from_millis(rand::rng().random_range(1_000..5_000));
                        sleep(wait).await;
                        return reconnect("invalid session".into());
                    }
                    _ => {}
                }
            }

            command = commands.recv() => {
                let Some(command) = command else {
                    let _ = sink.send(WsMessage::Close(Some(CloseFrame {
                        code: CloseCode::Normal,
                        reason: "".into(),
                    }))).await;
                    return Exit::Shutdown;
                };
                if !ready {
                    pending.push(command);
                    continue;
                }
                match command.encode() {
                    Ok(text) => {
                        if let Err(e) = sink.send(WsMessage::Text(text.into())).await {
                            queued.push(command);
                            return reconnect(format!("send failed: {e}"));
                        }
                    }
                    Err(e) => tracing::warn!("could not encode gateway command: {e}"),
                }
            }
        }
    }
}

/// Returns the JSON text of a frame, or `None` while a compressed message is incomplete.
fn decode<'z>(
    zlib: &'z mut ZlibStream,
    frame: &'z WsMessage,
) -> Result<Option<std::borrow::Cow<'z, [u8]>>, String> {
    match frame {
        WsMessage::Binary(bytes) => zlib
            .push(bytes)
            .map(|o| o.map(std::borrow::Cow::Borrowed))
            .map_err(|e| format!("decompression failed: {e}")),
        WsMessage::Text(text) => Ok(Some(std::borrow::Cow::Borrowed(text.as_bytes()))),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        assert!(backoff(0) >= Duration::from_secs(1));
        assert!(backoff(0) <= Duration::from_millis(1_250));
        assert!(backoff(20) >= Duration::from_secs(60));
        assert!(backoff(20) <= Duration::from_secs(75));
    }

    #[test]
    fn close_codes() {
        assert!(is_fatal(4004));
        assert!(!is_fatal(4000));
        assert!(invalidates_session(4009));
    }

    #[test]
    fn token_debug_is_redacted() {
        assert_eq!(format!("{:?}", Token::new("secret")), "Token(<redacted>)");
    }
}
