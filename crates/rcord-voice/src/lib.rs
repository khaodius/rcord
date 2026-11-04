//! Voice for rcord: the voice gateway protocol (v8) with DAVE end-to-end encryption, UDP
//! media with `rtpsize` AEAD transport encryption, and the audio path from microphone to
//! Opus and from Opus to speakers. See `docs/ARCHITECTURE.md` for how the pieces fit.

pub mod audio;
pub mod crypto;
pub mod dave;
pub mod jitter;
pub mod protocol;
pub mod resample;
pub mod rtp;
pub mod session;

pub use audio::{AudioSettings, DeviceList, Transmit, devices};
pub use protocol::{EncryptionMode, VoiceOp};
pub use rtp::{IpDiscovery, RtpHeader};
pub use session::{ConnectionState, VoiceConfig, VoiceEvent, VoiceHandle, connect};
