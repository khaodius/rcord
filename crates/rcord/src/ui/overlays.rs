//! Everything drawn above the main window: context menus, the profile card, the expression
//! picker, the quick switcher and toasts.

use gpui::{
    AnyElement, Context, Entity, Focusable, FontWeight, IntoElement, MouseButton, ObjectFit,
    Render, Subscription, Window, anchored, deferred, div, point, prelude::*, px,
};
use rcord_model::{Emoji, Snowflake, StickerFormat, StickerItem, cdn};

use crate::golive::StreamPreset;
use crate::state::{AppState, ExpressionTab, Overlay, StateEvent};
use crate::theme::theme;
use crate::ui::anim_img;
use crate::ui::chat::ChatView;
use crate::ui::icons::{Icon, icon};
use crate::ui::profile::profile_card;
use crate::ui::text_input::{InputEvent, TextInput};
use crate::ui::video::VideoPlayer;
use crate::ui::{Tooltip, section_label};

/// Common Unicode emoji with the names the picker searches by.
const UNICODE_EMOJI: &[(&str, &str)] = &[
    ("😀", "grinning"),
    ("😃", "smiley"),
    ("😄", "smile"),
    ("😁", "grin"),
    ("😆", "laughing"),
    ("😅", "sweat_smile"),
    ("🤣", "rofl"),
    ("😂", "joy"),
    ("🙂", "slight_smile"),
    ("🙃", "upside_down"),
    ("😉", "wink"),
    ("😊", "blush"),
    ("😇", "innocent"),
    ("🥰", "smiling_face_with_hearts"),
    ("😍", "heart_eyes"),
    ("🤩", "star_struck"),
    ("😘", "kissing_heart"),
    ("😋", "yum"),
    ("😛", "stuck_out_tongue"),
    ("😜", "stuck_out_tongue_winking_eye"),
    ("🤪", "zany_face"),
    ("🤔", "thinking"),
    ("🤨", "raised_eyebrow"),
    ("😐", "neutral_face"),
    ("😑", "expressionless"),
    ("😶", "no_mouth"),
    ("🙄", "rolling_eyes"),
    ("😏", "smirk"),
    ("😬", "grimacing"),
    ("😌", "relieved"),
    ("😔", "pensive"),
    ("😪", "sleepy"),
    ("😴", "sleeping"),
    ("😷", "mask"),
    ("🤒", "thermometer_face"),
    ("🥵", "hot_face"),
    ("🥶", "cold_face"),
    ("😵", "dizzy_face"),
    ("🤯", "exploding_head"),
    ("🤠", "cowboy"),
    ("🥳", "partying_face"),
    ("😎", "sunglasses"),
    ("🤓", "nerd"),
    ("😕", "confused"),
    ("😟", "worried"),
    ("🙁", "slight_frown"),
    ("😮", "open_mouth"),
    ("😲", "astonished"),
    ("😳", "flushed"),
    ("🥺", "pleading_face"),
    ("😦", "frowning"),
    ("😨", "fearful"),
    ("😰", "cold_sweat"),
    ("😢", "cry"),
    ("😭", "sob"),
    ("😱", "scream"),
    ("😤", "triumph"),
    ("😡", "rage"),
    ("😠", "angry"),
    ("🤬", "face_with_symbols_over_mouth"),
    ("💀", "skull"),
    ("☠️", "skull_and_crossbones"),
    ("💩", "poop"),
    ("🤡", "clown"),
    ("👻", "ghost"),
    ("👽", "alien"),
    ("🤖", "robot"),
    ("😺", "smiley_cat"),
    ("🙈", "see_no_evil"),
    ("🙉", "hear_no_evil"),
    ("👋", "wave"),
    ("🤚", "raised_back_of_hand"),
    ("✋", "raised_hand"),
    ("👌", "ok_hand"),
    ("🤌", "pinched_fingers"),
    ("✌️", "v"),
    ("🤞", "fingers_crossed"),
    ("🤟", "love_you_gesture"),
    ("🤘", "metal"),
    ("👈", "point_left"),
    ("👉", "point_right"),
    ("👆", "point_up_2"),
    ("👇", "point_down"),
    ("👍", "thumbsup"),
    ("👎", "thumbsdown"),
    ("✊", "fist"),
    ("👊", "punch"),
    ("👏", "clap"),
    ("🙌", "raised_hands"),
    ("🙏", "pray"),
    ("💪", "muscle"),
    ("🫡", "saluting_face"),
    ("🫠", "melting_face"),
    ("🫶", "heart_hands"),
    ("👀", "eyes"),
    ("❤️", "heart"),
    ("🧡", "orange_heart"),
    ("💛", "yellow_heart"),
    ("💚", "green_heart"),
    ("💙", "blue_heart"),
    ("💜", "purple_heart"),
    ("🖤", "black_heart"),
    ("🤍", "white_heart"),
    ("💔", "broken_heart"),
    ("💯", "100"),
    ("🔥", "fire"),
    ("✨", "sparkles"),
    ("⭐", "star"),
    ("🌟", "star2"),
    ("💥", "boom"),
    ("🎉", "tada"),
    ("🎊", "confetti_ball"),
    ("🎁", "gift"),
    ("🏆", "trophy"),
    ("🥇", "first_place"),
    ("✅", "white_check_mark"),
    ("❌", "x"),
    ("⚠️", "warning"),
    ("❓", "question"),
    ("❗", "exclamation"),
    ("💤", "zzz"),
    ("💬", "speech_balloon"),
    ("🗿", "moyai"),
    ("🚀", "rocket"),
    ("🍕", "pizza"),
    ("🍔", "hamburger"),
    ("☕", "coffee"),
    ("🍺", "beer"),
    ("🎮", "video_game"),
    ("🎧", "headphones"),
    ("💻", "computer"),
    ("📌", "pushpin"),
    ("🔒", "lock"),
    ("🔑", "key"),
    ("🐱", "cat"),
    ("🐶", "dog"),
    ("🦀", "crab"),
    ("🐍", "snake"),
    ("🌈", "rainbow"),
    ("☀️", "sunny"),
];

pub struct OverlayLayer {
    state: Entity<AppState>,
    chat: Entity<ChatView>,
    search: Entity<TextInput>,
    switcher: Entity<TextInput>,
    video: Option<Entity<VideoPlayer>>,
    _subs: Vec<Subscription>,
}

impl OverlayLayer {
    pub fn new(
        state: Entity<AppState>,
        chat: Entity<ChatView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| TextInput::new("Search", cx));
        let switcher = cx.new(|cx| TextInput::new("Where would you like to go?", cx));
        let mut subs = vec![cx.subscribe(&state, |_, _, event, cx| {
            if matches!(
                event,
                StateEvent::Overlay
                    | StateEvent::Settings
                    | StateEvent::Selection
                    | StateEvent::Store(_)
            ) {
                cx.notify();
            }
        })];
        subs.push(cx.observe(&state, |_, _, cx| cx.notify()));
        subs.push(cx.subscribe_in(
            &search,
            window,
            |this, input, event, _window, cx| match event {
                InputEvent::Submit => {
                    let q = input.read(cx).text().to_owned();
                    let tab = match this.state.read(cx).overlay {
                        Some(Overlay::Expressions(tab)) => Some(tab),
                        _ => None,
                    };
                    if tab == Some(ExpressionTab::Gifs) {
                        this.state.update(cx, |s, cx| s.search_gifs(q, cx));
                    }
                }
                InputEvent::Changed => cx.notify(),
                InputEvent::Cancel => this.state.update(cx, |s, cx| s.set_overlay(None, cx)),
                InputEvent::UpInEmpty => {}
            },
        ));
        subs.push(cx.subscribe_in(
            &switcher,
            window,
            |this, input, event, _window, cx| match event {
                InputEvent::Submit => {
                    let q = input.read(cx).text().to_owned();
                    if let Some(first) = this.switcher_results(&q, cx).first().map(|r| r.0) {
                        input.update(cx, |i, cx| i.clear(cx));
                        this.state.update(cx, |s, cx| {
                            s.set_overlay(None, cx);
                            s.select_channel(first, cx);
                        });
                    }
                }
                InputEvent::Changed => cx.notify(),
                InputEvent::Cancel => this.state.update(cx, |s, cx| s.set_overlay(None, cx)),
                InputEvent::UpInEmpty => {}
            },
        ));
        Self {
            state,
            chat,
            search,
            switcher,
            video: None,
            _subs: subs,
        }
    }

    pub fn focus_for(&self, overlay: &Overlay, window: &mut Window, cx: &mut gpui::App) {
        match overlay {
            Overlay::Expressions(_) => window.focus(&self.search.focus_handle(cx)),
            Overlay::QuickSwitcher => window.focus(&self.switcher.focus_handle(cx)),
            _ => {}
        }
    }

    /// Opens or closes the video player to match the overlay. Closing drops the player,
    /// which stops its thread and its sound.
    pub fn sync_video(
        &mut self,
        overlay: Option<&Overlay>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match overlay {
            Some(Overlay::Video(source)) => {
                if self
                    .video
                    .as_ref()
                    .is_none_or(|v| v.read(cx).source != *source)
                {
                    let state = self.state.clone();
                    let source = source.clone();
                    self.video = Some(cx.new(|cx| VideoPlayer::new(state, source, window, cx)));
                }
                if let Some(v) = &self.video {
                    window.focus(&v.focus_handle(cx));
                }
            }
            _ => self.video = None,
        }
    }

    fn close(&self, cx: &mut gpui::App) {
        self.state.update(cx, |s, cx| s.set_overlay(None, cx));
    }

    /// Channels and DMs whose name contains the query, unread ones first.
    fn switcher_results(
        &self,
        query: &str,
        cx: &gpui::App,
    ) -> Vec<(Snowflake, String, Option<String>)> {
        let s = self.state.read(cx);
        let q = query.trim().trim_start_matches(['#', '@']).to_lowercase();
        let mut out: Vec<(bool, Snowflake, String, Option<String>)> = Vec::new();
        for c in s.store.private_channels() {
            let name = s.store.channel_name(c.id);
            if q.is_empty() || name.to_lowercase().contains(&q) {
                out.push((s.store.is_unread(c.id), c.id, name, None));
            }
        }
        for g in s.store.guilds() {
            for id in &g.channel_ids {
                let Some(c) = s.store.channel(*id) else {
                    continue;
                };
                if !c.kind.has_messages() || c.kind.is_voice() || !s.store.can_view(*id) {
                    continue;
                }
                let name = c.name.clone().unwrap_or_default();
                if q.is_empty() || name.to_lowercase().contains(&q) {
                    out.push((
                        s.store.is_unread(*id),
                        *id,
                        name,
                        Some(g.guild.name.clone()),
                    ));
                }
            }
        }
        out.sort_by_key(|(unread, id, ..)| {
            (
                !*unread,
                std::cmp::Reverse(s.store.channel(*id).and_then(|c| c.last_message_id)),
            )
        });
        out.into_iter()
            .take(12)
            .map(|(_, id, n, g)| (id, n, g))
            .collect()
    }

    fn menu(
        &self,
        at: gpui::Point<gpui::Pixels>,
        items: Vec<crate::state::MenuItem>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = theme(cx).clone();
        let state = self.state.clone();
        let mut col = div()
            .id("menu")
            .occlude()
            .min_w(px(188.))
            .p(px(6.))
            .flex()
            .flex_col()
            .rounded(t.radius_small)
            .bg(t.floating)
            .border_1()
            .border_color(t.border)
            .shadow_lg();
        for (i, item) in items.into_iter().enumerate() {
            let action = item.action.clone();
            let state = state.clone();
            col = col.child(
                div()
                    .id(("menu-item", i))
                    .px(px(8.))
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .text_color(if item.danger { t.danger } else { t.text })
                    .hover(|s| {
                        s.bg(if item.danger { t.danger } else { t.accent })
                            .text_color(gpui::white())
                    })
                    .child(item.label)
                    .on_click(move |_, _, cx| {
                        state.update(cx, |s, cx| s.set_overlay(None, cx));
                        action(cx);
                    }),
            );
        }
        deferred(
            anchored()
                .position(at)
                .snap_to_window_with_margin(px(8.))
                .child(col),
        )
        .with_priority(2)
        .into_any_element()
    }

    fn expressions(&self, tab: ExpressionTab, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let query = self.search.read(cx).text().to_lowercase();
        let s = self.state.read(cx);
        let channel = s.channel;
        let react_to = s.react_to;
        let mut grid = div()
            .id("expr-grid")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(8.));
        match tab {
            ExpressionTab::Emoji => {
                let custom: Vec<(String, Emoji)> = s
                    .store
                    .emojis()
                    .filter(|(_, e)| {
                        query.is_empty()
                            || e.name
                                .as_deref()
                                .is_some_and(|n| n.to_lowercase().contains(&query))
                    })
                    .map(|(g, e)| (g.name.clone(), e.clone()))
                    .take(400)
                    .collect();
                let mut current_guild = String::new();
                let mut row = div().flex().flex_row().flex_wrap().gap(px(2.));
                for (guild, e) in custom {
                    if guild != current_guild {
                        grid = grid.child(std::mem::replace(
                            &mut row,
                            div().flex().flex_row().flex_wrap().gap(px(2.)),
                        ));
                        grid = grid.child(section_label(guild.clone(), &t).mt(px(4.)));
                        current_guild = guild;
                    }
                    let url = cdn::emoji(&e, 64).unwrap_or_default();
                    let name = e.name.clone().unwrap_or_default();
                    let state = self.state.clone();
                    let chat = self.chat.clone();
                    let emoji = e.clone();
                    row = row.child(
                        div()
                            .id(gpui::ElementId::NamedInteger(
                                "emoji".into(),
                                e.id.map_or(0, |i| i.get()),
                            ))
                            .size(px(40.))
                            .p(px(4.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover))
                            .tooltip(move |_, cx| Tooltip::view(format!(":{name}:"), cx))
                            .child(anim_img(url).size_full().object_fit(ObjectFit::Contain))
                            .on_click(move |_, window, cx| {
                                pick_emoji(&state, &chat, &emoji, react_to, window, cx)
                            }),
                    );
                }
                grid = grid.child(row);
                grid = grid.child(section_label("Emoji", &t).mt(px(4.)));
                let mut row = div().flex().flex_row().flex_wrap().gap(px(2.));
                for (ch, name) in UNICODE_EMOJI
                    .iter()
                    .filter(|(_, n)| query.is_empty() || n.contains(&query))
                {
                    let state = self.state.clone();
                    let chat = self.chat.clone();
                    let emoji = Emoji {
                        id: None,
                        name: Some((*ch).to_owned()),
                        animated: false,
                        roles: Vec::new(),
                        available: true,
                        managed: false,
                    };
                    let tip = format!(":{name}:");
                    row = row.child(
                        div()
                            .id(gpui::ElementId::Name(format!("u-{name}").into()))
                            .size(px(40.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(26.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover))
                            .tooltip(move |_, cx| Tooltip::view(tip.clone(), cx))
                            .child(*ch)
                            .on_click(move |_, window, cx| {
                                pick_emoji(&state, &chat, &emoji, react_to, window, cx)
                            }),
                    );
                }
                grid = grid.child(row);
            }
            ExpressionTab::Gifs => {
                let gifs = s.gifs.clone();
                if gifs.loading {
                    grid = grid.child(
                        div()
                            .p(px(16.))
                            .text_color(t.text_muted)
                            .child("Loading GIFs…"),
                    );
                }
                let mut cols = [
                    div().flex_1().flex().flex_col().gap(px(6.)),
                    div().flex_1().flex().flex_col().gap(px(6.)),
                ];
                let mut heights = [0f32; 2];
                for (i, gif) in gifs.results.iter().enumerate() {
                    let w = 196f32;
                    let h = if gif.width > 0 {
                        w * gif.height as f32 / gif.width as f32
                    } else {
                        120.
                    };
                    let col = if heights[0] <= heights[1] { 0 } else { 1 };
                    heights[col] += h;
                    let url = gif.url.clone();
                    let state = self.state.clone();
                    let src = gif.gif_src.clone().unwrap_or_else(|| gif.src.clone());
                    let el = div()
                        .id(("gif", i))
                        .w_full()
                        .h(px(h))
                        .rounded(t.radius_small)
                        .overflow_hidden()
                        .bg(t.hover)
                        .cursor_pointer()
                        .child(anim_img(src).size_full().object_fit(ObjectFit::Cover))
                        .on_click(move |_, _, cx| {
                            if let Some(c) = channel {
                                state.update(cx, |s, cx| {
                                    s.set_overlay(None, cx);
                                    s.send_message(c, url.clone(), cx);
                                });
                            }
                        });
                    let taken = std::mem::replace(&mut cols[col], div());
                    cols[col] = taken.child(el);
                }
                if gifs.results.is_empty() && !gifs.loading {
                    let mut cats = div().flex().flex_row().flex_wrap().gap(px(6.));
                    for (i, c) in gifs.categories.iter().enumerate() {
                        let name = c.name.clone();
                        let state = self.state.clone();
                        let search = self.search.clone();
                        cats = cats.child(
                            div()
                                .id(("gif-cat", i))
                                .relative()
                                .w(px(196.))
                                .h(px(100.))
                                .rounded(t.radius_small)
                                .overflow_hidden()
                                .cursor_pointer()
                                .child(
                                    anim_img(c.src.clone())
                                        .size_full()
                                        .object_fit(ObjectFit::Cover),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .bg(t.scrim)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(gpui::white())
                                        .child(c.name.clone()),
                                )
                                .on_click(move |_, _, cx| {
                                    search.update(cx, |i, cx| i.set_text(name.clone(), cx));
                                    state.update(cx, |s, cx| s.search_gifs(name.clone(), cx));
                                }),
                        );
                    }
                    grid = grid.child(cats);
                } else {
                    let [a, b] = cols;
                    grid = grid.child(div().flex().flex_row().gap(px(6.)).child(a).child(b));
                }
            }
            ExpressionTab::Stickers => {
                let mut sections: Vec<(String, Vec<StickerItem>)> = s
                    .store
                    .guilds()
                    .filter(|g| !g.guild.stickers.is_empty())
                    .map(|g| {
                        (
                            g.guild.name.clone(),
                            g.guild
                                .stickers
                                .iter()
                                .filter(|st| st.available)
                                .map(|st| StickerItem {
                                    id: st.id,
                                    name: st.name.clone(),
                                    format_type: st.format_type,
                                })
                                .collect(),
                        )
                    })
                    .collect();
                if let Some(packs) = &s.sticker_packs {
                    for p in packs {
                        sections.push((
                            p.name.clone(),
                            p.stickers
                                .iter()
                                .map(|st| StickerItem {
                                    id: st.id,
                                    name: st.name.clone(),
                                    format_type: st.format_type,
                                })
                                .collect(),
                        ));
                    }
                }
                for (title, items) in sections {
                    let items: Vec<_> = items
                        .into_iter()
                        .filter(|i| query.is_empty() || i.name.to_lowercase().contains(&query))
                        .collect();
                    if items.is_empty() {
                        continue;
                    }
                    grid = grid.child(section_label(title, &t).mt(px(4.)));
                    let mut row = div().flex().flex_row().flex_wrap().gap(px(4.));
                    for st in items {
                        let state = self.state.clone();
                        let id = st.id;
                        let name = st.name.clone();
                        let preview = match cdn::sticker(&st, 160) {
                            Some(url) => anim_img(url)
                                .size_full()
                                .object_fit(ObjectFit::Contain)
                                .into_any_element(),
                            None => div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_xs()
                                .text_color(t.text_muted)
                                .child(if st.format_type == StickerFormat::Lottie {
                                    name.clone()
                                } else {
                                    String::new()
                                })
                                .into_any_element(),
                        };
                        row = row.child(
                            div()
                                .id(gpui::ElementId::NamedInteger("sticker".into(), id.get()))
                                .size(px(96.))
                                .p(px(6.))
                                .rounded(t.radius_small)
                                .cursor_pointer()
                                .hover(|s| s.bg(t.hover))
                                .tooltip(move |_, cx| Tooltip::view(name.clone(), cx))
                                .child(preview)
                                .on_click(move |_, _, cx| {
                                    if let Some(c) = channel {
                                        state.update(cx, |s, cx| {
                                            s.set_overlay(None, cx);
                                            s.send_sticker(c, id, cx);
                                        });
                                    }
                                }),
                        );
                    }
                    grid = grid.child(row);
                }
            }
        }

        let tab_button = |label: &'static str, which: ExpressionTab, state: Entity<AppState>| {
            let active = which == tab;
            div()
                .id(label)
                .px(px(10.))
                .h(px(28.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .cursor_pointer()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if active { t.header } else { t.text_muted })
                .when(active, |el| el.bg(t.selected))
                .hover(|s| s.text_color(t.text))
                .child(label)
                .on_click(move |_, _, cx| {
                    state.update(cx, |s, cx| {
                        s.set_overlay(Some(Overlay::Expressions(which)), cx)
                    })
                })
        };

        let panel = div()
            .id("expressions")
            .occlude()
            .w(px(440.))
            .h(px(460.))
            .flex()
            .flex_col()
            .rounded(t.radius)
            .bg(t.floating)
            .border_1()
            .border_color(t.border)
            .shadow_lg()
            .child(
                div()
                    .flex_none()
                    .p(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .gap(px(4.))
                            .child(tab_button("GIFs", ExpressionTab::Gifs, self.state.clone()))
                            .child(tab_button(
                                "Stickers",
                                ExpressionTab::Stickers,
                                self.state.clone(),
                            ))
                            .child(tab_button(
                                "Emoji",
                                ExpressionTab::Emoji,
                                self.state.clone(),
                            )),
                    )
                    .child(
                        div()
                            .h(px(34.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .rounded(t.radius_small)
                            .bg(t.base)
                            .text_sm()
                            .text_color(t.text)
                            .child(self.search.clone())
                            .child(icon(Icon::Search).size(px(16.)).text_color(t.text_faint)),
                    ),
            )
            .child(grid);
        div()
            .absolute()
            .right(px(24.))
            .bottom(px(84.))
            .child(panel)
            .into_any_element()
    }

    fn switcher(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let query = self.switcher.read(cx).text().to_owned();
        let results = self.switcher_results(&query, cx);
        let mut list = div().flex().flex_col().gap(px(2.)).mt(px(8.));
        for (i, (id, name, guild)) in results.into_iter().enumerate() {
            let state = self.state.clone();
            list = list.child(
                div()
                    .id(("switch", i))
                    .h(px(36.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(t.radius_small)
                    .cursor_pointer()
                    .when(i == 0, |el| el.bg(t.selected))
                    .hover(|s| s.bg(t.hover))
                    .child(
                        icon(if guild.is_some() {
                            Icon::Hash
                        } else {
                            Icon::At
                        })
                        .size(px(18.))
                        .text_color(t.text_faint),
                    )
                    .child(div().flex_1().truncate().text_color(t.text).child(name))
                    .when_some(guild, |el, g| {
                        el.child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(t.text_faint)
                                .child(g.to_uppercase()),
                        )
                    })
                    .on_click(move |_, _, cx| {
                        state.update(cx, |s, cx| {
                            s.set_overlay(None, cx);
                            s.select_channel(id, cx);
                        })
                    }),
            );
        }
        div()
            .id("switcher")
            .occlude()
            .w(px(560.))
            .p(px(16.))
            .rounded(t.radius)
            .bg(t.floating)
            .border_1()
            .border_color(t.border)
            .shadow_lg()
            .child(
                div()
                    .h(px(52.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .rounded(t.radius_small)
                    .bg(t.base)
                    .text_size(px(18.))
                    .text_color(t.text)
                    .child(self.switcher.clone()),
            )
            .child(list)
            .child(
                div()
                    .mt(px(10.))
                    .text_xs()
                    .text_color(t.text_faint)
                    .child("Type # for channels or @ for people. Enter opens the first result."),
            )
            .into_any_element()
    }

    /// Discord-style Go Live dialog: what to share, then the stream quality.
    fn go_live_picker(
        &self,
        sources: Vec<rcord_capture::Source>,
        selected: usize,
        preset: StreamPreset,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = theme(cx).clone();
        let wayland = rcord_capture::is_wayland();
        let mut list = div().flex().flex_col().gap(px(4.));
        for (i, src) in sources.iter().enumerate() {
            let state = self.state.clone();
            let all = sources.clone();
            let on = i == selected;
            list = list.child(
                div()
                    .id(("golive-source", i))
                    .px(px(12.))
                    .py(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .rounded(t.radius_small)
                    .border_1()
                    .border_color(if on { t.accent } else { t.border })
                    .when(on, |el| el.bg(t.selected))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .child(icon(Icon::Screen).size(px(22.)).text_color(if on {
                        t.text
                    } else {
                        t.text_muted
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_color(t.text)
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(src.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(t.text_faint)
                                    .child(src.detail.clone()),
                            ),
                    )
                    .on_click(move |_, _, cx| {
                        let sources = all.clone();
                        state.update(cx, |s, cx| {
                            s.set_overlay(
                                Some(Overlay::GoLive {
                                    sources,
                                    selected: i,
                                    preset,
                                }),
                                cx,
                            )
                        })
                    }),
            );
        }
        let mut presets = div().flex().gap(px(6.));
        for p in StreamPreset::ALL {
            let state = self.state.clone();
            let all = sources.clone();
            let on = p == preset;
            presets = presets.child(
                div()
                    .id(p.label())
                    .flex_1()
                    .py(px(8.))
                    .flex()
                    .justify_center()
                    .rounded(t.radius_small)
                    .text_sm()
                    .cursor_pointer()
                    .when(on, |el| el.bg(t.accent).text_color(t.accent_text))
                    .when(!on, |el| {
                        el.bg(t.base)
                            .text_color(t.text_muted)
                            .hover(|s| s.bg(t.hover).text_color(t.text))
                    })
                    .child(p.label())
                    .on_click(move |_, _, cx| {
                        let sources = all.clone();
                        state.update(cx, |s, cx| {
                            s.set_overlay(
                                Some(Overlay::GoLive {
                                    sources,
                                    selected,
                                    preset: p,
                                }),
                                cx,
                            )
                        })
                    }),
            );
        }
        let source = sources.get(selected).map(|s| s.id.clone());
        let state = self.state.clone();
        let cancel = self.state.clone();
        div()
            .id("golive")
            .occlude()
            .w(px(480.))
            .p(px(20.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .rounded(t.radius)
            .bg(t.floating)
            .border_1()
            .border_color(t.border)
            .shadow_lg()
            .child(
                div()
                    .text_size(px(20.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(t.header)
                    .child("Screen Share"),
            )
            .child(section_label(
                if wayland { "Source" } else { "Screens" },
                &t,
            ))
            .child(list)
            .when(wayland, |el| {
                el.child(div().text_xs().text_color(t.text_faint).child(
                    "Your desktop will ask which screen or window to share. Capture runs \
                     natively on Wayland through the desktop portal and PipeWire.",
                ))
            })
            .child(section_label("Stream quality", &t))
            .child(presets)
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("golive-cancel")
                            .px(px(16.))
                            .py(px(8.))
                            .rounded(t.radius_small)
                            .cursor_pointer()
                            .text_color(t.text)
                            .hover(|s| s.bg(t.hover))
                            .child("Cancel")
                            .on_click(move |_, _, cx| {
                                cancel.update(cx, |s, cx| s.set_overlay(None, cx))
                            }),
                    )
                    .child(
                        div()
                            .id("golive-start")
                            .px(px(16.))
                            .py(px(8.))
                            .rounded(t.radius_small)
                            .cursor_pointer()
                            .bg(t.accent)
                            .hover(|s| s.bg(t.accent_hover))
                            .text_color(t.accent_text)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Go Live")
                            .on_click(move |_, _, cx| {
                                let Some(source) = source.clone() else {
                                    return;
                                };
                                state.update(cx, |s, cx| {
                                    s.set_overlay(None, cx);
                                    s.start_go_live(source, preset, cx);
                                })
                            }),
                    ),
            )
            .into_any_element()
    }

    fn toasts(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = theme(cx).clone();
        let toasts = self.state.read(cx).toasts.clone();
        if toasts.is_empty() {
            return None;
        }
        Some(
            div()
                .absolute()
                .bottom(px(24.))
                .left(px(0.))
                .right(px(0.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.))
                .children(toasts.into_iter().map(|toast| {
                    div()
                        .max_w(px(520.))
                        .px(px(14.))
                        .py(px(10.))
                        .rounded(t.radius)
                        .bg(t.floating)
                        .border_1()
                        .border_color(t.border)
                        .shadow_lg()
                        .text_sm()
                        .text_color(t.text)
                        .child(toast.text)
                }))
                .into_any_element(),
        )
    }
}

fn pick_emoji(
    state: &Entity<AppState>,
    chat: &Entity<ChatView>,
    emoji: &Emoji,
    react_to: Option<(Snowflake, Snowflake)>,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    match react_to {
        Some((channel, message)) => state.update(cx, |s, cx| {
            s.set_overlay(None, cx);
            s.react_to = None;
            s.toggle_reaction(channel, message, emoji.clone(), cx);
        }),
        None => {
            let text = format!("{} ", emoji.to_message_form());
            chat.update(cx, |c, cx| c.insert_into_composer(&text, window, cx));
        }
    }
}

impl Render for OverlayLayer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let overlay = self.state.read(cx).overlay.clone();
        let mut root = div().absolute().inset_0().size_full();
        let scrim_close = |el: gpui::Div, state: Entity<AppState>| {
            el.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                state.update(cx, |s, cx| s.set_overlay(None, cx))
            })
            .on_mouse_down(MouseButton::Right, |_, _, _| {})
        };
        match overlay.clone() {
            None | Some(Overlay::Settings(_)) => {}
            Some(Overlay::Menu { at, items }) => {
                root = scrim_close(root, self.state.clone()).child(self.menu(at, items, cx));
            }
            Some(Overlay::Profile { user, guild, at }) => {
                let card = profile_card(&self.state, user, guild, cx);
                root = scrim_close(root, self.state.clone()).child(
                    deferred(
                        anchored()
                            .position(point(at.x + px(12.), at.y - px(40.)))
                            .snap_to_window_with_margin(px(12.))
                            .child(card),
                    )
                    .with_priority(1),
                );
            }
            Some(Overlay::Expressions(tab)) => {
                if tab == ExpressionTab::Gifs
                    && self.state.read(cx).gifs.results.is_empty()
                    && !self.state.read(cx).gifs.loading
                {
                    let q = self.search.read(cx).text().to_owned();
                    self.state.update(cx, |s, cx| s.search_gifs(q, cx));
                }
                if tab == ExpressionTab::Stickers {
                    self.state.update(cx, |s, cx| s.load_sticker_packs(cx));
                }
                root = scrim_close(root, self.state.clone()).child(self.expressions(tab, cx));
            }
            Some(Overlay::Video(_)) => {
                if let Some(video) = &self.video {
                    root = scrim_close(root.bg(gpui::black().opacity(0.85)), self.state.clone())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(video.clone());
                }
            }
            Some(Overlay::QuickSwitcher) => {
                root = scrim_close(root.bg(t.scrim), self.state.clone())
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(self.switcher(cx));
            }
            Some(Overlay::GoLive {
                sources,
                selected,
                preset,
            }) => {
                root = scrim_close(root.bg(t.scrim), self.state.clone())
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(self.go_live_picker(sources, selected, preset, cx));
            }
        }
        let _ = Self::close;
        if overlay.is_none() || matches!(overlay, Some(Overlay::Settings(_))) {
            // Without an overlay the layer must not swallow clicks meant for the window.
            root = div().absolute().bottom(px(0.)).left(px(0.)).right(px(0.));
        }
        root.children(self.toasts(cx))
    }
}
