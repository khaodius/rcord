//! H.264 over RTP (RFC 6184, packetization mode 1), as Discord's clients send it.
//!
//! Outgoing access units are split at their Annex B start codes; a NAL unit that fits in one
//! packet goes as is, a larger one as FU-A fragments. Incoming packets are put back together
//! into an Annex B access unit, which is what both DAVE and the decoder expect. Single NAL
//! units, STAP-A aggregates and FU-A fragments are understood.

/// The largest H.264 payload per packet, leaving room for the RTP header, the extension,
/// the transport tag and nonce inside a typical 1280-byte path MTU.
pub const MAX_PAYLOAD: usize = 1200;

const START_CODE: [u8; 4] = [0, 0, 0, 1];
const STAP_A: u8 = 24;
const FU_A: u8 = 28;

/// The NAL units of an Annex B byte stream, without their start codes.
pub fn split_nals(stream: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= stream.len() {
        if stream[i] == 0 && stream[i + 1] == 0 && stream[i + 2] == 1 {
            starts.push((i, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut nals = Vec::with_capacity(starts.len());
    for (n, &(_, body)) in starts.iter().enumerate() {
        let mut end = starts.get(n + 1).map_or(stream.len(), |&(code, _)| code);
        // A four-byte start code leaves a zero at the end of the previous unit.
        while end > body && stream[end - 1] == 0 && n + 1 < starts.len() {
            end -= 1;
        }
        if end > body {
            nals.push(&stream[body..end]);
        }
    }
    nals
}

/// True when the access unit holds an IDR slice, so a decoder can start from it.
pub fn is_keyframe(stream: &[u8]) -> bool {
    split_nals(stream)
        .iter()
        .any(|n| n.first().is_some_and(|h| h & 0x1f == 5))
}

/// Splits one access unit into RTP payloads, in order. The last payload ends the frame and
/// gets the marker bit.
pub fn packetize(access_unit: &[u8], max_payload: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for nal in split_nals(access_unit) {
        if nal.len() <= max_payload {
            out.push(nal.to_vec());
            continue;
        }
        let header = nal[0];
        let indicator = (header & 0xe0) | FU_A;
        let kind = header & 0x1f;
        let body = &nal[1..];
        let chunk = max_payload - 2;
        let count = body.len().div_ceil(chunk);
        for (i, part) in body.chunks(chunk).enumerate() {
            let mut fu = Vec::with_capacity(part.len() + 2);
            let mut fu_header = kind;
            if i == 0 {
                fu_header |= 0x80;
            }
            if i + 1 == count {
                fu_header |= 0x40;
            }
            fu.extend_from_slice(&[indicator, fu_header]);
            fu.extend_from_slice(part);
            out.push(fu);
        }
    }
    out
}

/// Reassembles access units from one sender's packets.
#[derive(Debug, Default)]
pub struct Depacketizer {
    frame: Vec<u8>,
    timestamp: Option<u32>,
    next_sequence: Option<u16>,
    /// A packet of the current frame was lost; it is dropped instead of decoded.
    damaged: bool,
    in_fragment: bool,
}

/// What a packet completed.
#[derive(Debug, PartialEq, Eq)]
pub enum Depacketized {
    /// Nothing yet.
    Pending,
    /// A whole access unit in Annex B form.
    Frame(Vec<u8>),
    /// A frame was lost; the decoder needs a keyframe to recover.
    Lost,
}

impl Depacketizer {
    pub fn push(
        &mut self,
        sequence: u16,
        timestamp: u32,
        marker: bool,
        payload: &[u8],
    ) -> Depacketized {
        let mut result = Depacketized::Pending;
        if let Some(expected) = self.next_sequence
            && sequence != expected
        {
            // Late or duplicate packets are ignored; a gap damages the frame.
            if sequence.wrapping_sub(expected) > 0x8000 {
                return Depacketized::Pending;
            }
            self.damaged = true;
        }
        self.next_sequence = Some(sequence.wrapping_add(1));

        if self.timestamp.is_some_and(|t| t != timestamp) {
            // A new frame started without the last one's marker: the end was lost.
            if !self.frame.is_empty() || self.damaged {
                result = Depacketized::Lost;
            }
            self.reset();
            // The gap belonged to the old frame unless this packet starts mid-unit.
        }
        self.timestamp = Some(timestamp);
        self.add(payload);

        if marker {
            let frame = std::mem::take(&mut self.frame);
            let damaged = self.damaged || self.in_fragment;
            self.reset();
            self.timestamp = None;
            if damaged {
                return Depacketized::Lost;
            }
            if !frame.is_empty() {
                return Depacketized::Frame(frame);
            }
        }
        result
    }

    fn reset(&mut self) {
        self.frame.clear();
        self.damaged = false;
        self.in_fragment = false;
    }

    fn add(&mut self, payload: &[u8]) {
        let Some(&first) = payload.first() else {
            return; // Padding-only packets probe bandwidth.
        };
        match first & 0x1f {
            1..=23 => {
                self.frame.extend_from_slice(&START_CODE);
                self.frame.extend_from_slice(payload);
            }
            STAP_A => {
                let mut rest = &payload[1..];
                while rest.len() >= 2 {
                    let len = usize::from(u16::from_be_bytes([rest[0], rest[1]]));
                    rest = &rest[2..];
                    if len == 0 || len > rest.len() {
                        self.damaged = true;
                        return;
                    }
                    self.frame.extend_from_slice(&START_CODE);
                    self.frame.extend_from_slice(&rest[..len]);
                    rest = &rest[len..];
                }
            }
            FU_A => {
                if payload.len() < 2 {
                    self.damaged = true;
                    return;
                }
                let fu = payload[1];
                let start = fu & 0x80 != 0;
                let end = fu & 0x40 != 0;
                if start {
                    if self.in_fragment {
                        self.damaged = true;
                    }
                    self.frame.extend_from_slice(&START_CODE);
                    self.frame.push((first & 0xe0) | (fu & 0x1f));
                    self.in_fragment = true;
                } else if !self.in_fragment {
                    self.damaged = true;
                    return;
                }
                self.frame.extend_from_slice(&payload[2..]);
                if end {
                    self.in_fragment = false;
                }
            }
            _ => self.damaged = true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn annex_b(nals: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for n in nals {
            out.extend_from_slice(&START_CODE);
            out.extend_from_slice(n);
        }
        out
    }

    #[test]
    fn splits_three_and_four_byte_start_codes() {
        let stream = [
            0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 4,
        ];
        assert_eq!(
            split_nals(&stream),
            vec![&[0x67, 1, 2][..], &[0x68, 3][..], &[0x65, 4][..]]
        );
        assert!(is_keyframe(&stream));
    }

    #[test]
    fn round_trip_with_fragmentation() {
        let sps = [0x67u8, 0x42, 0xe0, 0x1f];
        let mut idr = vec![0x65u8];
        idr.extend((0..5000u32).map(|i| (i % 251) as u8 | 1));
        let au = annex_b(&[&sps, &idr]);
        let payloads = packetize(&au, MAX_PAYLOAD);
        assert!(payloads.len() > 4);
        assert!(payloads.iter().all(|p| p.len() <= MAX_PAYLOAD));

        let mut d = Depacketizer::default();
        let last = payloads.len() - 1;
        let mut got = None;
        for (i, p) in payloads.iter().enumerate() {
            match d.push(100 + i as u16, 9000, i == last, p) {
                Depacketized::Frame(f) => got = Some(f),
                Depacketized::Pending => {}
                Depacketized::Lost => panic!("nothing was lost"),
            }
        }
        assert_eq!(got.unwrap(), au);
    }

    #[test]
    fn a_gap_loses_the_frame_and_the_next_recovers() {
        let mut big = vec![0x41u8];
        big.extend(std::iter::repeat_n(7u8, 3000));
        let payloads = packetize(&annex_b(&[&big]), MAX_PAYLOAD);
        let mut d = Depacketizer::default();
        assert_eq!(d.push(1, 10, false, &payloads[0]), Depacketized::Pending);
        // payloads[1] is lost.
        assert_eq!(d.push(3, 10, true, &payloads[2]), Depacketized::Lost);
        let small = annex_b(&[&[0x41, 1, 2]]);
        assert_eq!(
            d.push(4, 20, true, &packetize(&small, MAX_PAYLOAD)[0]),
            Depacketized::Frame(small)
        );
    }

    #[test]
    fn stap_a_is_unpacked() {
        let payload = [STAP_A, 0, 2, 0x67, 1, 0, 3, 0x68, 2, 3];
        let mut d = Depacketizer::default();
        assert_eq!(
            d.push(0, 0, true, &payload),
            Depacketized::Frame(annex_b(&[&[0x67, 1], &[0x68, 2, 3]]))
        );
    }
}
