//! Voice and Go Live transport: the voice gateway protocol, UDP IP discovery and RTP
//! framing. Media encryption, Opus, DAVE end-to-end encryption and capture are built on
//! these; see `docs/ARCHITECTURE.md` for the order they land in.

pub mod protocol;
pub mod rtp;

pub use protocol::{EncryptionMode, VoiceOp};
pub use rtp::{IpDiscovery, RtpHeader};
