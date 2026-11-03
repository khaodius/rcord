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

/// A fixed 12-byte RTP header, as used for Opus audio (payload type 120).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtpHeader {
    pub payload_type: u8,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
}

impl RtpHeader {
    pub const LEN: usize = 12;
    pub const OPUS: u8 = 120;

    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut h = [0u8; Self::LEN];
        h[0] = 0x80;
        h[1] = self.payload_type & 0x7f;
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
                sequence: u16::from_be_bytes([p[2], p[3]]),
                timestamp: u32::from_be_bytes(p[4..8].try_into().ok()?),
                ssrc: u32::from_be_bytes(p[8..12].try_into().ok()?),
            },
            offset,
        ))
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
}
