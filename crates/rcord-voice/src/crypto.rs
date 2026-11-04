//! Transport encryption for RTP over UDP, in the two `rtpsize` AEAD modes Discord offers.
//!
//! A packet is the RTP header in the clear (it is the additional authenticated data), the
//! encrypted payload with its 16-byte tag, then the 32-bit nonce counter. The counter is
//! big-endian and fills the start of the cipher's nonce; the rest is zero. With an RTP header
//! extension, its four-byte header stays in the clear and its body is encrypted with the
//! payload.

use aes_gcm::Aes256Gcm;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::XChaCha20Poly1305;

use crate::protocol::EncryptionMode;
use crate::rtp::RtpHeader;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("the voice server sent a {0}-byte key; 32 bytes were expected")]
    KeyLength(usize),
    #[error("packet could not be encrypted")]
    Seal,
}

enum Cipher {
    Aes(Box<Aes256Gcm>),
    XChaCha(Box<XChaCha20Poly1305>),
}

/// Seals outgoing packets and opens incoming ones with the session's secret key.
pub struct TransportCipher {
    mode: EncryptionMode,
    cipher: Cipher,
    counter: u32,
}

impl std::fmt::Debug for TransportCipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransportCipher")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

/// Bytes the transport adds to a payload: the tag and the nonce counter.
pub const OVERHEAD: usize = 16 + 4;

impl TransportCipher {
    pub fn new(mode: EncryptionMode, key: &[u8]) -> Result<Self, CryptoError> {
        if key.len() != 32 {
            return Err(CryptoError::KeyLength(key.len()));
        }
        let cipher = match mode {
            EncryptionMode::Aes256Gcm => Cipher::Aes(Box::new(
                Aes256Gcm::new_from_slice(key).map_err(|_| CryptoError::KeyLength(key.len()))?,
            )),
            EncryptionMode::XChaCha20Poly1305 => Cipher::XChaCha(Box::new(
                XChaCha20Poly1305::new_from_slice(key)
                    .map_err(|_| CryptoError::KeyLength(key.len()))?,
            )),
        };
        Ok(Self {
            mode,
            cipher,
            counter: rand::random(),
        })
    }

    pub fn mode(&self) -> EncryptionMode {
        self.mode
    }

    /// Encrypts `payload` behind `header` and appends the result to `out`, which is cleared
    /// first.
    pub fn seal(
        &mut self,
        header: &[u8],
        payload: &[u8],
        out: &mut Vec<u8>,
    ) -> Result<(), CryptoError> {
        let counter = self.counter.to_be_bytes();
        self.counter = self.counter.wrapping_add(1);
        let msg = Payload {
            msg: payload,
            aad: header,
        };
        let sealed = match &self.cipher {
            Cipher::Aes(c) => c.encrypt(&nonce12(counter).into(), msg),
            Cipher::XChaCha(c) => c.encrypt(&nonce24(counter).into(), msg),
        }
        .map_err(|_| CryptoError::Seal)?;
        out.clear();
        out.reserve(header.len() + sealed.len() + 4);
        out.extend_from_slice(header);
        out.extend_from_slice(&sealed);
        out.extend_from_slice(&counter);
        Ok(())
    }

    /// Decrypts a received packet. Returns its header and the media payload with any header
    /// extension body and padding removed, or `None` if the packet is malformed or forged.
    pub fn open(&self, packet: &[u8]) -> Option<(RtpHeader, Vec<u8>)> {
        let (header, aad_len) = RtpHeader::parse(packet)?;
        if packet.len() < aad_len + OVERHEAD {
            return None;
        }
        let (body, counter) = packet.split_at(packet.len() - 4);
        let counter: [u8; 4] = counter.try_into().ok()?;
        let msg = Payload {
            msg: &body[aad_len..],
            aad: &body[..aad_len],
        };
        let mut plain = match &self.cipher {
            Cipher::Aes(c) => c.decrypt(&nonce12(counter).into(), msg),
            Cipher::XChaCha(c) => c.decrypt(&nonce24(counter).into(), msg),
        }
        .ok()?;
        // Padding: the last byte counts the padding bytes, itself included.
        if packet[0] & 0x20 != 0 {
            let pad = usize::from(*plain.last()?);
            plain.truncate(plain.len().checked_sub(pad)?);
        }
        let mut start = 0;
        if packet[0] & 0x10 != 0 {
            let words = u16::from_be_bytes([packet[aad_len - 2], packet[aad_len - 1]]);
            start = usize::from(words) * 4;
            if start > plain.len() {
                return None;
            }
        }
        plain.drain(..start);
        Some((header, plain))
    }
}

fn nonce12(counter: [u8; 4]) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..4].copy_from_slice(&counter);
    n
}

fn nonce24(counter: [u8; 4]) -> [u8; 24] {
    let mut n = [0u8; 24];
    n[..4].copy_from_slice(&counter);
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> RtpHeader {
        RtpHeader {
            payload_type: RtpHeader::OPUS,
            sequence: 1,
            timestamp: 960,
            ssrc: 99,
        }
    }

    #[test]
    fn round_trip_both_modes() {
        for mode in EncryptionMode::PREFERENCE {
            let mut c = TransportCipher::new(mode, &[7u8; 32]).unwrap();
            let h = header().encode();
            let mut out = Vec::new();
            c.seal(&h, b"opus frame", &mut out).unwrap();
            assert_eq!(out.len(), 12 + 10 + OVERHEAD);
            assert_eq!(&out[..12], &h);
            let (parsed, payload) = c.open(&out).unwrap();
            assert_eq!(parsed, header());
            assert_eq!(payload, b"opus frame");

            // Any flipped bit in the header or body fails authentication.
            for i in [3, 14, out.len() - 5] {
                let mut bad = out.clone();
                bad[i] ^= 1;
                assert!(c.open(&bad).is_none());
            }
        }
    }

    #[test]
    fn counter_advances_and_is_appended() {
        let mut c = TransportCipher::new(EncryptionMode::Aes256Gcm, &[1u8; 32]).unwrap();
        c.counter = u32::MAX;
        let mut a = Vec::new();
        c.seal(&header().encode(), b"x", &mut a).unwrap();
        assert_eq!(&a[a.len() - 4..], &u32::MAX.to_be_bytes());
        let mut b = Vec::new();
        c.seal(&header().encode(), b"x", &mut b).unwrap();
        assert_eq!(&b[b.len() - 4..], &0u32.to_be_bytes());
    }

    #[test]
    fn strips_extension_body() {
        let mut c = TransportCipher::new(EncryptionMode::XChaCha20Poly1305, &[3u8; 32]).unwrap();
        // Header with the extension bit and a one-word extension (0xBEDE profile).
        let mut h = header().encode().to_vec();
        h[0] |= 0x10;
        h.extend_from_slice(&[0xbe, 0xde, 0x00, 0x01]);
        let mut body = vec![0x10, 0xff, 0x00, 0x00];
        body.extend_from_slice(b"audio");
        let mut out = Vec::new();
        c.seal(&h, &body, &mut out).unwrap();
        let (_, payload) = c.open(&out).unwrap();
        assert_eq!(payload, b"audio");
    }

    #[test]
    fn rejects_bad_keys_and_short_packets() {
        assert!(TransportCipher::new(EncryptionMode::Aes256Gcm, &[0u8; 16]).is_err());
        let c = TransportCipher::new(EncryptionMode::Aes256Gcm, &[0u8; 32]).unwrap();
        assert!(c.open(&header().encode()).is_none());
    }
}
