//! Reads samples out of an MP4 or MOV file.

use std::io::Cursor;

use mp4::{MediaType, Mp4Reader, Mp4Track};

use crate::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
}

pub struct VideoSample {
    /// The sample's NAL units with start codes, the form the decoder reads, with the
    /// stream's parameter sets in front of each keyframe.
    pub annexb: Vec<u8>,
    pub at_ms: u64,
}

pub struct AudioTrack {
    pub id: u32,
    pub timescale: u32,
    pub sample_rate: u32,
    /// The AAC AudioSpecificConfig.
    pub config: Vec<u8>,
    pub count: u32,
    /// Start of each sample in milliseconds, by sample number - 1.
    pub starts_ms: Vec<u64>,
}

pub struct File {
    reader: Mp4Reader<Cursor<Vec<u8>>>,
    pub info: Info,
    video_id: u32,
    video_timescale: u32,
    video_count: u32,
    length_size: usize,
    parameter_sets: Vec<u8>,
    video_starts_ms: Vec<u64>,
    /// Keyframe sample numbers, ascending. Empty means every sample is one.
    keyframes: Vec<u32>,
    pub audio: Option<AudioTrack>,
}

const AAC_RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

fn starts_ms(track: &Mp4Track) -> Vec<u64> {
    let scale = u64::from(track.timescale().max(1));
    let mut out = Vec::with_capacity(track.sample_count() as usize);
    let mut t: u64 = 0;
    for e in &track.trak.mdia.minf.stbl.stts.entries {
        for _ in 0..e.sample_count {
            out.push(t * 1000 / scale);
            t += u64::from(e.sample_delta);
        }
    }
    out
}

impl File {
    pub fn open(data: Vec<u8>) -> Result<Self, Error> {
        let size = data.len() as u64;
        let reader = Mp4Reader::read_header(Cursor::new(data), size)?;
        let tracks = reader.tracks();
        let video = tracks
            .values()
            .find(|t| matches!(t.media_type(), Ok(MediaType::H264)))
            .ok_or_else(|| {
                let other = tracks
                    .values()
                    .find_map(|t| match t.media_type() {
                        Ok(MediaType::H265) => Some("HEVC (H.265)"),
                        Ok(MediaType::VP9) => Some("VP9"),
                        _ => None,
                    })
                    .unwrap_or("this format");
                Error::Unsupported(format!("rcord cannot play {other} video yet"))
            })?;
        let avc = video
            .trak
            .mdia
            .minf
            .stbl
            .stsd
            .avc1
            .as_ref()
            .ok_or_else(|| Error::Unsupported("the video has no H.264 settings".into()))?;
        let mut parameter_sets = Vec::new();
        for nal in avc
            .avcc
            .sequence_parameter_sets
            .iter()
            .chain(&avc.avcc.picture_parameter_sets)
        {
            parameter_sets.extend_from_slice(&[0, 0, 0, 1]);
            parameter_sets.extend_from_slice(&nal.bytes);
        }
        let keyframes = video
            .trak
            .mdia
            .minf
            .stbl
            .stss
            .as_ref()
            .map(|s| s.entries.clone())
            .unwrap_or_default();
        let video_starts_ms = starts_ms(video);
        let audio = tracks
            .values()
            .find(|t| matches!(t.media_type(), Ok(MediaType::AAC)))
            .and_then(|t| {
                let esds = t.trak.mdia.minf.stbl.stsd.mp4a.as_ref()?.esds.as_ref()?;
                let d = &esds.es_desc.dec_config.dec_specific;
                let sample_rate = *AAC_RATES.get(usize::from(d.freq_index))?;
                let asc = (u16::from(d.profile) << 11)
                    | (u16::from(d.freq_index) << 7)
                    | (u16::from(d.chan_conf) << 3);
                Some(AudioTrack {
                    id: t.track_id(),
                    timescale: t.timescale(),
                    sample_rate,
                    config: asc.to_be_bytes().to_vec(),
                    count: t.sample_count(),
                    starts_ms: starts_ms(t),
                })
            });
        let duration_ms = video.duration().as_millis() as u64;
        let info = Info {
            duration_ms: duration_ms.max(reader.duration().as_millis() as u64),
            width: u32::from(video.width()),
            height: u32::from(video.height()),
            has_audio: audio.is_some(),
        };
        Ok(Self {
            video_id: video.track_id(),
            video_timescale: video.timescale().max(1),
            video_count: video.sample_count(),
            length_size: usize::from(avc.avcc.length_size_minus_one & 3) + 1,
            parameter_sets,
            video_starts_ms,
            keyframes,
            audio,
            info,
            reader,
        })
    }

    pub fn video_samples(&self) -> u32 {
        self.video_count
    }

    pub fn video_sample(&mut self, number: u32) -> Option<VideoSample> {
        let sample = self.reader.read_sample(self.video_id, number).ok()??;
        let pts = (sample.start_time as i64 + i64::from(sample.rendering_offset)).max(0) as u64;
        let at_ms = pts * 1000 / u64::from(self.video_timescale);
        let mut annexb = Vec::with_capacity(sample.bytes.len() + self.parameter_sets.len() + 16);
        if sample.is_sync {
            annexb.extend_from_slice(&self.parameter_sets);
        }
        avcc_to_annexb(&sample.bytes, self.length_size, &mut annexb);
        Some(VideoSample { annexb, at_ms })
    }

    /// The keyframe at or before `ms`, as a sample number.
    pub fn keyframe_before(&self, ms: u64) -> u32 {
        let target = self.video_starts_ms.partition_point(|t| *t <= ms).max(1) as u32;
        if self.keyframes.is_empty() {
            return target;
        }
        let i = self.keyframes.partition_point(|k| *k <= target);
        self.keyframes
            .get(i.saturating_sub(1))
            .copied()
            .unwrap_or(1)
    }

    pub fn audio_packet(&mut self, number: u32) -> Option<Vec<u8>> {
        let id = self.audio.as_ref()?.id;
        let sample = self.reader.read_sample(id, number).ok()??;
        Some(sample.bytes.to_vec())
    }

    /// The first audio sample starting at or after `ms`.
    pub fn audio_sample_at(&self, ms: u64) -> u32 {
        self.audio
            .as_ref()
            .map_or(1, |a| a.starts_ms.partition_point(|t| *t < ms) as u32 + 1)
    }
}

/// Rewrites length-prefixed NAL units as start-code-prefixed ones.
fn avcc_to_annexb(mut data: &[u8], length_size: usize, out: &mut Vec<u8>) {
    while data.len() >= length_size {
        let len = data[..length_size]
            .iter()
            .fold(0usize, |n, b| (n << 8) | usize::from(*b));
        data = &data[length_size..];
        let len = len.min(data.len());
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&data[..len]);
        data = &data[len..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_nal_lengths() {
        let mut out = Vec::new();
        avcc_to_annexb(&[0, 0, 0, 2, 0x65, 0xAA, 0, 0, 0, 1, 0x41], 4, &mut out);
        assert_eq!(out, [0, 0, 0, 1, 0x65, 0xAA, 0, 0, 0, 1, 0x41]);
        let mut short = Vec::new();
        avcc_to_annexb(&[0, 9, 0x65], 2, &mut short);
        assert_eq!(short, [0, 0, 0, 1, 0x65]);
    }
}
