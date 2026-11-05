//! A size-bounded image cache. GPUI's default keeps every decoded image for the life of
//! the window; a chat client scrolls past thousands, so this one evicts the least
//! recently drawn images once their decoded size passes the budget.
//!
//! Animated images are handled here too. GPUI's own `img` redraws its whole view on every
//! display refresh while an animated image is on screen, whatever the GIF's frame rate,
//! which kept rcord busy at 60 to 240 frames a second for one animated emoji. This cache
//! splits an animation into single-frame images (shrinking frames that are far larger than
//! anything rcord draws) and [`crate::ui::anim`] picks the frame for the current time and
//! wakes its view only when that frame changes.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::FutureExt;
use futures::future::Shared;
use gpui::{
    App, AppContext, Asset, AssetLogger, Entity, ImageAssetLoader, ImageCache, ImageCacheError,
    RenderImage, Resource, Task, Window, hash,
};
use image::{Frame, RgbaImage, imageops};

/// The main window's cache, for other windows to draw from the same images.
pub struct SharedImages(pub Entity<LruImageCache>);

impl gpui::Global for SharedImages {}

/// The longest side an animation is kept at. Nothing in rcord draws an animated image
/// wider than 520 logical pixels, and most are emoji and avatars under 100.
const MAX_ANIMATED_SIDE: u32 = 640;
/// The decoded size one animation may take. Larger ones lose resolution, not frames.
const MAX_ANIMATED_BYTES: usize = 24 * 1024 * 1024;
/// Browsers treat a GIF frame delay this short as broken and show it for 100 ms instead.
/// Without this, a GIF with zero delays would ask for a redraw on every refresh.
const MIN_FRAME_DELAY: Duration = Duration::from_millis(20);
const BROKEN_DELAY_REPLACEMENT: Duration = Duration::from_millis(100);

/// A decoded image, split into frames if it is animated.
pub struct Decoded {
    /// The first frame, or the whole image when it is still.
    pub still: Arc<RenderImage>,
    /// Every frame, as a single-frame image, when there are two or more.
    frames: Vec<Arc<RenderImage>>,
    /// When each frame ends, measured from the start of the loop.
    ends: Vec<Duration>,
    bytes: usize,
}

impl Decoded {
    fn from_render(image: Arc<RenderImage>) -> Self {
        let count = image.frame_count();
        if count < 2 {
            let bytes = frame_bytes(&image, 0);
            return Self {
                still: image,
                frames: Vec::new(),
                ends: Vec::new(),
                bytes,
            };
        }
        let total: usize = (0..count).map(|i| frame_bytes(&image, i)).sum();
        let largest = (0..count)
            .map(|i| {
                let s = image.size(i);
                s.width.0.max(s.height.0).max(1) as u32
            })
            .max()
            .unwrap_or(1);
        let scale = shrink_factor(largest, total);

        let mut frames = Vec::with_capacity(count);
        let mut ends = Vec::with_capacity(count);
        let mut at = Duration::ZERO;
        let mut bytes = 0;
        for i in 0..count {
            let size = image.size(i);
            let (w, h) = (size.width.0.max(1) as u32, size.height.0.max(1) as u32);
            let Some(buffer) = image
                .as_bytes(i)
                .and_then(|raw| RgbaImage::from_raw(w, h, raw.to_vec()))
            else {
                continue;
            };
            // The bytes are BGRA, not RGBA; resizing treats every channel the same, so
            // the order does not matter.
            let buffer = if scale < 1.0 {
                let nw = ((w as f32 * scale).round() as u32).max(1);
                let nh = ((h as f32 * scale).round() as u32).max(1);
                imageops::resize(&buffer, nw, nh, imageops::FilterType::Triangle)
            } else {
                buffer
            };
            bytes += buffer.as_raw().len();
            at += frame_delay(&image, i);
            ends.push(at);
            frames.push(Arc::new(RenderImage::new([Frame::new(buffer)])));
        }
        match frames.len() {
            0 => Self {
                still: image,
                frames: Vec::new(),
                ends: Vec::new(),
                bytes: total,
            },
            1 => Self {
                still: frames.pop().unwrap_or(image),
                frames: Vec::new(),
                ends: Vec::new(),
                bytes,
            },
            _ => Self {
                still: frames[0].clone(),
                frames,
                ends,
                bytes,
            },
        }
    }

    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    /// The frame to show `elapsed` after the animation started, and how long it stays.
    pub fn frame_at(&self, elapsed: Duration) -> (Arc<RenderImage>, Duration) {
        let Some(&total) = self.ends.last() else {
            return (self.still.clone(), Duration::MAX);
        };
        let total_ns = total.as_nanos().max(1);
        let pos = Duration::from_nanos((elapsed.as_nanos() % total_ns) as u64);
        let ix = self.ends.partition_point(|end| *end <= pos);
        let ix = ix.min(self.frames.len() - 1);
        (self.frames[ix].clone(), self.ends[ix].saturating_sub(pos))
    }

    fn images(&self) -> impl Iterator<Item = &Arc<RenderImage>> {
        std::iter::once(&self.still).chain(self.frames.iter().skip(1))
    }
}

fn frame_bytes(image: &RenderImage, i: usize) -> usize {
    let s = image.size(i);
    (s.width.0.max(0) as usize) * (s.height.0.max(0) as usize) * 4
}

fn frame_delay(image: &RenderImage, i: usize) -> Duration {
    let (num, den) = image.delay(i).numer_denom_ms();
    let delay = Duration::from_micros(u64::from(num) * 1000 / u64::from(den.max(1)));
    if delay <= Duration::from_millis(10) {
        BROKEN_DELAY_REPLACEMENT
    } else {
        delay.max(MIN_FRAME_DELAY)
    }
}

/// How much to shrink an animation whose largest frame side is `side` pixels and whose
/// frames take `bytes` decoded. 1.0 keeps it as it is.
fn shrink_factor(side: u32, bytes: usize) -> f32 {
    let by_side = MAX_ANIMATED_SIDE as f32 / side.max(1) as f32;
    let by_bytes = (MAX_ANIMATED_BYTES as f32 / bytes.max(1) as f32).sqrt();
    by_side.min(by_bytes).min(1.0)
}

type Load = Shared<Task<Result<Arc<Decoded>, ImageCacheError>>>;

struct Entry {
    load: Load,
    bytes: usize,
    last_used: u64,
    /// When the image was first drawn, so its animation starts at its first frame.
    started: Option<Instant>,
}

pub struct LruImageCache {
    entries: HashMap<u64, Entry>,
    budget: usize,
    used: usize,
    clock: u64,
}

impl LruImageCache {
    pub fn new(budget_bytes: usize, cx: &mut App) -> Entity<Self> {
        let cache = cx.new(|_| Self {
            entries: HashMap::new(),
            budget: budget_bytes,
            used: 0,
            clock: 0,
        });
        cx.observe_release(&cache, |cache, cx| {
            for (_, entry) in cache.entries.drain() {
                if let Some(Ok(decoded)) = entry.load.now_or_never() {
                    for image in decoded.images() {
                        cx.drop_image(image.clone(), None);
                    }
                }
            }
        })
        .detach();
        cache
    }

    pub fn set_budget(&mut self, bytes: usize) {
        self.budget = bytes;
    }

    /// The loaded image for `resource`, if it has finished loading, and when its animation
    /// started. Marks it as drawn.
    pub fn loaded(&mut self, resource: &Resource) -> Option<(Arc<Decoded>, Instant)> {
        let entry = self.entries.get_mut(&hash(resource))?;
        let decoded = entry.load.clone().now_or_never()?.ok()?;
        self.clock += 1;
        entry.last_used = self.clock;
        let started = *entry.started.get_or_insert_with(Instant::now);
        Some((decoded, started))
    }

    fn evict(&mut self, keep: u64, window: &mut Window, cx: &mut App) {
        while self.used > self.budget && self.entries.len() > 1 {
            let Some((&oldest, _)) = self
                .entries
                .iter()
                .filter(|(k, e)| **k != keep && e.bytes > 0)
                .min_by_key(|(_, e)| e.last_used)
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.used = self.used.saturating_sub(entry.bytes);
                if let Some(Ok(decoded)) = entry.load.now_or_never() {
                    for image in decoded.images() {
                        cx.drop_image(image.clone(), Some(window));
                    }
                }
            }
        }
    }
}

impl ImageCache for LruImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let key = hash(resource);
        self.clock += 1;
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = self.clock;
            let result = entry.load.clone().now_or_never()?;
            if entry.bytes == 0
                && let Ok(decoded) = &result
            {
                entry.bytes = decoded.bytes.max(1);
                self.used += entry.bytes;
                self.evict(key, window, cx);
            }
            return Some(result.map(|d| d.still.clone()));
        }

        let fut = AssetLogger::<ImageAssetLoader>::load(resource.clone(), cx);
        let load = cx
            .background_executor()
            .spawn(async move { fut.await.map(|image| Arc::new(Decoded::from_render(image))) })
            .shared();
        self.entries.insert(
            key,
            Entry {
                load: load.clone(),
                bytes: 0,
                last_used: self.clock,
                started: None,
            },
        );
        let view = window.current_view();
        window
            .spawn(cx, async move |cx| {
                let _ = load.await;
                cx.on_next_frame(move |_, cx| cx.notify(view));
            })
            .detach();
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animation(w: u32, h: u32, delays_ms: &[u32]) -> Arc<RenderImage> {
        let frames: Vec<Frame> = delays_ms
            .iter()
            .map(|&d| {
                Frame::from_parts(
                    RgbaImage::new(w, h),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(d, 1),
                )
            })
            .collect();
        Arc::new(RenderImage::new(frames))
    }

    #[test]
    fn still_images_are_kept_whole() {
        let image = animation(10, 10, &[0]);
        let d = Decoded::from_render(image.clone());
        assert!(!d.is_animated());
        assert_eq!(d.still.id, image.id);
        assert_eq!(d.bytes, 400);
    }

    #[test]
    fn frames_follow_their_delays() {
        let d = Decoded::from_render(animation(4, 4, &[50, 100, 50]));
        assert!(d.is_animated());
        let at = |ms| d.frame_at(Duration::from_millis(ms));
        assert_eq!(at(0).0.id, d.frames[0].id);
        assert_eq!(at(0).1, Duration::from_millis(50));
        assert_eq!(at(50).0.id, d.frames[1].id);
        assert_eq!(at(120).1, Duration::from_millis(30));
        assert_eq!(at(160).0.id, d.frames[2].id);
        // The animation loops.
        assert_eq!(at(200).0.id, d.frames[0].id);
    }

    #[test]
    fn zero_delays_play_like_a_browser() {
        let d = Decoded::from_render(animation(2, 2, &[0, 0]));
        assert_eq!(
            d.ends,
            vec![Duration::from_millis(100), Duration::from_millis(200)]
        );
    }

    #[test]
    fn large_animations_are_shrunk() {
        let d = Decoded::from_render(animation(1280, 720, &[40, 40]));
        assert_eq!(d.frames[0].size(0).width.0, 640);
        assert_eq!(d.frames[0].size(0).height.0, 360);
        assert!(d.bytes <= MAX_ANIMATED_BYTES);
    }

    #[test]
    fn long_animations_fit_the_byte_budget() {
        // 500×280 with 120 frames is 67 MB decoded.
        let f = shrink_factor(500, 500 * 280 * 4 * 120);
        assert!(f > 0.6 && f < 0.62, "{f}");
        assert_eq!(shrink_factor(128, 128 * 128 * 4 * 30), 1.0);
    }
}
