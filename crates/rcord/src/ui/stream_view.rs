//! Watched streams, drawn above the chat. One stream fills the stage; several share it as a
//! grid of tiles, as in Discord's call view. Each tile shows the stream's quality and who is
//! streaming, and on hover the controls: stream volume, fullscreen and stop watching.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Entity, FontWeight, ImageSource, ObjectFit, SharedString, div, img,
    prelude::*, px,
};
use rcord_model::Snowflake;

use crate::golive::{StreamStatus, Watching};
use crate::state::{AppState, Overlay};
use crate::theme::{Theme, theme};
use crate::ui::icons::{Icon, icon};
use crate::ui::volume::volume_bar;
use crate::ui::{Tooltip, snowflake_id};

/// The stage above the chat, when anything is being watched.
pub fn stream_panel(state: &Entity<AppState>, cx: &mut App) -> Option<AnyElement> {
    let t = theme(cx).clone();
    let s = state.read(cx);
    if s.watching.is_empty() || matches!(s.overlay, Some(Overlay::StreamFullscreen)) {
        return None;
    }
    let tiles: Vec<AnyElement> = s
        .watching
        .iter()
        .map(|w| stream_tile(state, s, w, false, &t))
        .collect();
    let n = tiles.len();
    // Two across from two streams on, as Discord lays out a call.
    let columns = if n == 1 { 1 } else { 2 };
    let mut grid = div().size_full().flex().flex_col().gap(px(8.)).p(px(8.));
    let mut tiles = tiles.into_iter();
    for _ in 0..n.div_ceil(columns) {
        let mut row = div().flex_1().min_h_0().flex().gap(px(8.));
        for _ in 0..columns {
            row = match tiles.next() {
                Some(tile) => row.child(div().flex_1().min_w_0().h_full().child(tile)),
                // An odd tile out keeps its column's width.
                None => row.child(div().flex_1()),
            };
        }
        grid = grid.child(row);
    }
    Some(
        div()
            .flex_none()
            .h(gpui::relative(if n > 2 { 0.7 } else { 0.55 }))
            .min_h(px(220.))
            .bg(gpui::black())
            .border_b_1()
            .border_color(t.border)
            .child(grid)
            .into_any_element(),
    )
}

/// The focused stream over the whole window.
pub fn fullscreen_stream(state: &Entity<AppState>, cx: &mut App) -> Option<AnyElement> {
    let t = theme(cx).clone();
    let s = state.read(cx);
    let w = s.focused_watch()?;
    Some(
        div()
            .size_full()
            .bg(gpui::black())
            .child(stream_tile(state, s, w, true, &t))
            .into_any_element(),
    )
}

fn stream_tile(
    state: &Entity<AppState>,
    s: &AppState,
    w: &Watching,
    fullscreen: bool,
    t: &Theme,
) -> AnyElement {
    let streamer = w.streamer;
    let guild = s.call.as_ref().and_then(|c| c.guild);
    let name: SharedString = s
        .store
        .display_name(guild, streamer)
        .map(str::to_owned)
        .or_else(|| s.store.user(streamer).map(|u| u.display_name().to_owned()))
        .unwrap_or_else(|| "Someone".into())
        .into();
    let volume = s.stream_volume(streamer);
    let viewers = w.viewers.len();
    let group: SharedString = format!("stream-tile-{streamer}").into();
    let status = match w.status {
        StreamStatus::Live if w.frame.is_none() => Some("Waiting for the picture…"),
        StreamStatus::Live => None,
        StreamStatus::Reconnecting => Some("Reconnecting…"),
        StreamStatus::Starting | StreamStatus::Connecting => Some("Connecting to the stream…"),
    };
    let picture = match w.frame.clone() {
        Some(frame) => img(ImageSource::Render(frame))
            .size_full()
            .object_fit(ObjectFit::Contain)
            .into_any_element(),
        None => div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .text_color(gpui::white().opacity(0.7))
            .child(icon(Icon::Screen).size(px(40.)))
            .child(status.unwrap_or_default())
            .into_any_element(),
    };
    let quality = (w.size.1 > 0).then(|| {
        if w.fps > 0 {
            format!("{}P {}FPS", w.size.1, w.fps)
        } else {
            format!("{}P", w.size.1)
        }
    });

    let chip = |el: gpui::Div| {
        el.px(px(8.))
            .h(px(24.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(6.))
            .bg(gpui::black().opacity(0.6))
            .text_color(gpui::white())
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
    };
    let control = |id: &'static str, which: Icon, tip: &'static str| {
        div()
            .id(snowflake_id(id, streamer))
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .cursor_pointer()
            .text_color(gpui::white())
            .hover(|s| s.bg(gpui::white().opacity(0.15)))
            .tooltip(move |_, cx| Tooltip::view(tip, cx))
            .child(icon(which).size(px(20.)))
    };

    let mute_state = state.clone();
    let volume_state = state.clone();
    let full_state = state.clone();
    let stop_state = state.clone();
    let focus_state = state.clone();
    let muted = volume <= 0.0;
    let controls = div()
        .absolute()
        .left(px(0.))
        .right(px(0.))
        .bottom(px(0.))
        .px(px(12.))
        .pb(px(10.))
        .pt(px(28.))
        .flex()
        .items_center()
        .gap(px(6.))
        .bg(gpui::linear_gradient(
            0.,
            gpui::linear_color_stop(gpui::black().opacity(0.75), 0.),
            gpui::linear_color_stop(gpui::transparent_black(), 1.),
        ))
        .invisible()
        .group_hover(group.clone(), |s| s.visible())
        .child(
            control(
                "stream-mute",
                if muted { Icon::VolumeOff } else { Icon::Volume },
                if muted {
                    "Unmute Stream"
                } else {
                    "Mute Stream"
                },
            )
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                mute_state.update(cx, |s, cx| {
                    s.set_stream_volume(streamer, if muted { 1.0 } else { 0.0 }, cx)
                })
            }),
        )
        .child(div().w(px(120.)).child(volume_bar(
            snowflake_id("stream-volume", streamer),
            volume,
            t,
            Rc::new(move |v, cx| {
                volume_state.update(cx, |s, cx| s.set_stream_volume(streamer, v, cx))
            }),
        )))
        .child(
            div()
                .w(px(40.))
                .text_xs()
                .text_color(gpui::white().opacity(0.8))
                .child(format!("{:.0}%", volume * 100.)),
        )
        .child(div().flex_1())
        .child(
            control(
                "stream-fullscreen",
                if fullscreen {
                    Icon::Restore
                } else {
                    Icon::Expand
                },
                if fullscreen {
                    "Exit Full Screen"
                } else {
                    "Full Screen"
                },
            )
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                full_state.update(cx, |s, cx| {
                    if fullscreen {
                        s.set_overlay(None, cx);
                    } else {
                        s.focused_stream = Some(streamer);
                        s.set_overlay(Some(Overlay::StreamFullscreen), cx);
                    }
                })
            }),
        )
        .child(
            control("stream-stop", Icon::Close, "Stop Watching").on_click(move |_, _, cx| {
                cx.stop_propagation();
                stop_state.update(cx, |s, cx| s.stop_watching(streamer, cx))
            }),
        );

    div()
        .id(snowflake_id("stream-tile", streamer))
        .group(group)
        .relative()
        .size_full()
        .overflow_hidden()
        .when(!fullscreen, |el| el.rounded(px(8.)))
        .bg(gpui::black())
        .child(picture)
        .child(
            div()
                .absolute()
                .top(px(10.))
                .right(px(10.))
                .flex()
                .items_center()
                .gap(px(6.))
                .when_some(quality, |el, q| {
                    el.child(chip(div()).child(icon(Icon::Screen).size(px(14.))).child(q))
                })
                .child(
                    div()
                        .px(px(6.))
                        .h(px(24.))
                        .flex()
                        .items_center()
                        .rounded(px(6.))
                        .bg(t.danger)
                        .text_color(gpui::white())
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .child("LIVE"),
                ),
        )
        .child(
            div()
                .absolute()
                .top(px(10.))
                .left(px(10.))
                .flex()
                .gap(px(6.))
                .child(
                    chip(div())
                        .child(icon(Icon::Screen).size(px(14.)))
                        .child(name),
                )
                .when(viewers > 0, |el| {
                    el.child(
                        chip(div())
                            .child(icon(Icon::Users).size(px(14.)))
                            .child(viewers.to_string()),
                    )
                }),
        )
        .child(controls)
        // A double click toggles full screen, as on a video.
        .on_click(move |e, _, cx| {
            if e.click_count() == 2 {
                focus_state.update(cx, |s, cx| {
                    if fullscreen {
                        s.set_overlay(None, cx);
                    } else {
                        s.focused_stream = Some(streamer);
                        s.set_overlay(Some(Overlay::StreamFullscreen), cx);
                    }
                })
            }
        })
        .into_any_element()
}

/// The card a LIVE badge opens: the stream's latest thumbnail and a button to watch it.
pub fn stream_preview_card(
    state: &Entity<AppState>,
    channel: Snowflake,
    streamer: Snowflake,
    cx: &mut App,
) -> AnyElement {
    let t = theme(cx).clone();
    let s = state.read(cx);
    let guild = s.store.channel(channel).and_then(|c| c.guild_id);
    let key = crate::golive::stream_key(guild, channel, streamer);
    let preview = s.stream_previews.get(&key).map(|(url, _)| url.clone());
    let watching = s.is_watching(streamer);
    let me = s.store.me_id() == Some(streamer);
    let state = state.clone();

    let picture = match preview {
        Some(Some(url)) => crate::ui::anim_img(url)
            .size_full()
            .object_fit(ObjectFit::Cover)
            .with_fallback(|| div().size_full().bg(gpui::black()).into_any_element())
            .into_any_element(),
        Some(None) => div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .text_color(gpui::white().opacity(0.7))
            .text_sm()
            .child(icon(Icon::Screen).size(px(28.)))
            .child("No preview yet")
            .into_any_element(),
        None => div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(gpui::white().opacity(0.6))
            .text_sm()
            .child("Loading preview…")
            .into_any_element(),
    };
    div()
        .id("stream-preview")
        .occlude()
        .w(px(300.))
        .p(px(12.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .rounded(t.radius)
        .bg(t.floating)
        .border_1()
        .border_color(t.border)
        .shadow_lg()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text_muted)
                        .child("Streaming Now"),
                )
                .child(
                    div()
                        .px(px(6.))
                        .rounded(px(10.))
                        .bg(t.danger)
                        .text_color(gpui::white())
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .child("LIVE"),
                ),
        )
        .child(
            div()
                .w_full()
                .h(px(156.))
                .rounded(px(6.))
                .overflow_hidden()
                .bg(gpui::black())
                .child(picture),
        )
        .when(!me, |el| {
            el.child(
                div()
                    .id("stream-preview-watch")
                    .h(px(38.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .rounded(t.radius_small)
                    .cursor_pointer()
                    .text_color(gpui::white())
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(watching, |el| {
                        el.bg(t.base).hover(|s| s.bg(t.hover)).text_color(t.text)
                    })
                    .when(!watching, |el| el.bg(t.online).hover(|s| s.opacity(0.9)))
                    .child(icon(Icon::Screen).size(px(18.)))
                    .child(if watching {
                        "Stop Watching"
                    } else {
                        "Watch Stream"
                    })
                    .on_click(move |_, _, cx| {
                        state.update(cx, |s, cx| {
                            s.set_overlay(None, cx);
                            if watching {
                                s.stop_watching(streamer, cx);
                            } else {
                                s.watch_stream(channel, streamer, cx);
                            }
                        })
                    }),
            )
        })
        .into_any_element()
}
