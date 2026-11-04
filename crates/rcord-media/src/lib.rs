//! Video playback: MP4 and MOV files with H.264 video and AAC audio, the format nearly every
//! phone, screen recorder and editor writes and most videos sent on Discord use. Video is
//! decoded with Cisco's OpenH264, audio with Symphonia, and sound goes out through cpal.
//!
//! [`Player::open`] starts a thread that owns the file, both decoders and the audio stream.
//! Frames come back as BGRA pixels on a small channel that always holds the newest frames,
//! so a slow window never builds a backlog; everything else comes on an event channel.

mod audio;
mod demux;

use std::collections::BTreeSet;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use openh264::formats::YUVSource;

pub use demux::Info;

/// A decoded picture in BGRA order, ready to upload.
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    /// When it shows, from the start of the video.
    pub at_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The file was read and playback can start.
    Ready(Info),
    /// Playback reached the end. It stays paused on the last frame.
    Ended,
    Failed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Unsupported(String),
    #[error("the video could not be read: {0}")]
    Demux(#[from] mp4::Error),
    #[error("the video could not be decoded: {0}")]
    Decode(String),
}

enum Command {
    Play,
    Pause,
    Seek(u64),
    Volume(f32),
}

/// Controls one video. Dropping every clone stops its thread.
#[derive(Clone)]
pub struct Player {
    commands: mpsc::Sender<Command>,
}

/// What [`Player::open`] hands back besides the player.
pub struct Channels {
    pub events: async_channel::Receiver<Event>,
    pub frames: async_channel::Receiver<VideoFrame>,
}

impl Player {
    /// Starts reading `data`, a whole MP4 file. Playback starts paused on the first frame.
    pub fn open(data: Vec<u8>) -> (Self, Channels) {
        let (commands, rx) = mpsc::channel();
        let (event_tx, events) = async_channel::unbounded();
        let (frame_tx, frames) = async_channel::bounded(2);
        let stale = frames.clone();
        std::thread::Builder::new()
            .name("rcord-video".into())
            .spawn(move || {
                if let Err(e) = run(data, &rx, &event_tx, &frame_tx, &stale) {
                    let _ = event_tx.try_send(Event::Failed(e.to_string()));
                }
            })
            .ok();
        (Self { commands }, Channels { events, frames })
    }

    pub fn play(&self) {
        let _ = self.commands.send(Command::Play);
    }

    pub fn pause(&self) {
        let _ = self.commands.send(Command::Pause);
    }

    pub fn seek(&self, at_ms: u64) {
        let _ = self.commands.send(Command::Seek(at_ms));
    }

    /// 0 is silent, 1 is the file's own level.
    pub fn set_volume(&self, volume: f32) {
        let _ = self.commands.send(Command::Volume(volume));
    }
}

/// Where playback is: from the sound card when there is sound, else from the wall clock.
struct Clock {
    base_ms: u64,
    started: Option<Instant>,
}

impl Clock {
    fn now(&self, audio: Option<&audio::Output>) -> u64 {
        match audio {
            Some(a) => self.base_ms + a.played_ms(),
            None => self.base_ms + self.started.map_or(0, |s| s.elapsed().as_millis() as u64),
        }
    }
}

struct Video {
    decoder: openh264::decoder::Decoder,
    /// Presentation times of samples sent to the decoder whose pictures have not come out.
    pending: BTreeSet<u64>,
    next: u32,
    /// The decoded frame waiting for its time.
    ready: Option<VideoFrame>,
    /// Frames before this are decoded but not shown, after a seek.
    skip_before: u64,
    /// The pictures the decoder held back for reordering were taken at the end.
    flushed: bool,
    /// Pictures taken at the end, in order.
    tail: std::collections::VecDeque<VideoFrame>,
}

/// Flushing after every packet, the wrapper's default, breaks streams with B-frames, so the
/// held-back pictures are taken once at the end instead.
fn new_decoder() -> Result<openh264::decoder::Decoder, Error> {
    use openh264::decoder::{Decoder, DecoderConfig, Flush};
    Decoder::with_api_config(
        openh264::OpenH264API::from_source(),
        DecoderConfig::new().flush_after_decode(Flush::NoFlush),
    )
    .map_err(|e| Error::Decode(e.to_string()))
}

fn run(
    data: Vec<u8>,
    commands: &mpsc::Receiver<Command>,
    events: &async_channel::Sender<Event>,
    frames: &async_channel::Sender<VideoFrame>,
    stale: &async_channel::Receiver<VideoFrame>,
) -> Result<(), Error> {
    let mut file = demux::File::open(data)?;
    let info = file.info.clone();
    let mut sound = match &file.audio {
        Some(track) => match audio::Output::start(track) {
            Ok(out) => Some(out),
            Err(e) => {
                tracing::warn!("video plays without sound: {e}");
                None
            }
        },
        None => None,
    };
    let mut video = Video {
        decoder: new_decoder()?,
        pending: BTreeSet::new(),
        next: 1,
        ready: None,
        skip_before: 0,
        flushed: false,
        tail: std::collections::VecDeque::new(),
    };
    let _ = events.try_send(Event::Ready(info.clone()));

    let mut clock = Clock {
        base_ms: 0,
        started: None,
    };
    let mut playing = false;
    let mut ended = false;
    let mut first_frame_sent = false;

    loop {
        // Commands: wait for one while paused, otherwise take what is queued.
        let mut queued = Vec::new();
        if !playing && first_frame_sent {
            match commands.recv() {
                Ok(c) => queued.push(c),
                Err(_) => return Ok(()),
            }
        }
        loop {
            match commands.try_recv() {
                Ok(c) => queued.push(c),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        for c in queued {
            match c {
                Command::Play => {
                    if ended {
                        seek(&mut file, &mut video, sound.as_mut(), &mut clock, 0);
                        ended = false;
                    }
                    playing = true;
                    clock.base_ms = clock.now(sound.as_ref());
                    clock.started = Some(Instant::now());
                    if let Some(s) = &mut sound {
                        s.restart_clock(clock.base_ms);
                        clock.base_ms = 0;
                        s.set_playing(true);
                    }
                }
                Command::Pause => {
                    clock.base_ms = clock.now(sound.as_ref());
                    clock.started = None;
                    if let Some(s) = &mut sound {
                        s.set_playing(false);
                        s.restart_clock(clock.base_ms);
                        clock.base_ms = 0;
                    }
                    playing = false;
                }
                Command::Seek(to) => {
                    let to = to.min(info.duration_ms);
                    // Frames from before the seek that the window has not taken yet.
                    while stale.try_recv().is_ok() {}
                    seek(&mut file, &mut video, sound.as_mut(), &mut clock, to);
                    ended = false;
                    first_frame_sent = false;
                    if playing {
                        clock.started = Some(Instant::now());
                    }
                }
                Command::Volume(v) => {
                    if let Some(s) = &sound {
                        s.set_volume(v);
                    }
                }
            }
        }

        if let Some(s) = &mut sound
            && playing
        {
            s.fill(&mut file);
        }

        // Decode until a frame is waiting.
        while video.ready.is_none() && video.next <= file.video_samples() {
            let Some(sample) = file.video_sample(video.next) else {
                video.next += 1;
                continue;
            };
            video.next += 1;
            video.pending.insert(sample.at_ms);
            match video.decoder.decode(&sample.annexb) {
                Ok(Some(yuv)) => {
                    let at_ms = video.pending.pop_first().unwrap_or(sample.at_ms);
                    if at_ms + 1 < video.skip_before {
                        continue;
                    }
                    video.ready = Some(to_frame(&yuv, at_ms));
                }
                Ok(None) => {}
                Err(e) => tracing::debug!("skipping a broken frame: {e}"),
            }
        }
        if video.ready.is_none() && video.next > file.video_samples() {
            if !video.flushed {
                video.flushed = true;
                if let Ok(rest) = video.decoder.flush_remaining() {
                    for yuv in rest {
                        let Some(at_ms) = video.pending.pop_first() else {
                            break;
                        };
                        if at_ms + 1 >= video.skip_before {
                            video.tail.push_back(to_frame(&yuv, at_ms));
                        }
                    }
                }
            }
            video.ready = video.tail.pop_front();
        }

        let now = clock.now(sound.as_ref());
        if let Some(frame) = video.ready.take() {
            if !first_frame_sent || (playing && frame.at_ms <= now) {
                let _ = frames.force_send(frame);
                first_frame_sent = true;
                continue;
            }
            let wait = frame.at_ms.saturating_sub(now).min(10);
            video.ready = Some(frame);
            std::thread::sleep(Duration::from_millis(wait.max(1)));
            continue;
        }

        if video.next > file.video_samples() && playing {
            let audio_done = sound.as_ref().is_none_or(|s| s.drained(&file));
            if audio_done || now >= info.duration_ms {
                playing = false;
                ended = true;
                clock.base_ms = info.duration_ms;
                clock.started = None;
                if let Some(s) = &mut sound {
                    s.set_playing(false);
                    s.restart_clock(info.duration_ms);
                    clock.base_ms = 0;
                }
                let _ = events.try_send(Event::Ended);
                continue;
            }
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}

/// Moves both tracks to `to_ms`: video from the keyframe before it, decoding but not
/// showing the frames in between, and sound from the first packet at or after it.
fn seek(
    file: &mut demux::File,
    video: &mut Video,
    sound: Option<&mut audio::Output>,
    clock: &mut Clock,
    to_ms: u64,
) {
    video.next = file.keyframe_before(to_ms);
    video.pending.clear();
    video.ready = None;
    video.tail.clear();
    video.skip_before = to_ms;
    if let Ok(d) = new_decoder() {
        video.decoder = d;
    }
    video.flushed = false;
    match sound {
        Some(s) => {
            s.seek(file, to_ms);
            clock.base_ms = 0;
        }
        None => clock.base_ms = to_ms,
    }
}

fn to_frame(yuv: &openh264::decoder::DecodedYUV<'_>, at_ms: u64) -> VideoFrame {
    let (w, h) = yuv.dimensions();
    let mut bgra = vec![0u8; w * h * 4];
    yuv.write_rgba8(&mut bgra);
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    VideoFrame {
        width: w as u32,
        height: h as u32,
        bgra,
        at_ms,
    }
}

/// Formats a position as `m:ss`, or `h:mm:ss` past an hour.
pub fn format_time(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(format_time(0), "0:00");
        assert_eq!(format_time(65_400), "1:05");
        assert_eq!(format_time(3_725_000), "1:02:05");
    }
}
