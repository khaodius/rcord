/// The 74-byte UDP packet that asks the voice server for our public address and port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpDiscovery {
    pub ssrc: u32,
    pub address: String,
    pub port: u16,
}

impl IpDiscovery {
    pub const LEN: usize = 74;

    pub fn request(ssrc: u32) -> [u8; Self::LEN] {
        let mut p = [0u8; Self::LEN];
        p[0..2].copy_from_slice(&1u16.to_be_bytes());
        p[2..4].copy_from_slice(&70u16.to_be_bytes());
        p[4..8].copy_from_slice(&ssrc.to_be_bytes());
        p
    }

    pub fn parse_response(p: &[u8]) -> Option<Self> {
        if p.len() < Self::LEN || u16::from_be_bytes([p[0], p[1]]) != 2 {
            return None;
        }
        let ssrc = u32::from_be_bytes(p[4..8].try_into().ok()?);
        let raw = &p[8..72];
        let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
        let address = std::str::from_utf8(&raw[..end]).ok()?.to_owned();
        let port = u16::from_be_bytes([p[72], p[73]]);
        Some(Self {
            ssrc,
            address,
            port,
        })
    }
}

/// The playout-delay extension element (ID 5) with no minimum or maximum delay, sent at the
/// start of every encrypted video payload.
pub const PLAYOUT_DELAY: [u8; 4] = [0x51, 0x00, 0x00, 0x00];

/// An RTCP picture loss indication: asks the sender of `media_ssrc` for a new keyframe.
/// The first eight bytes are the additional authenticated data when sealed.
pub fn rtcp_pli(sender_ssrc: u32, media_ssrc: u32) -> ([u8; 8], [u8; 4]) {
    let mut head = [0u8; 8];
    head[0] = 0x81; // version 2, feedback message type 1 (PLI)
    head[1] = 206; // payload-specific feedback
    head[2..4].copy_from_slice(&2u16.to_be_bytes());
    head[4..8].copy_from_slice(&sender_ssrc.to_be_bytes());
    (head, media_ssrc.to_be_bytes())
}

/// A fixed 12-byte RTP header: Opus audio is payload type 120, H.264 video 101.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtpHeader {
    pub payload_type: u8,
    /// Set on the last packet of a video frame.
    pub marker: bool,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
}

impl RtpHeader {
    pub const LEN: usize = 12;
    pub const OPUS: u8 = 120;
    pub const H264: u8 = 101;
    pub const H264_RTX: u8 = 102;

    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut h = [0u8; Self::LEN];
        h[0] = 0x80;
        h[1] = (self.payload_type & 0x7f) | if self.marker { 0x80 } else { 0 };
        h[2..4].copy_from_slice(&self.sequence.to_be_bytes());
        h[4..8].copy_from_slice(&self.timestamp.to_be_bytes());
        h[8..12].copy_from_slice(&self.ssrc.to_be_bytes());
        h
    }

    /// Parses the header and returns it with the offset of the payload, skipping CSRCs.
    /// With the `rtpsize` modes the extension header stays in the clear, but its body is
    /// encrypted with the payload, so the offset stops after the four extension bytes.
    pub fn parse(p: &[u8]) -> Option<(Self, usize)> {
        if p.len() < Self::LEN || p[0] >> 6 != 2 {
            return None;
        }
        let csrc = usize::from(p[0] & 0x0f);
        let has_extension = p[0] & 0x10 != 0;
        let mut offset = Self::LEN + csrc * 4;
        if has_extension {
            offset += 4;
        }
        if p.len() < offset {
            return None;
        }
        Some((
            Self {
                payload_type: p[1] & 0x7f,
                marker: p[1] & 0x80 != 0,
                sequence: u16::from_be_bytes([p[2], p[3]]),
                timestamp: u32::from_be_bytes(p[4..8].try_into().ok()?),
                ssrc: u32::from_be_bytes(p[8..12].try_into().ok()?),
            },
            offset,
        ))
    }

    /// The header for a video packet: this header with its extension bit set, followed by
    /// a one-byte-form extension header announcing one element. Discord's clients send the
    /// playout-delay extension on every video packet; its element starts the payload, since
    /// the `rtpsize` modes encrypt extension bodies.
    pub fn encode_video(&self) -> [u8; Self::LEN + 4] {
        let mut h = [0u8; Self::LEN + 4];
        h[..Self::LEN].copy_from_slice(&self.encode());
        h[0] |= 0x10;
        h[12..16].copy_from_slice(&[0xbe, 0xde, 0x00, 0x01]);
        h
    }

    /// Advances to the next 20 ms Opus frame at 48 kHz.
    pub fn next_frame(&mut self) {
        self.sequence = self.sequence.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(960);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ip_discovery() {
        let req = IpDiscovery::request(0xdead_beef);
        assert_eq!(&req[..8], &[0, 1, 0, 70, 0xde, 0xad, 0xbe, 0xef]);
        let mut res = req;
        res[1] = 2;
        res[8..8 + 11].copy_from_slice(b"203.0.113.9");
        res[72..74].copy_from_slice(&50_000u16.to_be_bytes());
        assert_eq!(
            IpDiscovery::parse_response(&res),
            Some(IpDiscovery {
                ssrc: 0xdead_beef,
                address: "203.0.113.9".into(),
                port: 50_000
            })
        );
    }

    #[test]
    fn rtp_round_trip() {
        let mut h = RtpHeader {
            payload_type: RtpHeader::OPUS,
            marker: false,
            sequence: u16::MAX,
            timestamp: u32::MAX - 100,
            ssrc: 7,
        };
        let bytes = h.encode();
        assert_eq!(RtpHeader::parse(&bytes), Some((h, 12)));
        h.next_frame();
        assert_eq!(h.sequence, 0);
        assert_eq!(h.timestamp, 859);
    }

    #[test]
    fn video_header_carries_marker_and_extension() {
        let h = RtpHeader {
            payload_type: RtpHeader::H264,
            marker: true,
            sequence: 1,
            timestamp: 2,
            ssrc: 3,
        };
        let bytes = h.encode_video();
        assert_eq!(bytes[0], 0x90);
        assert_eq!(bytes[1], 0x80 | 101);
        assert_eq!(&bytes[12..], &[0xbe, 0xde, 0, 1]);
        assert_eq!(RtpHeader::parse(&bytes), Some((h, 16)));
    }

    #[test]
    fn pli_layout() {
        let (head, media) = rtcp_pli(1, 2);
        assert_eq!(head, [0x81, 206, 0, 2, 0, 0, 0, 1]);
        assert_eq!(media, [0, 0, 0, 2]);
    }
}
