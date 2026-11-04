//! The video player that opens over the window when a video in chat is clicked. The file is
//! downloaded whole, then decoded and played by `rcord-media`; frames arrive as BGRA pixels
//! and are shown as images, each one dropped from the GPU when the next replaces it.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, Bounds, Context, FocusHandle, Focusable, FontWeight, ImageSource, IntoElement,
    KeyDownEvent, MouseButton, ObjectFit, Pixels, Render, RenderImage, Task, Window, canvas, div,
    img, prelude::*, px,
};
use rcord_media::{Event, Info, Player, format_time};

use crate::net::Net;
use crate::state::AppState;
use crate::theme::{Theme, theme};
use crate::ui::Tooltip;
use crate::ui::icons::{Icon, icon};

/// Larger files open in the browser instead of being held in memory.
const MAX_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoSource {
    pub url: String,
    pub filename: String,
    pub size: u64,
    pub width: u32,
    pub height: u32,
}

impl VideoSource {
    /// Whether rcord can try to play the file: MP4 and MOV, the containers H.264 comes in.
    pub fn playable(filename: &str, content_type: Option<&str>, size: u64) -> bool {
        let name = filename.to_ascii_lowercase();
        let by_name = [".mp4", ".m4v", ".mov"].iter().any(|e| name.ends_with(e));
        let by_type = matches!(content_type, Some("video/mp4" | "video/quicktime"));
        (by_name || by_type) && size <= MAX_BYTES
    }
}

enum Stage {
    Downloading,
    Ready(Info),
    Failed(String),
}

pub struct VideoPlayer {
    state: gpui::Entity<AppState>,
    pub source: VideoSource,
    stage: Stage,
    player: Option<Player>,
    image: Option<Arc<RenderImage>>,
    position_ms: u64,
    playing: bool,
    ended: bool,
    volume: f32,
    muted: bool,
    expanded: bool,
    focus: FocusHandle,
    bar: Rc<Cell<Option<Bounds<Pixels>>>>,
    volume_bar: Rc<Cell<Option<Bounds<Pixels>>>>,
    _tasks: Vec<Task<()>>,
}

impl VideoPlayer {
    pub fn new(
        state: gpui::Entity<AppState>,
        source: VideoSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let url = source.url.clone();
        let fetch = cx.global::<Net>().run(download(url));
        let load = cx.spawn_in(window, async move |this, cx| {
            let result = fetch
                .await
                .unwrap_or_else(|| Err("the download stopped".into()));
            this.update_in(cx, |this, window, cx| match result {
                Ok(bytes) => this.start(bytes, window, cx),
                Err(e) => {
                    this.stage = Stage::Failed(format!("The video could not be downloaded: {e}"));
                    cx.notify();
                }
            })
            .ok();
        });
        Self {
            state,
            source,
            stage: Stage::Downloading,
            player: None,
            image: None,
            position_ms: 0,
            playing: false,
            ended: false,
            volume: 1.0,
            muted: false,
            expanded: false,
            focus: cx.focus_handle(),
            bar: Rc::default(),
            volume_bar: Rc::default(),
            _tasks: vec![load],
        }
    }

    fn start(&mut self, bytes: Vec<u8>, window: &mut Window, cx: &mut Context<Self>) {
        let (player, channels) = Player::open(bytes);
        let events = channels.events;
        let frames = channels.frames;
        self._tasks.push(cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let gone = this
                    .update(cx, |this, cx| {
                        match event {
                            Event::Ready(info) => {
                                this.stage = Stage::Ready(info);
                                if let Some(p) = &this.player {
                                    p.play();
                                    this.playing = true;
                                }
                            }
                            Event::Ended => {
                                this.playing = false;
                                this.ended = true;
                            }
                            Event::Failed(e) => this.stage = Stage::Failed(e),
                        }
                        cx.notify();
                    })
                    .is_err();
                if gone {
                    break;
                }
            }
        }));
        self._tasks.push(cx.spawn_in(window, async move |this, cx| {
            while let Ok(frame) = frames.recv().await {
                let Some(buffer) =
                    image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra)
                else {
                    continue;
                };
                let picture = Arc::new(RenderImage::new([image::Frame::new(buffer)]));
                let gone = this
                    .update_in(cx, |this, window, cx| {
                        this.position_ms = frame.at_ms;
                        if let Some(old) = this.image.replace(picture) {
                            cx.drop_image(old, Some(window));
                        }
                        cx.notify();
                    })
                    .is_err();
                if gone {
                    break;
                }
            }
        }));
        player.set_volume(self.effective_volume());
        self.player = Some(player);
    }

    fn duration(&self) -> u64 {
        match &self.stage {
            Stage::Ready(info) => info.duration_ms,
            _ => 0,
        }
    }

    fn effective_volume(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume }
    }

    fn toggle_play(&mut self, cx: &mut Context<Self>) {
        let Some(p) = &self.player else { return };
        if self.playing {
            p.pause();
        } else {
            p.play();
            self.ended = false;
        }
        self.playing = !self.playing;
        cx.notify();
    }

    fn seek(&mut self, ms: u64, cx: &mut Context<Self>) {
        let Some(p) = &self.player else { return };
        let ms = ms.min(self.duration());
        p.seek(ms);
        if self.ended {
            self.ended = false;
            p.play();
            self.playing = true;
        }
        self.position_ms = ms;
        cx.notify();
    }

    fn seek_by(&mut self, delta: i64, cx: &mut Context<Self>) {
        let to = (self.position_ms as i64 + delta).max(0) as u64;
        self.seek(to, cx);
    }

    fn set_volume(&mut self, v: f32, cx: &mut Context<Self>) {
        self.volume = v.clamp(0.0, 1.0);
        self.muted = self.volume == 0.0;
        if let Some(p) = &self.player {
            p.set_volume(self.effective_volume());
        }
        cx.notify();
    }

    fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        self.muted = !self.muted;
        if !self.muted && self.volume == 0.0 {
            self.volume = 1.0;
        }
        if let Some(p) = &self.player {
            p.set_volume(self.effective_volume());
        }
        cx.notify();
    }

    fn close(&self, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| s.set_overlay(None, cx));
    }

    fn open_in_browser(&self, cx: &mut Context<Self>) {
        cx.open_url(&self.source.url);
        self.close(cx);
    }

    fn on_key(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        match e.keystroke.key.as_str() {
            "space" | "k" => self.toggle_play(cx),
            "left" | "j" => self.seek_by(-5_000, cx),
            "right" | "l" => self.seek_by(5_000, cx),
            "m" => self.toggle_mute(cx),
            "f" => {
                self.expanded = !self.expanded;
                cx.notify();
            }
            "escape" => {
                if self.expanded {
                    self.expanded = false;
                    cx.notify();
                } else {
                    self.close(cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn picture(&self, t: &Theme) -> AnyElement {
        match (&self.stage, &self.image) {
            (Stage::Failed(e), _) => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .p(px(24.))
                .text_color(gpui::white())
                .child(icon(Icon::Video).size(px(36.)).text_color(t.text_faint))
                .child(div().text_center().child(e.clone()))
                .into_any_element(),
            (_, Some(image)) => img(ImageSource::Render(image.clone()))
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            _ => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.text_faint)
                .child(match self.stage {
                    Stage::Downloading => "Downloading the video…",
                    _ => "Starting…",
                })
                .into_any_element(),
        }
    }

    fn controls(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let duration = self.duration();
        let fraction = if duration == 0 {
            0.0
        } else {
            (self.position_ms.min(duration) as f32 / duration as f32).clamp(0.0, 1.0)
        };
        let button = |id: &'static str, glyph: Icon, tip: &'static str| {
            div()
                .id(id)
                .size(px(30.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .text_color(gpui::white())
                .hover(|s| s.bg(gpui::white().opacity(0.15)))
                .tooltip(move |_, cx| Tooltip::view(tip, cx))
                .child(icon(glyph).size(px(18.)))
        };
        let bar = self.bar.clone();
        let seek_bar = self.bar.clone();
        let drag_bar = self.bar.clone();
        let vol_bounds = self.volume_bar.clone();
        let vol_click = self.volume_bar.clone();
        let vol_drag = self.volume_bar.clone();
        let volume = self.effective_volume();
        let fraction_at = |b: &Rc<Cell<Option<Bounds<Pixels>>>>, x: Pixels| -> Option<f32> {
            let b = b.get()?;
            let w = f32::from(b.size.width);
            (w > 0.).then(|| (f32::from(x - b.origin.x) / w).clamp(0.0, 1.0))
        };
        let track = |fill: f32, color: gpui::Hsla| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(7.))
                .h(px(4.))
                .rounded_full()
                .bg(gpui::white().opacity(0.25))
                .child(
                    div()
                        .h_full()
                        .rounded_full()
                        .bg(color)
                        .w(gpui::relative(fill)),
                )
        };
        let knob = |fill: f32| {
            div()
                .absolute()
                .top(px(3.))
                .left(gpui::relative(fill))
                .ml(px(-6.))
                .size(px(12.))
                .rounded_full()
                .bg(gpui::white())
        };
        div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .px(px(12.))
            .pt(px(6.))
            .pb(px(8.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .bg(gpui::black().opacity(0.6))
            .child(
                div()
                    .id("video-seek")
                    .relative()
                    .h(px(18.))
                    .w_full()
                    .cursor_pointer()
                    .child(
                        canvas(move |b, _, _| bar.set(Some(b)), |_, _, _, _| {})
                            .absolute()
                            .size_full(),
                    )
                    .child(track(fraction, t.accent))
                    .child(knob(fraction))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                            if let Some(f) = fraction_at(&seek_bar, e.position.x) {
                                let ms = (f as f64 * this.duration() as f64) as u64;
                                this.seek(ms, cx);
                            }
                            cx.stop_propagation();
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, e: &gpui::MouseMoveEvent, _, cx| {
                        if e.pressed_button == Some(MouseButton::Left)
                            && let Some(f) = fraction_at(&drag_bar, e.position.x)
                        {
                            let ms = (f as f64 * this.duration() as f64) as u64;
                            this.seek(ms, cx);
                        }
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        button(
                            "video-play",
                            if self.playing {
                                Icon::Pause
                            } else {
                                Icon::Play
                            },
                            if self.playing {
                                "Pause (Space)"
                            } else {
                                "Play (Space)"
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx))),
                    )
                    .child(
                        button(
                            "video-mute",
                            if volume == 0.0 {
                                Icon::VolumeOff
                            } else {
                                Icon::Volume
                            },
                            "Mute (M)",
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_mute(cx))),
                    )
                    .child(
                        div()
                            .id("video-volume")
                            .relative()
                            .w(px(80.))
                            .h(px(18.))
                            .flex_none()
                            .cursor_pointer()
                            .child(
                                canvas(move |b, _, _| vol_bounds.set(Some(b)), |_, _, _, _| {})
                                    .absolute()
                                    .size_full(),
                            )
                            .child(track(volume, gpui::white()))
                            .child(knob(volume))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                                    if let Some(f) = fraction_at(&vol_click, e.position.x) {
                                        this.set_volume(f, cx);
                                    }
                                    cx.stop_propagation();
                                }),
                            )
                            .on_mouse_move(cx.listener(
                                move |this, e: &gpui::MouseMoveEvent, _, cx| {
                                    if e.pressed_button == Some(MouseButton::Left)
                                        && let Some(f) = fraction_at(&vol_drag, e.position.x)
                                    {
                                        this.set_volume(f, cx);
                                    }
                                },
                            )),
                    )
                    .child(
                        div()
                            .ml(px(6.))
                            .text_xs()
                            .text_color(gpui::white())
                            .child(format!(
                                "{} / {}",
                                format_time(self.position_ms.min(duration)),
                                format_time(duration)
                            )),
                    )
                    .child(div().flex_1())
                    .child(
                        button("video-browser", Icon::External, "Open in the browser")
                            .on_click(cx.listener(|this, _, _, cx| this.open_in_browser(cx))),
                    )
                    .child(
                        button(
                            "video-expand",
                            Icon::Expand,
                            if self.expanded {
                                "Shrink (F)"
                            } else {
                                "Fill the window (F)"
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.expanded = !this.expanded;
                            cx.notify();
                        })),
                    ),
            )
            .into_any_element()
    }
}

impl Focusable for VideoPlayer {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for VideoPlayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let viewport = window.viewport_size();
        let failed = matches!(self.stage, Stage::Failed(_));
        // The player keeps the video's shape inside most of the window.
        let (vw, vh) = (
            self.source.width.max(1) as f32,
            self.source.height.max(1) as f32,
        );
        let (max_w, max_h) = if self.expanded {
            (f32::from(viewport.width), f32::from(viewport.height))
        } else {
            (
                f32::from(viewport.width) * 0.86,
                f32::from(viewport.height) * 0.8 - 40.,
            )
        };
        let scale = (max_w / vw).min(max_h / vh);
        let (w, h) = if failed || self.source.width == 0 {
            (max_w.min(640.), max_h.min(360.))
        } else {
            (vw * scale, vh * scale)
        };
        let header = div()
            .w(px(w))
            .flex()
            .items_center()
            .gap(px(8.))
            .pb(px(8.))
            .text_color(gpui::white())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.source.filename.clone()),
            )
            .child(
                div()
                    .id("video-close")
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .cursor_pointer()
                    .hover(|s| s.bg(gpui::white().opacity(0.15)))
                    .tooltip(|_, cx| Tooltip::view("Close (Esc)", cx))
                    .child(icon(Icon::Close).size(px(18.)))
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            );
        let frame = div()
            .id("video-frame")
            .relative()
            .w(px(w))
            .h(px(h))
            .flex()
            .flex_col()
            .bg(gpui::black())
            .when(!self.expanded, |el| el.rounded(t.radius).overflow_hidden())
            .child(
                div()
                    .id("video-picture")
                    .absolute()
                    .inset_0()
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx)))
                    .child(self.picture(&t)),
            )
            .when(failed, |el| {
                el.child(
                    div()
                        .absolute()
                        .bottom(px(16.))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .id("video-fallback")
                                .px(px(14.))
                                .py(px(6.))
                                .rounded(t.radius_small)
                                .bg(t.accent)
                                .text_color(gpui::white())
                                .cursor_pointer()
                                .child("Open in the browser")
                                .on_click(cx.listener(|this, _, _, cx| this.open_in_browser(cx))),
                        ),
                )
            })
            .when(!failed, |el| el.child(self.controls(&t, cx)));
        div()
            .id("video-player")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| this.on_key(e, cx)))
            // Clicks inside the player must not reach the scrim, which closes it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .flex()
            .flex_col()
            .items_center()
            .when(!self.expanded, |el| el.child(header))
            .child(frame)
    }
}

async fn download(url: String) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let mut res = client
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    if res.content_length().is_some_and(|n| n > MAX_BYTES) {
        return Err("the file is too big to play here".into());
    }
    let mut out = Vec::with_capacity(res.content_length().unwrap_or(0) as usize);
    while let Some(chunk) = res.chunk().await.map_err(|e| e.to_string())? {
        out.extend_from_slice(&chunk);
        if out.len() as u64 > MAX_BYTES {
            return Err("the file is too big to play here".into());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_playable_files() {
        assert!(VideoSource::playable("clip.MP4", None, 10));
        assert!(VideoSource::playable("x", Some("video/quicktime"), 10));
        assert!(!VideoSource::playable("clip.webm", Some("video/webm"), 10));
        assert!(!VideoSource::playable("clip.mp4", None, MAX_BYTES + 1));
    }
}
