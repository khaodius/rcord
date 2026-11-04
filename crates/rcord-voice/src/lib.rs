//! Voice for rcord: the voice gateway protocol (v8) with DAVE end-to-end encryption, UDP
//! media with `rtpsize` AEAD transport encryption, and the audio path from microphone to
//! Opus and from Opus to speakers. See `docs/ARCHITECTURE.md` for how the pieces fit.

pub mod audio;
pub mod crypto;
pub mod dave;
pub mod h264;
pub mod jitter;
pub mod protocol;
pub mod resample;
pub mod rtp;
pub mod session;
pub mod video;

pub use audio::{AudioEvent, AudioSettings, Denoise, DeviceList, MicTest, Transmit, devices};
pub use protocol::{EncryptionMode, VoiceOp};
pub use rtp::{IpDiscovery, RtpHeader};
pub use session::{
    ConnectionState, StreamConfig, StreamHandle, StreamRole, VoiceConfig, VoiceEvent, VoiceHandle,
    connect, connect_stream,
};
pub use video::{DecodedFrame, VideoQuality};
