//! The video's sound: AAC packets decoded, resampled to the output device's rate and
//! channel count, and queued for the sound card. The count of frames the card has taken is
//! the playback clock the pictures follow.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use parking_lot::Mutex;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_AAC, CodecParameters, Decoder, DecoderOptions};
use symphonia::core::formats::Packet;

use crate::demux::{AudioTrack, File};

/// How much sound to keep queued ahead of the card.
const AHEAD_MS: u64 = 250;

struct Shared {
    queue: Mutex<VecDeque<f32>>,
    /// Frames the card has played since the clock last restarted.
    played: AtomicU64,
    volume: AtomicU32,
    playing: AtomicBool,
    /// Set once the last packet is queued, so silence after it still moves the clock.
    exhausted: AtomicBool,
}

pub struct Output {
    shared: Arc<Shared>,
    stream: cpal::Stream,
    decoder: Box<dyn Decoder>,
    rate: u32,
    channels: usize,
    source_rate: u32,
    resampler: Resampler,
    next: u32,
    base_ms: u64,
}

impl Output {
    pub fn start(track: &AudioTrack) -> Result<Self, String> {
        let mut params = CodecParameters::new();
        params
            .for_codec(CODEC_TYPE_AAC)
            .with_sample_rate(track.sample_rate)
            .with_time_base(symphonia::core::units::TimeBase::new(1, track.timescale))
            .with_extra_data(track.config.clone().into_boxed_slice());
        let decoder = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .map_err(|e| e.to_string())?;

        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no speakers or headphones found")?;
        let config = device.default_output_config().map_err(|e| e.to_string())?;
        let rate = config.sample_rate();
        let channels = usize::from(config.channels());
        let shared = Arc::new(Shared {
            queue: Mutex::new(VecDeque::new()),
            played: AtomicU64::new(0),
            volume: AtomicU32::new(1f32.to_bits()),
            playing: AtomicBool::new(false),
            exhausted: AtomicBool::new(false),
        });
        let stream = build(
            &device,
            config.config(),
            config.sample_format(),
            shared.clone(),
        )?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self {
            shared,
            stream,
            decoder,
            rate,
            channels,
            source_rate: track.sample_rate,
            resampler: Resampler::default(),
            next: 1,
            base_ms: 0,
        })
    }

    pub fn played_ms(&self) -> u64 {
        self.base_ms + self.shared.played.load(Ordering::Relaxed) * 1000 / u64::from(self.rate)
    }

    /// Makes the clock read `ms` from now on.
    pub fn restart_clock(&mut self, ms: u64) {
        self.base_ms = ms;
        self.shared.played.store(0, Ordering::Relaxed);
    }

    pub fn set_playing(&mut self, on: bool) {
        self.shared.playing.store(on, Ordering::Relaxed);
        let _ = if on {
            self.stream.play().map_err(|e| e.to_string())
        } else {
            self.stream.pause().map_err(|e| e.to_string())
        };
    }

    pub fn set_volume(&self, v: f32) {
        self.shared
            .volume
            .store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn seek(&mut self, file: &File, ms: u64) {
        self.shared.queue.lock().clear();
        self.shared.exhausted.store(false, Ordering::Relaxed);
        self.next = file.audio_sample_at(ms);
        self.decoder.reset();
        self.resampler = Resampler::default();
        let start = file
            .audio
            .as_ref()
            .and_then(|a| a.starts_ms.get(self.next as usize - 1).copied())
            .unwrap_or(ms);
        self.restart_clock(start.min(ms.max(start)));
    }

    /// Queues sound until a little is buffered ahead.
    pub fn fill(&mut self, file: &mut File) {
        let count = file.audio.as_ref().map_or(0, |a| a.count);
        let want = (AHEAD_MS * u64::from(self.rate) / 1000) as usize * self.channels;
        while self.shared.queue.lock().len() < want {
            if self.next > count {
                self.shared.exhausted.store(true, Ordering::Relaxed);
                return;
            }
            let Some(bytes) = file.audio_packet(self.next) else {
                self.next += 1;
                continue;
            };
            self.next += 1;
            let packet = Packet::new_from_slice(0, 0, 0, &bytes);
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(e) => {
                    tracing::debug!("skipping a broken sound packet: {e}");
                    continue;
                }
            };
            let spec = *decoded.spec();
            let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
            buf.copy_interleaved_ref(decoded);
            let src_channels = spec.channels.count().max(1);
            let out = self.resampler.run(
                buf.samples(),
                src_channels,
                self.channels,
                self.source_rate,
                self.rate,
            );
            self.shared.queue.lock().extend(out);
        }
    }

    pub fn drained(&self, file: &File) -> bool {
        let count = file.audio.as_ref().map_or(0, |a| a.count);
        self.next > count && self.shared.queue.lock().is_empty()
    }
}

fn build(
    device: &cpal::Device,
    config: StreamConfig,
    format: SampleFormat,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String> {
    fn make<T: SizedSample + FromSample<f32>>(
        device: &cpal::Device,
        config: StreamConfig,
        shared: Arc<Shared>,
    ) -> Result<cpal::Stream, cpal::Error> {
        let channels = usize::from(config.channels).max(1);
        device.build_output_stream::<T, _, _>(
            config,
            move |out: &mut [T], _| {
                if !shared.playing.load(Ordering::Relaxed) {
                    out.iter_mut().for_each(|s| *s = T::from_sample(0.0));
                    return;
                }
                let volume = f32::from_bits(shared.volume.load(Ordering::Relaxed));
                let mut q = shared.queue.lock();
                let real = q.len().min(out.len());
                for (i, s) in out.iter_mut().enumerate() {
                    let v = if i < real {
                        q.pop_front().unwrap_or(0.0)
                    } else {
                        0.0
                    };
                    *s = T::from_sample(v * volume);
                }
                drop(q);
                let counted = if shared.exhausted.load(Ordering::Relaxed) {
                    out.len()
                } else {
                    real
                };
                shared
                    .played
                    .fetch_add((counted / channels) as u64, Ordering::Relaxed);
            },
            |e| tracing::warn!("video sound stopped: {e}"),
            None,
        )
    }
    let result = match format {
        SampleFormat::F32 => make::<f32>(device, config, shared),
        SampleFormat::I16 => make::<i16>(device, config, shared),
        SampleFormat::I32 => make::<i32>(device, config, shared),
        SampleFormat::U16 => make::<u16>(device, config, shared),
        SampleFormat::U8 => make::<u8>(device, config, shared),
        SampleFormat::I8 => make::<i8>(device, config, shared),
        SampleFormat::F64 => make::<f64>(device, config, shared),
        other => return Err(format!("unsupported sample format {other}")),
    };
    result.map_err(|e| e.to_string())
}

/// Linear resampling and channel mapping, carrying its position between packets so
/// packet edges do not click.
#[derive(Default)]
struct Resampler {
    /// Position in source frames, relative to the start of the next packet. Negative means
    /// it still falls between the previous packet's last frame and this one's first.
    pos: f64,
    last: Vec<f32>,
}

impl Resampler {
    fn run(
        &mut self,
        input: &[f32],
        in_ch: usize,
        out_ch: usize,
        in_rate: u32,
        out_rate: u32,
    ) -> Vec<f32> {
        let frames = input.len() / in_ch;
        if frames == 0 {
            return Vec::new();
        }
        if self.last.len() != in_ch {
            self.last = input[..in_ch].to_vec();
        }
        let step = f64::from(in_rate) / f64::from(out_rate);
        let frame = |i: isize, c: usize| -> f32 {
            if i < 0 {
                self.last[c]
            } else {
                input[(i as usize).min(frames - 1) * in_ch + c]
            }
        };
        let mut out = Vec::with_capacity(((frames as f64 / step) as usize + 2) * out_ch);
        let mut pos = self.pos;
        while pos < (frames - 1) as f64 {
            let i = pos.floor();
            let frac = (pos - i) as f32;
            let i = i as isize;
            for c in 0..out_ch {
                let src = c.min(in_ch - 1);
                let a = frame(i, src);
                let b = frame(i + 1, src);
                out.push(a + (b - a) * frac);
            }
            pos += step;
        }
        self.pos = pos - frames as f64;
        self.last = input[(frames - 1) * in_ch..frames * in_ch].to_vec();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_and_maps_channels() {
        let mut r = Resampler::default();
        // Mono at 24 kHz to stereo at 48 kHz doubles the frames and the channels.
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let out = r.run(&input, 1, 2, 24_000, 48_000);
        assert!((out.len() as i64 - 400).abs() <= 4, "{}", out.len());
        assert_eq!(out[0], out[1]);
        assert!((out[2] - 0.5).abs() < 1e-4);
        // The next packet continues from where this one left off.
        let more = r.run(&input, 1, 2, 24_000, 48_000);
        assert!(!more.is_empty());
    }
}
