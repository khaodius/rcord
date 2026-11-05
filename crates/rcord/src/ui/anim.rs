//! Animated images (GIF, animated WebP, APNG) drawn from [`LruImageCache`]'s frames.
//!
//! GPUI's `img` keeps an animation going by redrawing its view on every display refresh.
//! [`AnimImg`] instead draws the frame due now and asks for one redraw when the next
//! frame is due, so a 10 fps GIF costs 10 redraws a second rather than 144. Animations
//! pause while their window is in the background, as Discord's do; GPUI redraws every
//! view when the window becomes active again, which starts them where they left off.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, EntityId, Global, ImageSource, IntoElement, ObjectFit, RenderOnce,
    SharedString, StyleRefinement, Styled, Window, img, prelude::*,
};

use crate::image_cache::SharedImages;

/// Redraws for animations are lined up on this grid, so several images on screen share
/// one redraw instead of each asking for its own. It also caps animations at 30 fps,
/// which almost every GIF, animated emoji and avatar already plays at or below.
const TICK: Duration = Duration::from_micros(33_333);

/// An image that plays if it is animated.
#[derive(IntoElement)]
pub struct AnimImg {
    src: SharedString,
    style: StyleRefinement,
    fit: Option<ObjectFit>,
    grayscale: bool,
    fallback: Option<Box<dyn Fn() -> AnyElement>>,
}

pub fn anim_img(src: impl Into<SharedString>) -> AnimImg {
    AnimImg {
        src: src.into(),
        style: StyleRefinement::default(),
        fit: None,
        grayscale: false,
        fallback: None,
    }
}

impl AnimImg {
    pub fn object_fit(mut self, fit: ObjectFit) -> Self {
        self.fit = Some(fit);
        self
    }

    pub fn grayscale(mut self, grayscale: bool) -> Self {
        self.grayscale = grayscale;
        self
    }

    pub fn with_fallback(mut self, fallback: impl Fn() -> AnyElement + 'static) -> Self {
        self.fallback = Some(Box::new(fallback));
        self
    }
}

impl Styled for AnimImg {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for AnimImg {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let source = current_frame(&self.src, window, cx)
            .map(ImageSource::Render)
            .unwrap_or_else(|| self.src.clone().into());
        let mut el = img(source).grayscale(self.grayscale);
        *el.style() = self.style;
        if let Some(fit) = self.fit {
            el = el.object_fit(fit);
        }
        if let Some(fallback) = self.fallback {
            el = el.with_fallback(fallback);
        }
        el
    }
}

/// The frame of `src` to draw now if it is a loaded animation, after arranging a redraw
/// for when the next frame is due. `None` leaves loading and still images to GPUI.
fn current_frame(
    src: &SharedString,
    window: &mut Window,
    cx: &mut App,
) -> Option<std::sync::Arc<gpui::RenderImage>> {
    let images = cx.try_global::<SharedImages>()?.0.clone();
    let resource = gpui::Resource::Uri(src.clone().into());
    let (decoded, started) = images.update(cx, |cache, _| cache.loaded(&resource))?;
    if !decoded.is_animated() {
        return None;
    }
    let now = Instant::now();
    let (frame, remaining) = decoded.frame_at(now - started);
    if window.is_window_active() {
        schedule(now + remaining, window, cx);
    }
    Some(frame)
}

/// When each view's next animation redraw is due.
#[derive(Default)]
struct Pending(HashMap<EntityId, Instant>);

impl Global for Pending {}

fn schedule(due: Instant, window: &mut Window, cx: &mut App) {
    let due = on_grid(due);
    let view = window.current_view();
    let pending = cx.default_global::<Pending>();
    if pending.0.get(&view).is_some_and(|at| *at <= due) {
        return;
    }
    pending.0.insert(view, due);
    window
        .spawn(cx, async move |cx| {
            cx.background_executor()
                .timer(due.saturating_duration_since(Instant::now()))
                .await;
            cx.update(|_, cx| {
                let pending = cx.default_global::<Pending>();
                if pending.0.get(&view) == Some(&due) {
                    pending.0.remove(&view);
                    cx.notify(view);
                }
            })
            .ok();
        })
        .detach();
}

/// Rounds `at` up to the next [`TICK`] boundary, counted from a fixed start.
fn on_grid(at: Instant) -> Instant {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let epoch = *EPOCH.get_or_init(Instant::now);
    let since = at.saturating_duration_since(epoch).as_nanos();
    let tick = TICK.as_nanos();
    let ticks = since.div_ceil(tick).max(1);
    epoch + Duration::from_nanos((ticks * tick) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadlines_round_up_to_the_grid() {
        let a = on_grid(Instant::now());
        let b = on_grid(a + Duration::from_millis(1));
        assert_eq!(b - a, TICK);
        assert_eq!(on_grid(a), a);
    }
}
