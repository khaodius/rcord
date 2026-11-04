//! H.264 video for Go Live: captured frames are scaled and converted to I420 and encoded
//! with OpenH264; received access units are decoded back to BGRA for display.
//!
//! The colour conversion is BT.601 limited range, which is what OpenH264 signals by default
//! and what Discord's clients assume when a stream carries no colour description.

use openh264::OpenH264API;
use openh264::decoder::{Decoder, DecoderConfig};
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod, Level,
    Profile, RateControlMode, SpsPpsStrategy, UsageType,
};
use openh264::formats::YUVSource;

#[derive(Debug, thiserror::Error)]
#[error("video codec: {0}")]
pub struct VideoError(String);

impl From<openh264::Error> for VideoError {
    fn from(e: openh264::Error) -> Self {
        Self(e.to_string())
    }
}

/// A picture to encode: packed 32-bit pixels, rows without padding.
#[derive(Debug, Clone, Copy)]
pub struct RawFrame<'a> {
    pub width: u32,
    pub height: u32,
    /// True for B, G, R, x byte order; false for R, G, B, x.
    pub bgra: bool,
    pub data: &'a [u8],
}

/// One encoded access unit in Annex B form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub keyframe: bool,
    pub width: u32,
    pub height: u32,
}

/// Stream quality: the largest size frames are scaled to, the frame rate, and the bitrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoQuality {
    pub max_width: u32,
    pub max_height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
}

impl VideoQuality {
    pub const P720_30: Self = Self {
        max_width: 1280,
        max_height: 720,
        fps: 30,
        bitrate_kbps: 2500,
    };
}

/// Keeps a stream decodable for viewers who join late or lose packets.
const KEYFRAME_SECONDS: u32 = 5;

pub struct VideoEncoder {
    quality: VideoQuality,
    encoder: Option<Encoder>,
    yuv: Option<I420>,
    size: (u32, u32),
    force_keyframe: bool,
}

impl std::fmt::Debug for VideoEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoEncoder")
            .field("quality", &self.quality)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl VideoEncoder {
    pub fn new(quality: VideoQuality) -> Self {
        Self {
            quality,
            encoder: None,
            yuv: None,
            size: (0, 0),
            force_keyframe: false,
        }
    }

    pub fn quality(&self) -> VideoQuality {
        self.quality
    }

    /// The size frames are sent at, once the first frame has been seen.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Makes the next frame a keyframe, for a viewer that lost the picture.
    pub fn request_keyframe(&mut self) {
        self.force_keyframe = true;
    }

    fn open(&self) -> Result<Encoder, VideoError> {
        let q = self.quality;
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .profile(Profile::Baseline)
            .level(Level::Level_4_1)
            .bitrate(BitRate::from_bps(q.bitrate_kbps * 1000))
            .max_frame_rate(FrameRate::from_hz(q.fps as f32))
            .rate_control_mode(RateControlMode::Bitrate)
            .sps_pps_strategy(SpsPpsStrategy::ConstantId)
            .intra_frame_period(IntraFramePeriod::from_num_frames(q.fps * KEYFRAME_SECONDS))
            .complexity(Complexity::Low)
            .skip_frames(true)
            .scene_change_detect(true);
        Ok(Encoder::with_api_config(
            OpenH264API::from_source(),
            config,
        )?)
    }

    /// Encodes a frame, scaling it down to fit the quality's limits. Returns `None` when
    /// the encoder skips the frame to hold its bitrate.
    pub fn encode(&mut self, frame: RawFrame<'_>) -> Result<Option<EncodedFrame>, VideoError> {
        let needed = frame.width as usize * frame.height as usize * 4;
        if frame.width < 2 || frame.height < 2 || frame.data.len() < needed {
            return Err(VideoError("frame is empty or truncated".into()));
        }
        let size = fit(
            frame.width,
            frame.height,
            self.quality.max_width,
            self.quality.max_height,
        );
        if size != self.size || self.encoder.is_none() {
            // A new size needs a fresh sequence, which starts with a keyframe anyway.
            self.encoder = Some(self.open()?);
            self.yuv = Some(I420::new(size.0 as usize, size.1 as usize));
            self.size = size;
            self.force_keyframe = false;
        }
        let (Some(encoder), Some(yuv)) = (&mut self.encoder, &mut self.yuv) else {
            return Ok(None);
        };
        to_i420(frame, size, yuv);
        if std::mem::take(&mut self.force_keyframe) {
            encoder.force_intra_frame();
        }
        let out = encoder.encode(&*yuv)?;
        let kind = out.frame_type();
        if matches!(kind, FrameType::Skip | FrameType::Invalid) {
            return Ok(None);
        }
        let data = out.to_vec();
        if data.is_empty() {
            return Ok(None);
        }
        Ok(Some(EncodedFrame {
            data,
            keyframe: matches!(kind, FrameType::IDR | FrameType::I),
            width: size.0,
            height: size.1,
        }))
    }
}

/// The largest even size within `max_w` × `max_h` with the source's aspect ratio.
pub fn fit(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let scale = (f64::from(max_w) / f64::from(w))
        .min(f64::from(max_h) / f64::from(h))
        .min(1.0);
    let even = |v: f64| ((v.round() as u32) & !1).max(2);
    (even(f64::from(w) * scale), even(f64::from(h) * scale))
}

/// Converts and scales packed RGB into the I420 buffer, averaging the source pixels each
/// output pixel covers along both axes so text stays legible when shrunk.
fn to_i420(frame: RawFrame<'_>, (dw, dh): (u32, u32), yuv: &mut I420) {
    let (sw, sh) = (frame.width as usize, frame.height as usize);
    let (dw, dh) = (dw as usize, dh as usize);
    let (ri, bi) = if frame.bgra { (2, 0) } else { (0, 2) };
    let src = frame.data;

    // Source spans for each output column and row.
    let span = |d: usize, s: usize| -> Vec<(usize, usize)> {
        (0..d)
            .map(|i| {
                let a = i * s / d;
                let b = ((i + 1) * s / d).max(a + 1).min(s);
                (a, b)
            })
            .collect()
    };
    let cols = span(dw, sw);
    let rows = span(dh, sh);
    // Average up to two samples per axis: cheap and smooth enough for screens.
    let picks = |(a, b): (usize, usize)| -> [usize; 2] { [a, (b - 1).max(a)] };

    let mut rgb = vec![[0i32; 3]; dw * dh];
    for (y, &row) in rows.iter().enumerate() {
        let ys = picks(row);
        for (x, &col) in cols.iter().enumerate() {
            let xs = picks(col);
            let mut acc = [0i32; 3];
            for sy in ys {
                let line = &src[sy * sw * 4..];
                for sx in xs {
                    let p = &line[sx * 4..sx * 4 + 4];
                    acc[0] += i32::from(p[ri]);
                    acc[1] += i32::from(p[1]);
                    acc[2] += i32::from(p[bi]);
                }
            }
            rgb[y * dw + x] = [acc[0] >> 2, acc[1] >> 2, acc[2] >> 2];
        }
    }

    let (y_len, c_len) = (dw * dh, (dw / 2) * (dh / 2));
    let buf = &mut yuv.data;
    let (yp, rest) = buf.split_at_mut(y_len);
    let (up, vp) = rest.split_at_mut(c_len);
    for (i, [r, g, b]) in rgb.iter().copied().enumerate() {
        yp[i] = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
    }
    for cy in 0..dh / 2 {
        for cx in 0..dw / 2 {
            let mut s = [0i32; 3];
            for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                let p = rgb[(cy * 2 + dy) * dw + cx * 2 + dx];
                s[0] += p[0];
                s[1] += p[1];
                s[2] += p[2];
            }
            let [r, g, b] = [s[0] >> 2, s[1] >> 2, s[2] >> 2];
            let i = cy * (dw / 2) + cx;
            up[i] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
            vp[i] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
        }
    }
}

/// A planar I420 picture: the luma plane, then the two quarter-size chroma planes.
struct I420 {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

impl I420 {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: vec![0; width * height * 3 / 2],
        }
    }
}

impl YUVSource for I420 {
    fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn strides(&self) -> (usize, usize, usize) {
        (self.width, self.width / 2, self.width / 2)
    }

    fn y(&self) -> &[u8] {
        &self.data[..self.width * self.height]
    }

    fn u(&self) -> &[u8] {
        let y = self.width * self.height;
        &self.data[y..y + y / 4]
    }

    fn v(&self) -> &[u8] {
        let y = self.width * self.height;
        &self.data[y + y / 4..]
    }
}

/// A decoded picture ready to draw: BGRA, rows packed.
#[derive(Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl std::fmt::Debug for DecodedFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecodedFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

pub struct VideoDecoder {
    decoder: Decoder,
}

impl std::fmt::Debug for VideoDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoDecoder").finish_non_exhaustive()
    }
}

impl VideoDecoder {
    pub fn new() -> Result<Self, VideoError> {
        let decoder = Decoder::with_api_config(OpenH264API::from_source(), DecoderConfig::new())?;
        Ok(Self { decoder })
    }

    /// Decodes one access unit. Returns `None` while the decoder waits for more data, such
    /// as a keyframe after joining mid-stream.
    pub fn decode(&mut self, access_unit: &[u8]) -> Result<Option<DecodedFrame>, VideoError> {
        let Some(pic) = self.decoder.decode(access_unit)? else {
            return Ok(None);
        };
        let (w, h) = pic.dimensions();
        let (ys, us, vs) = pic.strides();
        Ok(Some(DecodedFrame {
            width: w as u32,
            height: h as u32,
            bgra: i420_to_bgra(pic.y(), pic.u(), pic.v(), (w, h), (ys, us, vs)),
        }))
    }
}

/// BT.601 limited-range I420 to packed BGRA.
pub fn i420_to_bgra(
    y: &[u8],
    u: &[u8],
    v: &[u8],
    (w, h): (usize, usize),
    (ys, us, vs): (usize, usize, usize),
) -> Vec<u8> {
    let mut out = vec![255u8; w * h * 4];
    for row in 0..h {
        let yl = &y[row * ys..row * ys + w];
        let ul = &u[(row / 2) * us..];
        let vl = &v[(row / 2) * vs..];
        let dst = &mut out[row * w * 4..(row + 1) * w * 4];
        for (col, px) in dst.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let c = 298 * (i32::from(yl[col]) - 16);
            let d = i32::from(ul[col / 2]) - 128;
            let e = i32::from(vl[col / 2]) - 128;
            px[0] = ((c + 516 * d + 128) >> 8).clamp(0, 255) as u8;
            px[1] = ((c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
            px[2] = ((c + 409 * e + 128) >> 8).clamp(0, 255) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_within_limits_keeping_aspect() {
        assert_eq!(fit(2560, 1440, 1280, 720), (1280, 720));
        assert_eq!(fit(1920, 1200, 1280, 720), (1152, 720));
        assert_eq!(fit(801, 601, 1280, 720), (800, 600));
        assert_eq!(fit(3, 3, 1280, 720), (2, 2));
    }

    #[test]
    fn encodes_and_decodes_a_picture() {
        let (w, h) = (320u32, 180u32);
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for (i, px) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x = (i as u32 % w) as u8;
            px.copy_from_slice(&[x, 128, 255 - x, 255]);
        }
        let mut enc = VideoEncoder::new(VideoQuality {
            max_width: 160,
            max_height: 90,
            fps: 30,
            bitrate_kbps: 800,
        });
        let frame = RawFrame {
            width: w,
            height: h,
            bgra: true,
            data: &pixels,
        };
        let first = enc.encode(frame).unwrap().expect("first frame is encoded");
        assert!(first.keyframe);
        assert!(crate::h264::is_keyframe(&first.data));
        assert_eq!((first.width, first.height), (160, 90));

        let mut dec = VideoDecoder::new().unwrap();
        let pic = dec
            .decode(&first.data)
            .unwrap()
            .expect("a keyframe decodes");
        assert_eq!((pic.width, pic.height), (160, 90));
        // Middle row, green channel: close to the source after the round trip.
        let mid = ((45 * 160 + 80) * 4) as usize;
        assert!((i32::from(pic.bgra[mid + 1]) - 128).abs() < 16);

        enc.request_keyframe();
        let again = enc.encode(frame).unwrap().expect("forced keyframe");
        assert!(again.keyframe);
    }

    #[test]
    fn grey_survives_conversion() {
        let (w, h) = (4usize, 2usize);
        let y = vec![126u8; w * h];
        let u = vec![128u8; 2];
        let v = vec![128u8; 2];
        let out = i420_to_bgra(&y, &u, &v, (w, h), (w, 2, 2));
        assert!(
            out.as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[0] == p[1] && p[1] == p[2])
        );
    }
}
