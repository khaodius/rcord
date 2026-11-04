use serde::{Deserialize, Serialize};

/// Voice gateway version this crate speaks.
pub const VOICE_GATEWAY_VERSION: u8 = 8;

/// Voice gateway opcodes, including the DAVE (end-to-end encryption) handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum VoiceOp {
    Identify = 0,
    SelectProtocol = 1,
    Ready = 2,
    Heartbeat = 3,
    SessionDescription = 4,
    Speaking = 5,
    HeartbeatAck = 6,
    Resume = 7,
    Hello = 8,
    Resumed = 9,
    ClientsConnect = 11,
    Video = 12,
    ClientDisconnect = 13,
    MediaSinkWants = 15,
    DavePrepareTransition = 21,
    DaveExecuteTransition = 22,
    DaveTransitionReady = 23,
    DavePrepareEpoch = 24,
    DaveMlsExternalSender = 25,
    DaveMlsKeyPackage = 26,
    DaveMlsProposals = 27,
    DaveMlsCommitWelcome = 28,
    DaveMlsAnnounceCommitTransition = 29,
    DaveMlsWelcome = 30,
    DaveMlsInvalidCommitWelcome = 31,
}

impl VoiceOp {
    pub fn from_u8(v: u8) -> Option<Self> {
        use VoiceOp::*;
        Some(match v {
            0 => Identify,
            1 => SelectProtocol,
            2 => Ready,
            3 => Heartbeat,
            4 => SessionDescription,
            5 => Speaking,
            6 => HeartbeatAck,
            7 => Resume,
            8 => Hello,
            9 => Resumed,
            11 => ClientsConnect,
            12 => Video,
            13 => ClientDisconnect,
            15 => MediaSinkWants,
            21 => DavePrepareTransition,
            22 => DaveExecuteTransition,
            23 => DaveTransitionReady,
            24 => DavePrepareEpoch,
            25 => DaveMlsExternalSender,
            26 => DaveMlsKeyPackage,
            27 => DaveMlsProposals,
            28 => DaveMlsCommitWelcome,
            29 => DaveMlsAnnounceCommitTransition,
            30 => DaveMlsWelcome,
            31 => DaveMlsInvalidCommitWelcome,
            _ => return None,
        })
    }

    /// DAVE's MLS messages travel as binary frames: sequence, opcode, then payload.
    pub const fn is_binary(self) -> bool {
        matches!(
            self,
            Self::DaveMlsExternalSender
                | Self::DaveMlsKeyPackage
                | Self::DaveMlsProposals
                | Self::DaveMlsCommitWelcome
                | Self::DaveMlsAnnounceCommitTransition
                | Self::DaveMlsWelcome
        )
    }
}

/// Transport encryption modes, best first. Discord has retired the older modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EncryptionMode {
    #[serde(rename = "aead_aes256_gcm_rtpsize")]
    Aes256Gcm,
    #[serde(rename = "aead_xchacha20_poly1305_rtpsize")]
    XChaCha20Poly1305,
}

impl EncryptionMode {
    pub const PREFERENCE: [Self; 2] = [Self::Aes256Gcm, Self::XChaCha20Poly1305];

    /// Picks the first preferred mode the server offers.
    pub fn choose(offered: &[String]) -> Option<Self> {
        Self::PREFERENCE.into_iter().find(|m| {
            let name = serde_json::to_value(m).ok();
            offered
                .iter()
                .any(|o| name.as_ref().and_then(|n| n.as_str()) == Some(o.as_str()))
        })
    }

    pub const fn tag_len(self) -> usize {
        16
    }

    /// Both modes append a 32-bit counter to the packet and expand it to the full nonce.
    pub const fn nonce_suffix_len(self) -> usize {
        4
    }
}

/// A text frame from the voice gateway. Since v8 most frames carry a sequence number that the
/// client acknowledges in heartbeats and resumes.
#[derive(Debug, Deserialize)]
pub struct Incoming {
    pub op: u8,
    #[serde(default)]
    pub d: serde_json::Value,
    #[serde(default)]
    pub seq: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct Outgoing<T> {
    pub op: u8,
    pub d: T,
}

impl<T: Serialize> Outgoing<T> {
    pub fn new(op: VoiceOp, d: T) -> Self {
        Self { op: op as u8, d }
    }
}

/// Splits a binary frame from the server into its sequence number, opcode and payload.
pub fn parse_binary(frame: &[u8]) -> Option<(u16, VoiceOp, &[u8])> {
    let (head, payload) = frame.split_at_checked(3)?;
    let seq = u16::from_be_bytes([head[0], head[1]]);
    Some((seq, VoiceOp::from_u8(head[2])?, payload))
}

/// Builds a binary frame for the server: opcode, then payload. Clients send no sequence.
pub fn encode_binary(op: VoiceOp, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 1);
    out.push(op as u8);
    out.extend_from_slice(payload);
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct Identify<'a> {
    pub server_id: &'a str,
    pub user_id: &'a str,
    pub session_id: &'a str,
    pub token: &'a str,
    pub video: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub streams: Vec<StreamDescriptor>,
    /// Highest DAVE protocol version supported; 0 refuses end-to-end encryption.
    pub max_dave_protocol_version: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct Resume<'a> {
    pub server_id: &'a str,
    pub session_id: &'a str,
    pub token: &'a str,
    pub seq_ack: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamDescriptor {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub rid: &'static str,
    pub quality: u8,
}

impl StreamDescriptor {
    /// The one full-quality layer of a Go Live stream.
    pub const SCREEN: Self = Self {
        kind: "screen",
        rid: "100",
        quality: 100,
    };
}

#[derive(Debug, Clone, Deserialize)]
pub struct Hello {
    pub heartbeat_interval: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Heartbeat {
    /// A nonce the server echoes back, here the send time in milliseconds.
    pub t: u64,
    /// The last sequence number received, or -1 before any.
    pub seq_ack: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HeartbeatAck {
    #[serde(default)]
    pub t: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Ready {
    pub ssrc: u32,
    pub ip: String,
    pub port: u16,
    pub modes: Vec<String>,
    /// The SSRCs reserved for video, one per requested stream layer.
    #[serde(default)]
    pub streams: Vec<ReadyStream>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReadyStream {
    pub ssrc: u32,
    #[serde(default)]
    pub rtx_ssrc: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SelectProtocol {
    pub protocol: &'static str,
    pub data: SelectProtocolData,
    /// The codecs this client sends and receives; needed for video.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub codecs: Vec<Codec>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Codec {
    pub name: &'static str,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub priority: u32,
    pub payload_type: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtx_payload_type: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encode: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decode: Option<bool>,
}

impl Codec {
    pub const OPUS: Self = Self {
        name: "opus",
        kind: "audio",
        priority: 1000,
        payload_type: crate::rtp::RtpHeader::OPUS,
        rtx_payload_type: None,
        encode: None,
        decode: None,
    };
    pub const H264: Self = Self {
        name: "H264",
        kind: "video",
        priority: 1000,
        payload_type: crate::rtp::RtpHeader::H264,
        rtx_payload_type: Some(crate::rtp::RtpHeader::H264_RTX),
        encode: Some(true),
        decode: Some(true),
    };
}

/// Announces (or, with zero SSRCs and no streams, withdraws) this client's video.
#[derive(Debug, Clone, Serialize)]
pub struct VideoOut {
    pub audio_ssrc: u32,
    pub video_ssrc: u32,
    pub rtx_ssrc: u32,
    pub streams: Vec<VideoStreamOut>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VideoStreamOut {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub rid: &'static str,
    pub ssrc: u32,
    pub active: bool,
    pub quality: u8,
    pub rtx_ssrc: u32,
    pub max_bitrate: u32,
    pub max_framerate: u32,
    pub max_resolution: Resolution,
}

#[derive(Debug, Clone, Serialize)]
pub struct Resolution {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub width: u32,
    pub height: u32,
}

/// Another participant's video SSRCs.
#[derive(Debug, Clone, Deserialize)]
pub struct VideoIn {
    #[serde(deserialize_with = "id_string")]
    pub user_id: u64,
    #[serde(default)]
    pub audio_ssrc: u32,
    #[serde(default)]
    pub video_ssrc: u32,
    #[serde(default)]
    pub streams: Vec<VideoStreamIn>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VideoStreamIn {
    pub ssrc: u32,
    #[serde(default)]
    pub active: bool,
    /// The layer's quality, 0 to 100; a simulcast sender's preview layer is lower.
    #[serde(default)]
    pub quality: Option<u32>,
    #[serde(default)]
    pub max_resolution: Option<ResolutionIn>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ResolutionIn {
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

impl VideoIn {
    /// The layer to watch: the highest quality active one, then the largest. Returns its
    /// SSRC and pixel count (0 when the server did not say).
    pub fn best_layer(&self) -> Option<(u32, u32)> {
        let pixels = |s: &VideoStreamIn| {
            s.max_resolution
                .map_or(0, |r| r.width.saturating_mul(r.height))
        };
        self.streams
            .iter()
            .filter(|s| s.ssrc != 0 && s.active)
            .max_by_key(|s| (s.quality.unwrap_or(100), pixels(s)))
            .map(|s| (s.ssrc, pixels(s)))
            .or((self.video_ssrc != 0).then_some((self.video_ssrc, 0)))
    }
}

/// What this client wants to receive: full quality from `ssrc` at `pixels` on screen, and
/// nothing from the sender's other layers.
pub fn sink_wants(ssrc: u32, pixels: u32, others: &[u32]) -> serde_json::Value {
    let mut streams = serde_json::Map::new();
    for o in others.iter().filter(|o| **o != ssrc) {
        streams.insert(o.to_string(), 0.into());
    }
    streams.insert(ssrc.to_string(), 100.into());
    serde_json::json!({
        "any": 100,
        "streams": streams,
        "pixelCounts": { ssrc.to_string(): pixels },
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct SelectProtocolData {
    pub address: String,
    pub port: u16,
    pub mode: EncryptionMode,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionDescription {
    pub mode: EncryptionMode,
    pub secret_key: Vec<u8>,
    #[serde(default)]
    pub dave_protocol_version: u16,
}

/// What a sender is transmitting, as bit flags.
pub mod speaking {
    pub const MICROPHONE: u32 = 1 << 0;
    pub const SOUNDSHARE: u32 = 1 << 1;
    pub const PRIORITY: u32 = 1 << 2;
}

#[derive(Debug, Clone, Serialize)]
pub struct SpeakingOut {
    pub speaking: u32,
    pub delay: u32,
    pub ssrc: u32,
}

/// Sent by the server when another participant's SSRC becomes known or their speaking
/// flags change.
#[derive(Debug, Clone, Deserialize)]
pub struct SpeakingIn {
    #[serde(deserialize_with = "id_string")]
    pub user_id: u64,
    pub ssrc: u32,
    #[serde(default)]
    pub speaking: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientsConnect {
    #[serde(deserialize_with = "id_strings")]
    pub user_ids: Vec<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientDisconnect {
    #[serde(deserialize_with = "id_string")]
    pub user_id: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DavePrepareTransition {
    pub transition_id: u16,
    #[serde(default)]
    pub protocol_version: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DaveExecuteTransition {
    pub transition_id: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DavePrepareEpoch {
    pub epoch: u64,
    pub protocol_version: u16,
}

/// Body of `DaveTransitionReady` and `DaveMlsInvalidCommitWelcome`.
#[derive(Debug, Clone, Serialize)]
pub struct TransitionId {
    pub transition_id: u16,
}

/// Voice gateway close codes rcord acts on.
pub mod close {
    /// The session is no longer valid; a fresh voice state update is needed.
    pub const SESSION_NO_LONGER_VALID: u16 = 4006;
    pub const SESSION_TIMEOUT: u16 = 4009;
    pub const SERVER_NOT_FOUND: u16 = 4011;
    pub const UNKNOWN_PROTOCOL: u16 = 4012;
    /// Kicked, moved to another channel, or the channel was deleted.
    pub const DISCONNECTED: u16 = 4014;
    /// The voice server crashed; resuming works.
    pub const VOICE_SERVER_CRASHED: u16 = 4015;
    pub const UNKNOWN_ENCRYPTION_MODE: u16 = 4016;
    /// The call requires end-to-end encryption this client did not offer.
    pub const E2EE_REQUIRED: u16 = 4017;
    pub const BAD_REQUEST: u16 = 4020;
    pub const RATE_LIMITED: u16 = 4021;
    /// The call was ended by its owner.
    pub const CALL_TERMINATED: u16 = 4022;

    /// Whether a new connection may resume the session after this close.
    pub const fn can_resume(code: u16) -> bool {
        !matches!(
            code,
            4004 | SESSION_NO_LONGER_VALID
                | SESSION_TIMEOUT
                | SERVER_NOT_FOUND
                | UNKNOWN_PROTOCOL
                | DISCONNECTED
                | UNKNOWN_ENCRYPTION_MODE
                | E2EE_REQUIRED
                | BAD_REQUEST
                | CALL_TERMINATED
        )
    }
}

fn id_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        S(String),
        N(u64),
    }
    match Id::deserialize(d)? {
        Id::S(s) => s.parse().map_err(serde::de::Error::custom),
        Id::N(n) => Ok(n),
    }
}

fn id_strings<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
    let raw = Vec::<String>::deserialize(d)?;
    raw.iter()
        .map(|s| s.parse().map_err(serde::de::Error::custom))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watches_the_best_simulcast_layer() {
        let info: VideoIn = serde_json::from_value(serde_json::json!({
            "user_id": "5",
            "audio_ssrc": 1,
            "video_ssrc": 2,
            "streams": [
                {"ssrc": 2, "active": true, "quality": 50,
                 "max_resolution": {"type": "fixed", "width": 640, "height": 360}},
                {"ssrc": 4, "active": true, "quality": 100,
                 "max_resolution": {"type": "source", "width": 0, "height": 0}},
                {"ssrc": 6, "active": false, "quality": 100}
            ]
        }))
        .unwrap();
        assert_eq!(info.best_layer(), Some((4, 0)));
        let wants = sink_wants(4, 1920 * 1080, &[2, 4, 6]);
        assert_eq!(wants["any"], 100);
        assert_eq!(wants["streams"]["4"], 100);
        assert_eq!(wants["streams"]["2"], 0);
        assert_eq!(wants["pixelCounts"]["4"], 1920 * 1080);
    }

    #[test]
    fn opcodes_round_trip() {
        for v in 0..=40 {
            if let Some(op) = VoiceOp::from_u8(v) {
                assert_eq!(op as u8, v);
            }
        }
        assert!(VoiceOp::DaveMlsWelcome.is_binary());
        assert!(!VoiceOp::DavePrepareEpoch.is_binary());
    }

    #[test]
    fn chooses_best_offered_mode() {
        let offered = vec![
            "aead_xchacha20_poly1305_rtpsize".to_owned(),
            "aead_aes256_gcm_rtpsize".to_owned(),
            "xsalsa20_poly1305".to_owned(),
        ];
        assert_eq!(
            EncryptionMode::choose(&offered),
            Some(EncryptionMode::Aes256Gcm)
        );
        assert_eq!(EncryptionMode::choose(&["xsalsa20_poly1305".into()]), None);
    }

    #[test]
    fn select_protocol_lists_codecs() {
        let s = SelectProtocol {
            protocol: "udp",
            data: SelectProtocolData {
                address: "1.2.3.4".into(),
                port: 5,
                mode: EncryptionMode::Aes256Gcm,
            },
            codecs: vec![Codec::OPUS, Codec::H264],
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["codecs"][0]["name"], "opus");
        assert!(v["codecs"][0].get("rtx_payload_type").is_none());
        assert_eq!(v["codecs"][1]["payload_type"], 101);
        assert_eq!(v["codecs"][1]["rtx_payload_type"], 102);
        assert_eq!(v["codecs"][1]["type"], "video");
    }

    #[test]
    fn binary_frames() {
        let frame = [0x01, 0x02, 25, 0xaa, 0xbb];
        let (seq, op, payload) = parse_binary(&frame).unwrap();
        assert_eq!(seq, 0x0102);
        assert_eq!(op, VoiceOp::DaveMlsExternalSender);
        assert_eq!(payload, &[0xaa, 0xbb]);
        assert_eq!(parse_binary(&[0, 1]), None);
        assert_eq!(encode_binary(VoiceOp::DaveMlsKeyPackage, &[9]), vec![26, 9]);
    }

    #[test]
    fn reads_server_payloads() {
        let s: SpeakingIn =
            serde_json::from_str(r#"{"user_id":"80351110224678912","ssrc":42,"speaking":1}"#)
                .unwrap();
        assert_eq!(
            (s.user_id, s.ssrc, s.speaking),
            (80_351_110_224_678_912, 42, 1)
        );
        let c: ClientsConnect = serde_json::from_str(r#"{"user_ids":["1","2"]}"#).unwrap();
        assert_eq!(c.user_ids, vec![1, 2]);
        let m: Incoming =
            serde_json::from_str(r#"{"op":8,"d":{"heartbeat_interval":13750.5},"seq":3}"#).unwrap();
        assert_eq!((m.op, m.seq), (8, Some(3)));
        let d: SessionDescription = serde_json::from_str(
            r#"{"mode":"aead_aes256_gcm_rtpsize","secret_key":[1,2,3],"dave_protocol_version":1}"#,
        )
        .unwrap();
        assert_eq!(d.mode, EncryptionMode::Aes256Gcm);
        assert_eq!(d.dave_protocol_version, 1);
        let v: VideoIn = serde_json::from_str(
            r#"{"user_id":"7","audio_ssrc":1,"video_ssrc":2,"rtx_ssrc":3,"streams":[{"ssrc":2,"rtx_ssrc":3,"active":true,"rid":"100","quality":100}]}"#,
        )
        .unwrap();
        assert_eq!((v.user_id, v.video_ssrc, v.streams[0].active), (7, 2, true));
        let r: Ready = serde_json::from_str(
            r#"{"ssrc":1,"ip":"1.2.3.4","port":5,"modes":[],"streams":[{"type":"screen","ssrc":2,"rtx_ssrc":3,"rid":"100","quality":100,"active":false}]}"#,
        )
        .unwrap();
        assert_eq!((r.streams[0].ssrc, r.streams[0].rtx_ssrc), (2, 3));
        assert!(close::can_resume(close::VOICE_SERVER_CRASHED));
        assert!(!close::can_resume(close::DISCONNECTED));
    }
}
