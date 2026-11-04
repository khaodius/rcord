//! DAVE, Discord's end-to-end encryption for calls, on top of the MLS implementation in
//! `davey`. The voice gateway drives it: the server hands out the external sender, proposals,
//! commits and welcomes, and announces when every member should switch keys. This type turns
//! each of those into calls on the MLS session and the replies the gateway expects.
//!
//! Media is encrypted per frame with the sender's key before transport encryption, so the
//! voice server forwards packets it cannot read.

use std::borrow::Cow;
use std::collections::HashSet;
use std::num::NonZeroU16;

use davey::{Codec, DaveSession, MediaType, ProposalsOperationType};

use crate::protocol::{TransitionId, VoiceOp, encode_binary};

/// Highest DAVE protocol version this client speaks.
pub const MAX_PROTOCOL_VERSION: u16 = davey::DAVE_PROTOCOL_VERSION;

/// A frame for the voice gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Json(String),
    Binary(Vec<u8>),
}

impl Reply {
    fn json(op: VoiceOp, transition_id: u16) -> Self {
        let body = crate::protocol::Outgoing::new(op, TransitionId { transition_id });
        Self::Json(serde_json::to_string(&body).unwrap_or_default())
    }
}

pub struct Dave {
    user_id: u64,
    channel_id: u64,
    protocol_version: u16,
    session: Option<DaveSession>,
    /// Everyone the voice server says is in the call, ourselves included.
    members: HashSet<u64>,
    /// Set while recovering from a bad commit or welcome, so repeated failures during the
    /// same recovery are not reported again.
    reinitializing: bool,
    /// The prepared transition and the protocol version it moves to.
    pending: Option<(u16, u16)>,
}

impl std::fmt::Debug for Dave {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dave")
            .field("protocol_version", &self.protocol_version)
            .field("members", &self.members.len())
            .finish_non_exhaustive()
    }
}

impl Dave {
    pub fn new(user_id: u64, channel_id: u64) -> Self {
        Self {
            user_id,
            channel_id,
            protocol_version: 0,
            session: None,
            members: HashSet::from([user_id]),
            reinitializing: false,
            pending: None,
        }
    }

    pub fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    /// Whether the call is end-to-end encrypted right now.
    pub fn is_active(&self) -> bool {
        self.protocol_version > 0 && self.session.as_ref().is_some_and(|s| s.is_ready())
    }

    /// The code everyone in the call sees, so they can compare it out of band.
    pub fn privacy_code(&self) -> Option<String> {
        self.session
            .as_ref()
            .and_then(|s| s.voice_privacy_code())
            .map(str::to_owned)
    }

    pub fn add_members(&mut self, ids: &[u64]) {
        self.members.extend(ids);
    }

    pub fn remove_member(&mut self, id: u64) {
        if id != self.user_id {
            self.members.remove(&id);
        }
    }

    /// The protocol version from the session description. Starts the MLS session and sends
    /// a key package when the call is encrypted.
    pub fn start(&mut self, protocol_version: u16) -> Vec<Reply> {
        self.protocol_version = protocol_version;
        self.reinit()
    }

    fn reinit(&mut self) -> Vec<Reply> {
        let Some(version) = NonZeroU16::new(self.protocol_version) else {
            if let Some(s) = &mut self.session {
                if let Err(e) = s.reset() {
                    tracing::warn!("DAVE reset failed: {e}");
                }
                s.set_passthrough_mode(true, Some(10));
            }
            return Vec::new();
        };
        let result = match &mut self.session {
            Some(s) => s
                .reinit(version, self.user_id, self.channel_id, None)
                .map_err(|e| e.to_string()),
            None => DaveSession::new(version, self.user_id, self.channel_id, None)
                .map(|s| self.session = Some(s))
                .map_err(|e| e.to_string()),
        };
        if let Err(e) = result {
            tracing::error!("DAVE session could not start: {e}");
            return Vec::new();
        }
        self.key_package()
    }

    fn key_package(&mut self) -> Vec<Reply> {
        let Some(s) = &mut self.session else {
            return Vec::new();
        };
        match s.create_key_package() {
            Ok(kp) => vec![Reply::Binary(encode_binary(
                VoiceOp::DaveMlsKeyPackage,
                &kp,
            ))],
            Err(e) => {
                tracing::error!("DAVE key package failed: {e}");
                Vec::new()
            }
        }
    }

    pub fn external_sender(&mut self, payload: &[u8]) -> Vec<Reply> {
        if let Some(s) = &mut self.session
            && let Err(e) = s.set_external_sender(payload)
        {
            tracing::warn!("DAVE external sender rejected: {e}");
        }
        Vec::new()
    }

    pub fn proposals(&mut self, payload: &[u8]) -> Vec<Reply> {
        let Some(s) = &mut self.session else {
            return Vec::new();
        };
        let Some((&kind, proposals)) = payload.split_first() else {
            return Vec::new();
        };
        let kind = if kind == 0 {
            ProposalsOperationType::APPEND
        } else {
            ProposalsOperationType::REVOKE
        };
        let expected: Vec<u64> = self.members.iter().copied().collect();
        match s.process_proposals(kind, proposals, Some(&expected)) {
            Ok(Some(cw)) => {
                let mut body = cw.commit;
                if let Some(welcome) = cw.welcome {
                    body.extend_from_slice(&welcome);
                }
                vec![Reply::Binary(encode_binary(
                    VoiceOp::DaveMlsCommitWelcome,
                    &body,
                ))]
            }
            Ok(None) => Vec::new(),
            Err(e) => {
                tracing::warn!("DAVE proposals rejected: {e}");
                Vec::new()
            }
        }
    }

    /// An announced commit: a two-byte transition ID, then the MLS commit.
    pub fn commit(&mut self, payload: &[u8]) -> Vec<Reply> {
        let Some((tid, commit)) = split_transition(payload) else {
            return Vec::new();
        };
        let Some(s) = &mut self.session else {
            return Vec::new();
        };
        match s.process_commit(commit) {
            Ok(true) => self.prepared(tid, self.protocol_version),
            Ok(false) => Vec::new(),
            Err(e) => {
                tracing::warn!("DAVE commit for transition {tid} failed: {e}");
                self.recover(tid)
            }
        }
    }

    /// A welcome into the group: a two-byte transition ID, then the MLS welcome.
    pub fn welcome(&mut self, payload: &[u8]) -> Vec<Reply> {
        let Some((tid, welcome)) = split_transition(payload) else {
            return Vec::new();
        };
        let Some(s) = &mut self.session else {
            return Vec::new();
        };
        let expected: Vec<u64> = self.members.iter().copied().collect();
        match s.process_welcome(welcome, Some(&expected)) {
            Ok(()) => self.prepared(tid, self.protocol_version),
            Err(e) => {
                tracing::warn!("DAVE welcome for transition {tid} failed: {e}");
                self.recover(tid)
            }
        }
    }

    pub fn prepare_transition(&mut self, tid: u16, protocol_version: u16) -> Vec<Reply> {
        self.prepared(tid, protocol_version)
    }

    fn prepared(&mut self, tid: u16, protocol_version: u16) -> Vec<Reply> {
        if let Some(s) = &mut self.session
            && let Err(e) = s.prepare_transition(tid, protocol_version)
        {
            tracing::warn!("DAVE transition {tid} could not be prepared: {e}");
            return self.recover(tid);
        }
        if tid == 0 {
            // Transition 0 takes effect at once: it is the initial group or a solo call.
            self.protocol_version = protocol_version;
            self.reinitializing = false;
            Vec::new()
        } else {
            self.pending = Some((tid, protocol_version));
            vec![Reply::json(VoiceOp::DaveTransitionReady, tid)]
        }
    }

    pub fn execute_transition(&mut self, tid: u16) {
        if let Some(s) = &mut self.session
            && let Err(e) = s.execute_transition(tid)
        {
            tracing::warn!("DAVE transition {tid} could not execute: {e}");
        }
        if let Some((pending, version)) = self.pending
            && pending == tid
        {
            self.pending = None;
            self.protocol_version = version;
            self.reinitializing = false;
        }
    }

    /// Epoch 1 means a new group is starting, possibly with a new protocol version.
    pub fn prepare_epoch(&mut self, epoch: u64, protocol_version: u16) -> Vec<Reply> {
        if epoch == 1 {
            self.protocol_version = protocol_version;
            self.reinit()
        } else {
            Vec::new()
        }
    }

    fn recover(&mut self, tid: u16) -> Vec<Reply> {
        if self.reinitializing {
            return Vec::new();
        }
        self.reinitializing = true;
        let mut out = vec![Reply::json(VoiceOp::DaveMlsInvalidCommitWelcome, tid)];
        out.extend(self.reinit());
        out
    }

    /// Encrypts an outgoing Opus frame. Returns it untouched when the call is not encrypted,
    /// and `None` while keys are not ready yet, since other members could not play it.
    pub fn encrypt_opus<'a>(&mut self, frame: &'a [u8]) -> Option<Cow<'a, [u8]>> {
        self.encrypt(MediaType::AUDIO, Codec::OPUS, frame)
    }

    /// Encrypts an H.264 access unit in Annex B form. The NAL unit headers stay readable so
    /// the frame can still be packetized; the rest is end-to-end encrypted.
    pub fn encrypt_h264<'a>(&mut self, frame: &'a [u8]) -> Option<Cow<'a, [u8]>> {
        self.encrypt(MediaType::VIDEO, Codec::H264, frame)
    }

    fn encrypt<'a>(
        &mut self,
        media: MediaType,
        codec: Codec,
        frame: &'a [u8],
    ) -> Option<Cow<'a, [u8]>> {
        if self.protocol_version == 0 {
            return Some(Cow::Borrowed(frame));
        }
        let s = self.session.as_mut()?;
        s.encrypt(media, codec, frame).ok()
    }

    /// Decrypts an incoming Opus frame from `user_id`. Unencrypted frames pass while the call
    /// is not encrypted or is moving into or out of encryption.
    pub fn decrypt_opus<'a>(&mut self, user_id: u64, frame: &'a [u8]) -> Option<Cow<'a, [u8]>> {
        self.decrypt(user_id, MediaType::AUDIO, frame)
    }

    /// Decrypts a reassembled video frame from `user_id`.
    pub fn decrypt_video<'a>(&mut self, user_id: u64, frame: &'a [u8]) -> Option<Cow<'a, [u8]>> {
        self.decrypt(user_id, MediaType::VIDEO, frame)
    }

    fn decrypt<'a>(
        &mut self,
        user_id: u64,
        media: MediaType,
        frame: &'a [u8],
    ) -> Option<Cow<'a, [u8]>> {
        let Some(s) = self.session.as_mut() else {
            return Some(Cow::Borrowed(frame));
        };
        if self.protocol_version == 0 && !s.is_ready() {
            return Some(Cow::Borrowed(frame));
        }
        match s.decrypt(user_id, media, frame) {
            Ok(plain) => Some(Cow::Owned(plain)),
            Err(e) => {
                if self.protocol_version == 0 || s.can_passthrough(user_id) {
                    Some(Cow::Borrowed(frame))
                } else {
                    tracing::trace!("DAVE decrypt from {user_id} failed: {e}");
                    None
                }
            }
        }
    }
}

fn split_transition(payload: &[u8]) -> Option<(u16, &[u8])> {
    let (tid, rest) = payload.split_at_checked(2)?;
    Some((u16::from_be_bytes([tid[0], tid[1]]), rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unencrypted_calls_pass_media_through() {
        let mut d = Dave::new(1, 2);
        assert!(d.start(0).is_empty());
        assert!(!d.is_active());
        assert_eq!(d.encrypt_opus(b"abc").as_deref(), Some(&b"abc"[..]));
        assert_eq!(d.decrypt_opus(5, b"abc").as_deref(), Some(&b"abc"[..]));
    }

    #[test]
    fn encrypted_call_sends_a_key_package_and_holds_media() {
        let mut d = Dave::new(1, 2);
        let replies = d.start(MAX_PROTOCOL_VERSION);
        assert_eq!(replies.len(), 1);
        let Reply::Binary(frame) = &replies[0] else {
            panic!("key package must be binary");
        };
        assert_eq!(frame[0], VoiceOp::DaveMlsKeyPackage as u8);
        assert!(frame.len() > 1);
        // No group yet, so nothing can be sent that others could decrypt.
        assert!(d.encrypt_opus(b"abc").is_none());
        assert!(!d.is_active());
    }

    #[test]
    fn transition_ready_is_acknowledged() {
        let mut d = Dave::new(1, 2);
        d.start(MAX_PROTOCOL_VERSION);
        let replies = d.prepare_transition(7, 0);
        assert_eq!(
            replies,
            vec![Reply::Json(r#"{"op":23,"d":{"transition_id":7}}"#.into())]
        );
        d.execute_transition(7);
        assert_eq!(d.protocol_version(), 0);
        assert!(d.prepare_transition(0, 1).is_empty());
        assert_eq!(d.protocol_version(), 1);
    }

    #[test]
    fn bad_commit_reports_once_and_rejoins() {
        let mut d = Dave::new(1, 2);
        d.start(MAX_PROTOCOL_VERSION);
        // A commit without a group is ignored; a welcome that cannot be read is reported.
        assert!(d.commit(&[0, 9, 1, 2, 3]).is_empty());
        let replies = d.welcome(&[0, 9, 1, 2, 3]);
        assert_eq!(
            replies[0],
            Reply::Json(r#"{"op":31,"d":{"transition_id":9}}"#.into())
        );
        assert!(matches!(replies.get(1), Some(Reply::Binary(_))));
        assert!(d.welcome(&[0, 9, 1, 2, 3]).is_empty());
    }
}
