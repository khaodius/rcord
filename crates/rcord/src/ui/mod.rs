//! Views and the small components they share.

pub mod anim;
pub mod channels;
pub mod chat;
pub mod context_menus;
pub mod friends;
pub mod icons;
pub mod login;
pub mod markup;
pub mod members;
pub mod overlays;
pub mod panel_window;
pub mod profile;
pub mod servers;
pub mod settings;
pub mod stream_view;
pub mod text_input;
pub mod title_bar;
pub mod user_menu;
pub mod video;
pub mod volume;
pub mod workspace;

use gpui::{
    App, Context, Div, ElementId, FontWeight, Hsla, IntoElement, ObjectFit, Pixels, Render,
    SharedString, Stateful, Window, div, prelude::*, px,
};
use rcord_model::{Snowflake, Status, User, cdn};
use rcord_settings::Animate;

use crate::theme::{Theme, theme};
use icons::{Icon, icon};

pub use anim::anim_img;

/// A round avatar with an optional status dot cut into its corner.
pub fn avatar(
    url: impl Into<SharedString>,
    size: Pixels,
    status: Option<(Status, &Theme, Hsla)>,
) -> Div {
    let url: SharedString = url.into();
    let color = placeholder_color(&url);
    let picture = anim_img(url)
        .size(size)
        .rounded_full()
        .object_fit(ObjectFit::Cover)
        .with_fallback(move || div().size(size).rounded_full().bg(color).into_any_element());
    let mut root = div()
        .relative()
        .flex_none()
        .size(size)
        .rounded_full()
        .bg(color)
        .child(picture);
    if let Some((status, t, ring)) = status {
        let dot = (size * 0.3).max(px(8.));
        let ring_width = (size * 0.06).max(px(2.));
        root = root.child(
            div()
                .absolute()
                .right(-ring_width)
                .bottom(-ring_width)
                .size(dot + ring_width * 2.)
                .rounded_full()
                .bg(ring)
                .flex()
                .items_center()
                .justify_center()
                .child(div().size(dot).rounded_full().bg(t.status(status))),
        );
    }
    root
}

/// A stable color for an image that could not load, from Discord's default avatar palette.
fn placeholder_color(url: &str) -> Hsla {
    const PALETTE: [u32; 6] = [0x5865F2, 0x757E8A, 0x3BA55C, 0xFAA61A, 0xED4245, 0xEB459E];
    let index = url
        .rsplit_once("/embed/avatars/")
        .and_then(|(_, rest)| rest.split('.').next()?.parse::<usize>().ok())
        .unwrap_or_else(|| {
            url.bytes()
                .fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize))
        });
    gpui::rgb(PALETTE[index % PALETTE.len()]).into()
}

/// Whether an animated asset should animate given a setting and hover state.
pub fn should_animate(setting: Animate, hovered: bool) -> bool {
    match setting {
        Animate::Always => true,
        Animate::OnHover => hovered,
        Animate::Never => false,
    }
}

pub fn user_avatar_url(user: &User, px_size: f32, animate: bool) -> String {
    cdn::user_avatar(user, (px_size * 2.0) as u32, animate)
}

pub fn icon_button(id: impl Into<ElementId>, which: Icon, t: &Theme) -> Stateful<Div> {
    div()
        .id(id.into())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(32.))
        .rounded(t.radius_small)
        .text_color(t.text_muted)
        .cursor_pointer()
        .hover(|s| s.bg(t.hover).text_color(t.text))
        .child(icon(which).size(px(20.)))
}

pub struct Tooltip {
    text: SharedString,
}

impl Tooltip {
    pub fn view(text: impl Into<SharedString>, cx: &mut App) -> gpui::AnyView {
        cx.new(|_| Self { text: text.into() }).into()
    }
}

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        div()
            .px_2p5()
            .py_1p5()
            .rounded(t.radius_small)
            .bg(t.floating)
            .border_1()
            .border_color(t.border)
            .shadow_md()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.header)
            .child(self.text.clone())
    }
}

pub fn section_label(text: impl Into<SharedString>, t: &Theme) -> Div {
    div()
        .text_xs()
        .font_weight(FontWeight::BOLD)
        .text_color(t.text_faint)
        .child(text.into().to_uppercase())
}

/// The red pill with a mention count.
pub fn mention_badge(count: u32, t: &Theme) -> Div {
    div()
        .flex_none()
        .min_w(px(16.))
        .h(px(16.))
        .px_1()
        .rounded_full()
        .bg(t.danger)
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .font_weight(FontWeight::BOLD)
        .text_color(gpui::white())
        .child(if count > 99 {
            "99+".to_string()
        } else {
            count.to_string()
        })
}

pub fn snowflake_id(prefix: &'static str, id: Snowflake) -> ElementId {
    ElementId::NamedInteger(prefix.into(), id.get())
}
