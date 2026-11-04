//! A streaming resampler for interleaved audio, used when a device does not run at Opus's
//! 48 kHz. Cubic Hermite interpolation keeps speech clean without a filter bank, and the
//! state carries across calls, so chunk boundaries are seamless.

#[derive(Debug, Clone)]
pub struct Resampler {
    channels: usize,
    /// Input frames per output frame.
    step: f64,
    /// Position of the next output frame in `buf`, in frames.
    pos: f64,
    buf: Vec<f32>,
}

impl Resampler {
    pub fn new(from_hz: u32, to_hz: u32, channels: usize) -> Self {
        let channels = channels.max(1);
        Self {
            channels,
            step: f64::from(from_hz) / f64::from(to_hz.max(1)),
            pos: 1.0,
            // One frame of history so the first output has a left neighbour.
            buf: vec![0.0; channels],
        }
    }

    pub fn is_identity(&self) -> bool {
        (self.step - 1.0).abs() < f64::EPSILON
    }

    /// Appends the resampled form of `input` to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.is_identity() {
            out.extend_from_slice(input);
            return;
        }
        let ch = self.channels;
        self.buf.extend_from_slice(input);
        let frames = self.buf.len() / ch;
        loop {
            let i = self.pos as usize;
            if i + 2 >= frames {
                break;
            }
            let t = (self.pos - i as f64) as f32;
            for c in 0..ch {
                let at = |k: usize| self.buf[k * ch + c];
                out.push(hermite(at(i - 1), at(i), at(i + 1), at(i + 2), t));
            }
            self.pos += self.step;
        }
        // Keep the frames the next call still needs.
        let keep_from = (self.pos as usize).saturating_sub(1).min(frames);
        self.buf.drain(..keep_from * ch);
        self.pos -= keep_from as f64;
    }
}

fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c0 = y1;
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + c0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_passes_through() {
        let mut r = Resampler::new(48_000, 48_000, 2);
        let mut out = Vec::new();
        r.process(&[1.0, 2.0, 3.0, 4.0], &mut out);
        assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn keeps_rate_and_shape_across_chunks() {
        // One second of a 440 Hz tone at 44.1 kHz, fed in uneven chunks.
        let input: Vec<f32> = (0..44_100)
            .map(|n| (n as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin())
            .collect();
        let mut r = Resampler::new(44_100, 48_000, 1);
        let mut out = Vec::new();
        for chunk in input.chunks(441 + 7) {
            r.process(chunk, &mut out);
        }
        assert!((out.len() as i64 - 48_000).abs() < 4, "{}", out.len());
        // Compare with the ideal tone at 48 kHz.
        let err = out
            .iter()
            .enumerate()
            .skip(10)
            .map(|(n, &y)| {
                let ideal = (n as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin();
                (y - ideal).abs()
            })
            .fold(0.0f32, f32::max);
        assert!(err < 0.01, "max error {err}");
    }

    #[test]
    fn interleaved_channels_stay_apart() {
        let mut r = Resampler::new(24_000, 48_000, 2);
        let input: Vec<f32> = (0..200).flat_map(|_| [0.5, -0.5]).collect();
        let mut out = Vec::new();
        r.process(&input, &mut out);
        for frame in out.chunks(2).skip(4) {
            assert!((frame[0] - 0.5).abs() < 1e-4 && (frame[1] + 0.5).abs() < 1e-4);
        }
    }
}
