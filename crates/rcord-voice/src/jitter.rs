//! A jitter buffer per speaker. Packets arrive late, early or out of order; playback pulls
//! one 20 ms frame per tick in sequence order, after holding a few frames back so a little
//! network jitter never reaches the speakers as a gap.

use std::collections::HashMap;

/// Frames held before playback starts.
const TARGET_DEPTH: usize = 3;
/// More than this many frames queued means playback fell behind; the oldest are skipped.
const MAX_DEPTH: usize = 25;
/// Consecutive empty ticks after which the speaker is considered to have stopped.
const IDLE_AFTER: u32 = 10;

#[derive(Debug, PartialEq, Eq)]
pub enum Playout {
    /// The next frame, in order.
    Frame(Vec<u8>),
    /// The next frame is missing. Carries the frame after it, if already here, whose forward
    /// error correction data can rebuild the missing one.
    Lost(Option<Vec<u8>>),
    /// Nothing to play: not started yet, or the speaker went quiet.
    Idle,
}

#[derive(Debug, Default)]
pub struct JitterBuffer {
    packets: HashMap<u16, Vec<u8>>,
    next: Option<u16>,
    playing: bool,
    empty_ticks: u32,
}

fn ahead(seq: u16, of: u16) -> i16 {
    seq.wrapping_sub(of) as i16
}

impl JitterBuffer {
    pub fn push(&mut self, seq: u16, frame: Vec<u8>) {
        match self.next {
            None => self.next = Some(seq),
            Some(next) => {
                let d = ahead(seq, next);
                if d < 0 {
                    if self.playing {
                        return; // Too late to play.
                    }
                    self.next = Some(seq);
                } else if d as usize > MAX_DEPTH * 4 {
                    // A jump this large is a new stream from the same sender.
                    self.reset();
                    self.next = Some(seq);
                }
            }
        }
        self.packets.insert(seq, frame);
        while self.packets.len() > MAX_DEPTH {
            if let Some(next) = self.next {
                self.packets.remove(&next);
                self.next = Some(next.wrapping_add(1));
            }
        }
    }

    pub fn pop(&mut self) -> Playout {
        let Some(next) = self.next else {
            return Playout::Idle;
        };
        if !self.playing {
            if self.packets.len() < TARGET_DEPTH {
                return Playout::Idle;
            }
            self.playing = true;
        }
        self.next = Some(next.wrapping_add(1));
        if let Some(frame) = self.packets.remove(&next) {
            self.empty_ticks = 0;
            return Playout::Frame(frame);
        }
        if self.packets.is_empty() {
            self.empty_ticks += 1;
            if self.empty_ticks >= IDLE_AFTER {
                self.reset();
                return Playout::Idle;
            }
        }
        Playout::Lost(self.packets.get(&next.wrapping_add(1)).cloned())
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    fn reset(&mut self) {
        self.packets.clear();
        self.next = None;
        self.playing = false;
        self.empty_ticks = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(n: u8) -> Vec<u8> {
        vec![n]
    }

    #[test]
    fn reorders_and_waits_for_depth() {
        let mut j = JitterBuffer::default();
        j.push(11, f(11));
        assert_eq!(j.pop(), Playout::Idle);
        j.push(10, f(10));
        j.push(12, f(12));
        assert_eq!(j.pop(), Playout::Frame(f(10)));
        assert_eq!(j.pop(), Playout::Frame(f(11)));
        assert_eq!(j.pop(), Playout::Frame(f(12)));
    }

    #[test]
    fn reports_loss_with_the_following_frame() {
        let mut j = JitterBuffer::default();
        for s in [1, 2, 4, 5] {
            j.push(s, f(s as u8));
        }
        assert_eq!(j.pop(), Playout::Frame(f(1)));
        assert_eq!(j.pop(), Playout::Frame(f(2)));
        assert_eq!(j.pop(), Playout::Lost(Some(f(4))));
        assert_eq!(j.pop(), Playout::Frame(f(4)));
        // A packet for a slot already played is dropped.
        j.push(3, f(3));
        assert_eq!(j.pop(), Playout::Frame(f(5)));
    }

    #[test]
    fn wraps_sequence_numbers() {
        let mut j = JitterBuffer::default();
        for s in [65_534u16, 65_535, 0, 1] {
            j.push(s, f(s as u8));
        }
        let order: Vec<_> = (0..4).map(|_| j.pop()).collect();
        assert_eq!(
            order,
            vec![
                Playout::Frame(f(254)),
                Playout::Frame(f(255)),
                Playout::Frame(f(0)),
                Playout::Frame(f(1))
            ]
        );
    }

    #[test]
    fn goes_idle_after_silence_and_restarts() {
        let mut j = JitterBuffer::default();
        for s in 0..3 {
            j.push(s, f(0));
        }
        for _ in 0..3 {
            j.pop();
        }
        for _ in 0..IDLE_AFTER - 1 {
            assert!(matches!(j.pop(), Playout::Lost(None)));
        }
        assert_eq!(j.pop(), Playout::Idle);
        assert!(!j.is_playing());
        for s in 500..503 {
            j.push(s, f(1));
        }
        assert_eq!(j.pop(), Playout::Frame(f(1)));
    }

    #[test]
    fn bounds_queue_depth() {
        let mut j = JitterBuffer::default();
        for s in 0..(MAX_DEPTH as u16 + 10) {
            j.push(s, f(s as u8));
        }
        assert_eq!(j.pop(), Playout::Frame(f(10)));
    }
}
