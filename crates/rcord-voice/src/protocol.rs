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
    ClientDisconnect = 13,
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
            13 => ClientDisconnect,
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

#[derive(Debug, Clone, Serialize)]
pub struct Identify<'a> {
    pub server_id: &'a str,
    pub user_id: &'a str,
    pub session_id: &'a str,
    pub token: &'a str,
    pub video: bool,
    pub streams: Vec<StreamDescriptor>,
    /// Highest DAVE protocol version supported; 0 refuses end-to-end encryption.
    pub max_dave_protocol_version: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamDescriptor {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub rid: &'static str,
    pub quality: u8,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Ready {
    pub ssrc: u32,
    pub ip: String,
    pub port: u16,
    pub modes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionDescription {
    pub mode: EncryptionMode,
    pub secret_key: Vec<u8>,
    #[serde(default)]
    pub dave_protocol_version: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
