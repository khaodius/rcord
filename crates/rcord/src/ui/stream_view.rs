//! The stream being watched, drawn above the chat: the streamer's picture scaled to fit,
//! with who is streaming, how many watch, and a control to stop watching.

use gpui::{AnyElement, App, Entity, FontWeight, ImageSource, ObjectFit, div, img, prelude::*, px};

use crate::golive::StreamStatus;
use crate::state::AppState;
use crate::theme::theme;
use crate::ui::icons::{Icon, icon};
use crate::ui::{Tooltip, icon_button};

pub fn stream_panel(state: &Entity<AppState>, cx: &mut App) -> Option<AnyElement> {
    let t = theme(cx).clone();
    let s = state.read(cx);
    let w = s.watching.as_ref()?;
    let guild = s.call.as_ref().and_then(|c| c.guild);
    let name = s
        .store
        .display_name(guild, w.streamer)
        .map(str::to_owned)
        .or_else(|| s.store.user(w.streamer).map(|u| u.username.clone()))
        .unwrap_or_else(|| "Someone".into());
    let viewers = w.viewers.len();
    let status = match w.status {
        StreamStatus::Live if w.frame.is_none() => Some("Waiting for the picture…"),
        StreamStatus::Live => None,
        StreamStatus::Reconnecting => Some("Reconnecting…"),
        StreamStatus::Starting | StreamStatus::Connecting => Some("Connecting to the stream…"),
    };
    let frame = w.frame.clone();
    let stop = state.clone();

    let picture = match frame {
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

    Some(
        div()
            .flex_none()
            .h(gpui::relative(0.55))
            .min_h(px(200.))
            .flex()
            .flex_col()
            .bg(gpui::black())
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .flex_none()
                    .h(px(40.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .bg(t.base)
                    .child(
                        div()
                            .px(px(5.))
                            .rounded(px(4.))
                            .bg(t.danger)
                            .text_color(gpui::white())
                            .text_size(px(10.))
                            .font_weight(FontWeight::BOLD)
                            .child("LIVE"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.header)
                            .child(name),
                    )
                    .when(viewers > 0, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .text_xs()
                                .text_color(t.text_muted)
                                .child(icon(Icon::Users).size(px(14.)))
                                .child(viewers.to_string()),
                        )
                    })
                    .child(
                        icon_button("stream-stop-watching", Icon::Close, &t)
                            .tooltip(|_, cx| Tooltip::view("Stop Watching", cx))
                            .on_click(move |_, _, cx| stop.update(cx, |s, cx| s.stop_watching(cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(picture))
            .into_any_element(),
    )
}
