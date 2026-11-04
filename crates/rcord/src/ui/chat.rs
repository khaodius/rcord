//! The chat area: header, the virtualized message list, typing indicator and composer.

use std::collections::HashSet;
use std::rc::Rc;

use gpui::{
    AnyElement, App, ClipboardItem, Context, ElementId, Entity, FocusHandle, Focusable, FontWeight,
    Hsla, IntoElement, ListAlignment, ListState, ObjectFit, Pixels, Render, SharedString,
    Subscription, Window, div, list, prelude::*, px,
};
use rcord_markup::Block;
use rcord_model::{
    Attachment, ChannelKind, Embed, Message, MessageKind, Snowflake, StickerFormat, cdn,
};
use rcord_settings::MessageDisplay;
use rcord_store::Change;

use crate::LocalTime;
use crate::state::{AppState, ExpressionTab, MenuItem, Overlay, ReplyTo, StateEvent, now_ms};
use crate::theme::{Theme, theme};
use crate::ui::anim_img;
use crate::ui::icons::{Icon, icon};
use crate::ui::markup::{self, MarkupAction, MarkupCx};
use crate::ui::text_input::{InputEvent, TextInput};
use crate::ui::video::VideoSource;
use crate::ui::{Tooltip, avatar, icon_button, should_animate, snowflake_id, user_avatar_url};

/// Messages from one author within this window are grouped under one header.
const GROUP_WINDOW_MS: u64 = 7 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// The top of the channel, once its first message is loaded.
    Welcome,
    Loading,
    Day(SharedString),
    Message {
        id: Snowflake,
        grouped: bool,
    },
    Pending(usize),
}

pub struct ChatView {
    state: Entity<AppState>,
    channel: Option<Snowflake>,
    rows: Rc<Vec<Row>>,
    list: ListState,
    composer: Entity<TextInput>,
    friends: Entity<crate::ui::friends::FriendsView>,
    revealed: Rc<std::cell::RefCell<HashSet<Snowflake>>>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl ChatView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| TextInput::new("Message", cx));
        let mut subs = Vec::new();
        subs.push(cx.subscribe(&state, |this, state, event, cx| {
            let channel = state.read(cx).channel;
            match event {
                StateEvent::Store(change) => match change {
                    Change::Messages(c) | Change::Typing(c) if Some(*c) == channel => {
                        this.refresh(cx)
                    }
                    Change::Everything => this.refresh(cx),
                    Change::Members(_) | Change::Guild(_) | Change::Presence(_) => cx.notify(),
                    _ => {}
                },
                StateEvent::Selection => this.refresh(cx),
                StateEvent::Settings => {
                    this.list.reset(this.rows.len());
                    cx.notify();
                }
                _ => {}
            }
        }));
        subs.push(
            cx.subscribe_in(&composer, window, |this, input, event, window, cx| {
                match event {
                    InputEvent::Submit => {
                        let text = input.read(cx).text().to_owned();
                        if let Some(channel) = this.channel {
                            input.update(cx, |i, cx| i.clear(cx));
                            this.state
                                .update(cx, |s, cx| s.send_message(channel, text, cx));
                        }
                    }
                    InputEvent::Changed => {
                        if let Some(channel) = this.channel
                            && !input.read(cx).text().is_empty()
                        {
                            this.state.update(cx, |s, cx| s.typing(channel, cx));
                        }
                    }
                    InputEvent::Cancel => {
                        this.state.update(cx, |s, cx| {
                            s.reply = None;
                            if let Some(c) = s.channel {
                                s.ack(c, cx);
                            }
                            cx.notify();
                        });
                    }
                    InputEvent::UpInEmpty => {}
                }
                let _ = window;
            }),
        );
        let list = ListState::new(0, ListAlignment::Bottom, px(800.));
        let friends = cx.new(|cx| crate::ui::friends::FriendsView::new(state.clone(), window, cx));
        let mut this = Self {
            state,
            friends,
            channel: None,
            rows: Rc::new(Vec::new()),
            list,
            composer,
            revealed: Rc::default(),
            focus: cx.focus_handle(),
            _subs: subs,
        };
        this.install_scroll_handler(cx);
        this.refresh(cx);
        this
    }

    fn install_scroll_handler(&self, cx: &mut Context<Self>) {
        let state = self.state.clone();
        let weak = cx.weak_entity();
        self.list.set_scroll_handler(move |event, _window, cx| {
            if event.visible_range.start <= 3 {
                let channel = weak.upgrade().and_then(|v| v.read(cx).channel);
                if let Some(channel) = channel {
                    state.update(cx, |s, cx| s.load_older(channel, cx));
                }
            }
        });
    }

    fn build_rows(&self, cx: &App) -> Vec<Row> {
        let s = self.state.read(cx);
        let Some(channel) = s.channel else {
            return Vec::new();
        };
        let offset = cx.global::<LocalTime>().0;
        let mut rows = Vec::new();
        match s.store.messages(channel) {
            Some(list) => {
                rows.push(if list.reached_start {
                    Row::Welcome
                } else {
                    Row::Loading
                });
                let mut prev: Option<&Message> = None;
                let mut prev_day: Option<time::Date> = None;
                for m in &list.messages {
                    let day = day_of(m.created_at_ms(), offset);
                    if prev_day != Some(day) {
                        rows.push(Row::Day(day_label(day).into()));
                        prev = None;
                    }
                    prev_day = Some(day);
                    let grouped = prev.is_some_and(|p| {
                        p.author.id == m.author.id
                            && m.created_at_ms().saturating_sub(p.created_at_ms()) < GROUP_WINDOW_MS
                            && m.message_reference.is_none()
                            && !m.kind.is_system()
                            && !p.kind.is_system()
                            && m.webhook_id.is_none()
                    });
                    rows.push(Row::Message { id: m.id, grouped });
                    prev = Some(m);
                }
            }
            None => rows.push(Row::Loading),
        }
        for i in 0..s.store.pending(channel).len() {
            rows.push(Row::Pending(i));
        }
        rows
    }

    /// Rebuilds the row model and tells the list what changed, keeping the scroll position
    /// steady when history loads above and following the bottom when messages arrive.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let channel = self.state.read(cx).channel;
        let rows = self.build_rows(cx);
        if channel != self.channel {
            self.channel = channel;
            self.revealed.borrow_mut().clear();
            self.list.reset(rows.len());
        } else if *self.rows != rows {
            let old = &self.rows;
            let first_message = |rows: &[Row]| {
                rows.iter().find_map(|r| match r {
                    Row::Message { id, .. } => Some(*id),
                    _ => None,
                })
            };
            // When older history arrives above, the list would stay pinned to the loading row
            // at the top and the new page would appear below it. Remember the message the
            // view starts at, so it can stay put while the page lands above it.
            let top = self.list.logical_scroll_top();
            let prepended = first_message(old) != first_message(&rows);
            let anchor = old.get(top.item_ix..).and_then(|rest| {
                rest.iter().find_map(|r| match r {
                    Row::Message { id, .. } => Some(*id),
                    _ => None,
                })
            });
            let keep_offset = matches!(old.get(top.item_ix), Some(Row::Message { .. }));
            let prefix = old.iter().zip(&rows).take_while(|(a, b)| a == b).count();
            let suffix = old
                .iter()
                .rev()
                .zip(rows.iter().rev())
                .take_while(|(a, b)| a == b)
                .count()
                .min(old.len() - prefix)
                .min(rows.len() - prefix);
            self.list
                .splice(prefix..old.len() - suffix, rows.len() - suffix - prefix);
            if prepended
                && let Some(anchor) = anchor
                && let Some(ix) = rows
                    .iter()
                    .position(|r| matches!(r, Row::Message { id, .. } if *id == anchor))
            {
                self.list.scroll_to(gpui::ListOffset {
                    item_ix: ix,
                    offset_in_item: if keep_offset {
                        top.offset_in_item
                    } else {
                        px(0.)
                    },
                });
            }
        }
        self.rows = Rc::new(rows);
        cx.notify();
    }

    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let s = self.state.read(cx);
        let Some(channel) = s.channel.and_then(|c| s.store.channel(c)) else {
            return div()
                .h(px(48.))
                .flex_none()
                .border_b_1()
                .border_color(t.border)
                .into_any_element();
        };
        let name = s.store.channel_name(channel.id);
        let topic = channel.topic.clone();
        let which = match channel.kind {
            ChannelKind::Dm => Icon::At,
            ChannelKind::GroupDm => Icon::Users,
            ChannelKind::Announcement => Icon::Announcement,
            k if k.is_thread() => Icon::Thread,
            _ => Icon::Hash,
        };
        let members_visible = s
            .settings
            .layout
            .is_visible(rcord_settings::PanelId::Members);
        let state = self.state.clone();
        div()
            .flex_none()
            .h(px(48.))
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(t.border)
            .child(icon(which).size(px(22.)).text_color(t.text_faint))
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::BOLD)
                    .text_color(t.header)
                    .child(name),
            )
            .when_some(topic.filter(|t| !t.is_empty()), |el, topic| {
                el.child(div().w(px(1.)).h(px(24.)).bg(t.border).mx(px(4.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(t.text_muted)
                            .child(topic.replace('\n', " ")),
                    )
            })
            .child(div().flex_1())
            .child(
                icon_button("toggle-members", Icon::Users, &t)
                    .when(members_visible, |el| el.text_color(t.header))
                    .tooltip(move |_, cx| {
                        Tooltip::view(
                            if members_visible {
                                "Hide Member List"
                            } else {
                                "Show Member List"
                            },
                            cx,
                        )
                    })
                    .on_click(move |_, _, cx| {
                        state.update(cx, |s, cx| {
                            s.update_settings(cx, |st| {
                                let p = rcord_settings::PanelId::Members;
                                if st.layout.hidden.contains(&p) {
                                    st.layout.hidden.retain(|x| *x != p);
                                } else {
                                    st.layout.hidden.push(p);
                                }
                            })
                        })
                    }),
            )
            .into_any_element()
    }

    fn typing_line(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let s = self.state.read(cx);
        let Some(channel) = s.channel else {
            return div().h(px(24.)).into_any_element();
        };
        let guild = s.store.channel(channel).and_then(|c| c.guild_id);
        let users = s.store.typing(channel, now_ms());
        let names: Vec<String> = users
            .iter()
            .filter_map(|u| s.store.display_name(guild, *u).map(str::to_owned))
            .collect();
        let text = match names.as_slice() {
            [] => String::new(),
            [a] => format!("{a} is typing…"),
            [a, b] => format!("{a} and {b} are typing…"),
            [a, b, c] => format!("{a}, {b} and {c} are typing…"),
            _ => "Several people are typing…".into(),
        };
        div()
            .flex_none()
            .h(px(24.))
            .px(px(16.))
            .flex()
            .items_center()
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(t.text_muted)
            .child(text)
            .into_any_element()
    }

    fn composer(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let s = self.state.read(cx);
        let Some(channel_id) = s.channel else {
            return div().into_any_element();
        };
        let channel_name = s.store.channel_name(channel_id);
        let is_dm = s
            .store
            .channel(channel_id)
            .is_some_and(|c| c.kind.is_private());
        let can_send = s
            .store
            .permissions(channel_id)
            .contains(rcord_model::Permissions::SEND_MESSAGES);
        let placeholder = if !can_send {
            "You do not have permission to send messages in this channel.".to_owned()
        } else if is_dm {
            format!("Message @{channel_name}")
        } else {
            format!("Message #{channel_name}")
        };
        self.composer
            .update(cx, |c, cx| c.set_placeholder(placeholder, cx));
        let s = self.state.read(cx);
        let reply = s.reply.filter(|r| r.channel == channel_id).and_then(|r| {
            let m = s.store.message(channel_id, r.message)?;
            let guild = m.guild_id;
            Some((
                r,
                s.store
                    .display_name(guild, m.author.id)
                    .unwrap_or(&m.author.username)
                    .to_owned(),
            ))
        });
        let font = s.settings.appearance.chat_font_size;
        let state_g = self.state.clone();
        let state_s = self.state.clone();
        let state_e = self.state.clone();
        let state_r = self.state.clone();
        let state_p = self.state.clone();
        div()
            .flex_none()
            .px(px(16.))
            .flex()
            .flex_col()
            .when_some(reply, |el, (r, name)| {
                el.child(
                    div()
                        .px(px(16.))
                        .h(px(34.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded_t(t.radius)
                        .bg(Theme::panel_footer(&t))
                        .text_sm()
                        .text_color(t.text_muted)
                        .child("Replying to")
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.text)
                                .child(name),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .id("reply-ping")
                                .cursor_pointer()
                                .font_weight(FontWeight::BOLD)
                                .text_color(if r.ping { t.link } else { t.text_faint })
                                .child(if r.ping { "@ ON" } else { "@ OFF" })
                                .on_click(move |_, _, cx| {
                                    state_p.update(cx, |s, cx| {
                                        if let Some(r) = s.reply.as_mut() {
                                            r.ping = !r.ping;
                                        }
                                        cx.notify();
                                    })
                                }),
                        )
                        .child(
                            div()
                                .id("reply-cancel")
                                .cursor_pointer()
                                .text_color(t.text_faint)
                                .hover(|s| s.text_color(t.text))
                                .child(icon(Icon::Close).size(px(16.)))
                                .on_click(move |_, _, cx| {
                                    state_r.update(cx, |s, cx| {
                                        s.reply = None;
                                        cx.notify();
                                    })
                                }),
                        ),
                )
            })
            .child(
                div()
                    .min_h(px(44.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .bg(t.input)
                    .map(|el| {
                        if s.reply.is_some_and(|r| r.channel == channel_id) {
                            el.rounded_b(t.radius)
                        } else {
                            el.rounded(t.radius)
                        }
                    })
                    .child(
                        icon(Icon::PlusCircle)
                            .size(px(22.))
                            .text_color(t.text_muted),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(font))
                            .line_height(px(font * 1.375))
                            .text_color(t.text)
                            .child(self.composer.clone()),
                    )
                    .child(
                        icon_button("gif", Icon::Gif, &t)
                            .tooltip(|_, cx| Tooltip::view("GIFs", cx))
                            .on_click(move |_, _, cx| {
                                state_g.update(cx, |s, cx| {
                                    s.set_overlay(
                                        Some(Overlay::Expressions(ExpressionTab::Gifs)),
                                        cx,
                                    )
                                })
                            }),
                    )
                    .child(
                        icon_button("stickers", Icon::Sticker, &t)
                            .tooltip(|_, cx| Tooltip::view("Stickers", cx))
                            .on_click(move |_, _, cx| {
                                state_s.update(cx, |s, cx| {
                                    s.set_overlay(
                                        Some(Overlay::Expressions(ExpressionTab::Stickers)),
                                        cx,
                                    )
                                })
                            }),
                    )
                    .child(
                        icon_button("emoji", Icon::Smile, &t)
                            .tooltip(|_, cx| Tooltip::view("Emoji", cx))
                            .on_click(move |_, _, cx| {
                                state_e.update(cx, |s, cx| {
                                    s.react_to = None;
                                    s.set_overlay(
                                        Some(Overlay::Expressions(ExpressionTab::Emoji)),
                                        cx,
                                    )
                                })
                            }),
                    ),
            )
            .into_any_element()
    }

    pub fn insert_into_composer(&self, text: &str, window: &mut Window, cx: &mut App) {
        self.composer.update(cx, |c, cx| c.insert(text, window, cx));
    }
}

fn day_of(unix_ms: u64, offset: time::UtcOffset) -> time::Date {
    time::OffsetDateTime::from_unix_timestamp((unix_ms / 1000) as i64)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .to_offset(offset)
        .date()
}

fn day_label(day: time::Date) -> String {
    let fmt = time::macros::format_description!("[month repr:long] [day padding:none], [year]");
    day.format(fmt).unwrap_or_default()
}

/// "Today at 4:20 PM", "Yesterday at 9:02 AM", or the date and time.
pub fn message_time(unix_ms: u64, offset: time::UtcOffset, hour24: bool, compact: bool) -> String {
    let at = time::OffsetDateTime::from_unix_timestamp((unix_ms / 1000) as i64)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .to_offset(offset);
    let clock = if hour24 {
        at.format(time::macros::format_description!("[hour]:[minute]"))
    } else {
        at.format(time::macros::format_description!(
            "[hour repr:12 padding:none]:[minute] [period]"
        ))
    }
    .unwrap_or_default();
    if compact {
        return clock;
    }
    let today = time::OffsetDateTime::now_utc().to_offset(offset).date();
    if at.date() == today {
        format!("Today at {clock}")
    } else if at.date().next_day() == Some(today) {
        format!("Yesterday at {clock}")
    } else {
        let date = at
            .format(time::macros::format_description!("[month]/[day]/[year]"))
            .unwrap_or_default();
        format!("{date} {clock}")
    }
}

struct RowCx {
    state: Entity<AppState>,
    channel: Snowflake,
    revealed: Rc<std::cell::RefCell<HashSet<Snowflake>>>,
}

fn render_row(row: &Row, rcx: &RowCx, window: &mut Window, cx: &mut App) -> AnyElement {
    let t = theme(cx).clone();
    match row {
        Row::Welcome => {
            let s = rcx.state.read(cx);
            let name = s.store.channel_name(rcx.channel);
            let is_dm = s
                .store
                .channel(rcx.channel)
                .is_some_and(|c| c.kind.is_private());
            div()
                .px(px(16.))
                .pt(px(48.))
                .pb(px(16.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .size(px(68.))
                        .rounded_full()
                        .bg(t.selected)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(t.header)
                        .child(icon(if is_dm { Icon::At } else { Icon::Hash }).size(px(40.))),
                )
                .child(
                    div()
                        .text_size(px(30.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(t.header)
                        .child(if is_dm {
                            name.clone()
                        } else {
                            format!("Welcome to #{name}!")
                        }),
                )
                .child(div().text_color(t.text_muted).child(if is_dm {
                    format!("This is the beginning of your direct message history with {name}.")
                } else {
                    format!("This is the start of the #{name} channel.")
                }))
                .into_any_element()
        }
        Row::Loading => div()
            .px(px(16.))
            .py(px(16.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .children((0..3).map(|i| {
                div()
                    .flex()
                    .gap(px(16.))
                    .child(div().size(px(40.)).rounded_full().bg(t.hover))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .w(px(120. + 40. * i as f32))
                                    .h(px(12.))
                                    .rounded_full()
                                    .bg(t.hover),
                            )
                            .child(
                                div()
                                    .w(px(280. - 30. * i as f32))
                                    .h(px(12.))
                                    .rounded_full()
                                    .bg(t.hover),
                            ),
                    )
            }))
            .into_any_element(),
        Row::Day(label) => div()
            .mx(px(16.))
            .my(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().h(px(1.)).bg(t.border))
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text_faint)
                    .child(label.clone()),
            )
            .child(div().flex_1().h(px(1.)).bg(t.border))
            .into_any_element(),
        Row::Pending(i) => {
            let s = rcx.state.read(cx);
            let Some(p) = s.store.pending(rcx.channel).get(*i) else {
                return div().into_any_element();
            };
            let font = s.settings.appearance.chat_font_size;
            let failed = p.failed;
            let nonce = p.nonce.clone();
            let state = rcx.state.clone();
            let channel = rcx.channel;
            div()
                .id(ElementId::NamedInteger("pending".into(), *i as u64))
                .pl(px(72.))
                .pr(px(16.))
                .py(px(2.))
                .text_size(px(font))
                .text_color(if failed { t.danger } else { t.text_faint })
                .child(p.content.clone())
                .when(failed, |el| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(t.text_muted)
                            .child("Not sent. Click to dismiss."),
                    )
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        state.update(cx, |s, cx| s.discard_pending(channel, &nonce, cx))
                    })
                })
                .into_any_element()
        }
        Row::Message { id, grouped } => render_message(*id, *grouped, rcx, window, cx),
    }
}

fn render_message(
    id: Snowflake,
    grouped: bool,
    rcx: &RowCx,
    _window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = theme(cx).clone();
    let offset = cx.global::<LocalTime>().0;
    let s = rcx.state.read(cx);
    let Some(m) = s.store.message(rcx.channel, id) else {
        return div().into_any_element();
    };
    let settings = &s.settings;
    let compact = settings.appearance.message_display == MessageDisplay::Compact;
    let font = px(settings.appearance.chat_font_size);
    let guild = m
        .guild_id
        .or_else(|| s.store.channel(rcx.channel).and_then(|c| c.guild_id));
    let me = s.store.me_id();
    let mentioned = me.is_some_and(|me| m.mentions_user(me))
        || m.mention_roles.iter().any(|r| {
            guild
                .zip(me)
                .and_then(|(g, me)| s.store.member(g, me))
                .is_some_and(|mem| mem.roles.contains(r))
        });
    let name = s
        .store
        .display_name(guild, m.author.id)
        .unwrap_or(m.author.display_name())
        .to_owned();
    let color = match (guild, settings.appearance.role_colors) {
        (Some(g), rcord_settings::RoleColors::InNames) => s
            .store
            .name_color(g, m.author.id)
            .map(|c| t.role_color(c, settings.accessibility.role_color_contrast)),
        _ => None,
    };
    let revealed = rcx.revealed.borrow().contains(&id);
    let state_for_actions = rcx.state.clone();
    let revealed_set = rcx.revealed.clone();
    let on_action: markup::ActionHandler = Rc::new(move |action, window, cx| match action {
        MarkupAction::OpenUrl(url) => cx.open_url(&url),
        MarkupAction::User(u) => {
            let at = window.mouse_position();
            state_for_actions.update(cx, |s, cx| s.open_profile(u, guild, at, cx));
        }
        MarkupAction::Channel(c) => state_for_actions.update(cx, |s, cx| s.select_channel(c, cx)),
        MarkupAction::RevealSpoilers => {
            revealed_set.borrow_mut().insert(id);
            window.refresh();
        }
    });
    let mcx = MarkupCx {
        store: &s.store,
        guild,
        t: &t,
        font_size: font,
        animate_emoji: settings.appearance.animate_emoji != rcord_settings::Animate::Never,
        revealed,
        underline_links: settings.accessibility.underline_links,
        hour24: settings.appearance.hour24,
        utc_offset: offset,
        now_secs: (now_ms() / 1000) as i64,
        code_font: theme(cx).code_font.clone(),
        key: id.get(),
        on_action,
    };

    if m.kind.is_system() {
        let text = system_text(m, &name);
        return div()
            .px(px(16.))
            .py(px(4.))
            .pl(px(72.))
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(font)
            .text_color(t.text_muted)
            .child(icon(Icon::ChevronRight).size(px(16.)).text_color(t.online))
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.header)
                    .child(name),
            )
            .child(text)
            .child(div().text_xs().text_color(t.text_faint).child(message_time(
                m.created_at_ms(),
                offset,
                settings.appearance.hour24,
                false,
            )))
            .into_any_element();
    }

    let blocks: Vec<Block> = rcord_markup::parse(&m.content);
    let body = if m.content.is_empty() {
        None
    } else {
        Some(markup::blocks(&blocks, &mcx))
    };
    let edited = m.edited_timestamp.is_some();
    let time_text = message_time(
        m.created_at_ms(),
        offset,
        settings.appearance.hour24,
        compact,
    );

    let mut content = div().flex().flex_col().min_w_0().gap(px(4.));
    if let Some(body) = body {
        content = content.child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .text_size(font)
                .line_height(font * 1.375)
                .text_color(t.text)
                .child(body)
                .when(edited, |el| {
                    el.child(div().text_xs().text_color(t.text_faint).child("(edited)"))
                }),
        );
    }
    if settings.chat.inline_attachments {
        for a in &m.attachments {
            content = content.child(attachment(a, settings.chat.max_media_height, &t, id));
        }
    }
    if settings.chat.link_previews {
        for (i, e) in m.embeds.iter().enumerate() {
            content = content.child(embed(e, &mcx, settings.chat.max_media_height, i));
        }
    }
    for st in &m.sticker_items {
        content = content.child(sticker(st, &t, settings.chat.sticker_autoplay));
    }
    if settings.chat.show_reactions && !m.reactions.is_empty() {
        let mut row = div().flex().flex_row().flex_wrap().gap(px(4.)).mt(px(2.));
        for (i, r) in m.reactions.iter().enumerate() {
            let emoji = r.emoji.clone();
            let state = rcx.state.clone();
            let channel = rcx.channel;
            let emoji_el = match cdn::emoji(&r.emoji, 32) {
                Some(url) => anim_img(url)
                    .size(px(16.))
                    .object_fit(ObjectFit::Contain)
                    .into_any_element(),
                None => div()
                    .text_size(px(15.))
                    .child(r.emoji.name.clone().unwrap_or_default())
                    .into_any_element(),
            };
            row = row.child(
                div()
                    .id(gpui::ElementId::NamedInteger(
                        format!("reaction-{i}").into(),
                        id.get(),
                    ))
                    .h(px(26.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .rounded(px(8.))
                    .border_1()
                    .cursor_pointer()
                    .map(|el| {
                        if r.me || r.me_burst {
                            el.bg(Hsla {
                                a: 0.15,
                                ..t.accent
                            })
                            .border_color(t.accent)
                        } else {
                            el.bg(t.hover)
                                .border_color(gpui::transparent_black())
                                .hover(|s| s.border_color(t.border))
                        }
                    })
                    .child(emoji_el)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if r.me { t.mention_text } else { t.text_muted })
                            .child(r.count.to_string()),
                    )
                    .on_click(move |_, _, cx| {
                        state.update(cx, |s, cx| {
                            s.toggle_reaction(channel, id, emoji.clone(), cx)
                        })
                    }),
            );
        }
        content = content.child(row);
    }

    let reply = m.referenced_message.as_deref().map(|r| {
        let rname = s
            .store
            .display_name(guild, r.author.id)
            .unwrap_or(r.author.display_name())
            .to_owned();
        let preview = rcord_markup::parse(&r.content)
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph(i) => Some(rcord_markup::plain_text(i)),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        let preview = if preview.is_empty() {
            "Click to see attachment".to_owned()
        } else {
            preview
        };
        div()
            .pl(px(72.))
            .pr(px(16.))
            .pt(px(4.))
            .flex()
            .items_center()
            .gap(px(6.))
            .text_sm()
            .text_color(t.text_muted)
            .child(avatar(
                user_avatar_url(&r.author, 16., false),
                px(16.),
                None,
            ))
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child(format!("@{rname}")),
            )
            .child(div().flex_1().min_w_0().truncate().child(preview))
    });

    let author = m.author.clone();
    let author_id = author.id;
    let state_profile = rcx.state.clone();
    let state_profile2 = rcx.state.clone();
    let animate = should_animate(settings.appearance.animate_avatars, false);
    let avatar_url = match (
        guild,
        guild
            .and_then(|g| s.store.member(g, author_id))
            .and_then(|mm| mm.avatar.clone()),
    ) {
        (Some(g), Some(hash)) => cdn::member_avatar(g, author_id, &hash, 80, animate),
        _ => user_avatar_url(&author, 40., animate),
    };
    let bot = author.bot;
    let name_el = div()
        .id(gpui::ElementId::NamedInteger("author".into(), id.get()))
        .flex_none()
        .font_weight(FontWeight::MEDIUM)
        .text_color(color.unwrap_or(t.header))
        .cursor_pointer()
        .hover(|s| s.underline())
        .child(name.clone())
        .on_click(move |e, _, cx| {
            let at = e.position();
            state_profile.update(cx, |s, cx| s.open_profile(author_id, guild, at, cx))
        });
    let bot_tag = bot.then(|| {
        div()
            .px(px(4.))
            .rounded(px(3.))
            .bg(t.accent)
            .text_color(t.accent_text)
            .text_size(px(10.))
            .font_weight(FontWeight::BOLD)
            .child(if m.webhook_id.is_some() {
                "WEBHOOK"
            } else {
                "APP"
            })
    });

    let toolbar = message_toolbar(m, rcx, &t, me, guild);
    let group = SharedString::from(format!("msg-{id}"));
    let spacing = if grouped {
        px(0.)
    } else {
        px(settings.appearance.group_spacing)
    };

    let line = if compact {
        div()
            .pl(px(16.))
            .pr(px(48.))
            .py(px(2.))
            .flex()
            .flex_row()
            .gap(px(8.))
            .child(
                div()
                    .flex_none()
                    .w(px(56.))
                    .pt(px(3.))
                    .text_xs()
                    .text_color(t.text_faint)
                    .child(time_text),
            )
            .child(
                div().flex_1().min_w_0().flex().flex_col().child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(6.))
                        .items_start()
                        .text_size(font)
                        .when(settings.appearance.show_avatars_in_compact, |el| {
                            el.child(avatar(avatar_url.clone(), px(16.), None).mt(px(3.)))
                        })
                        .child(name_el)
                        .children(bot_tag)
                        .child(div().flex_1().min_w_0().child(content)),
                ),
            )
            .into_any_element()
    } else if grouped {
        div()
            .pl(px(72.))
            .pr(px(48.))
            .py(px(1.))
            .relative()
            .child(
                div()
                    .absolute()
                    .left(px(0.))
                    .top(px(4.))
                    .w(px(64.))
                    .text_size(px(11.))
                    .text_color(t.text_faint)
                    .flex()
                    .justify_end()
                    .invisible()
                    .group_hover(group.clone(), |s| s.visible())
                    .child(message_time(
                        m.created_at_ms(),
                        offset,
                        settings.appearance.hour24,
                        true,
                    )),
            )
            .child(content)
            .into_any_element()
    } else {
        div()
            .pl(px(16.))
            .pr(px(48.))
            .pt(px(2.))
            .pb(px(1.))
            .flex()
            .flex_row()
            .gap(px(16.))
            .child(
                div()
                    .id(gpui::ElementId::NamedInteger("avatar".into(), id.get()))
                    .flex_none()
                    .mt(px(2.))
                    .cursor_pointer()
                    .child(avatar(avatar_url, px(40.), None))
                    .on_click(move |e, _, cx| {
                        let at = e.position();
                        state_profile2.update(cx, |s, cx| s.open_profile(author_id, guild, at, cx))
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .text_size(font)
                            .child(name_el)
                            .children(bot_tag)
                            .child(div().text_xs().text_color(t.text_faint).child(time_text)),
                    )
                    .child(content),
            )
            .into_any_element()
    };

    div()
        .id(snowflake_id("message", id))
        .group(group)
        .relative()
        .mt(spacing)
        .flex()
        .flex_col()
        .when(mentioned, |el| {
            el.bg(t.mentioned_row).child(
                div()
                    .absolute()
                    .left(px(0.))
                    .top(px(0.))
                    .bottom(px(0.))
                    .w(px(2.))
                    .bg(t.idle),
            )
        })
        .when(!mentioned, |el| {
            el.hover(|s| {
                s.bg(Hsla {
                    a: t.hover.a * 0.6,
                    ..t.hover
                })
            })
        })
        .children(reply)
        .child(line)
        .child(toolbar)
        .into_any_element()
}

fn system_text(m: &Message, _name: &str) -> String {
    match m.kind {
        MessageKind::UserJoin => "joined the server.".into(),
        MessageKind::GuildBoost => "boosted the server!".into(),
        MessageKind::GuildBoostTier1 => "boosted the server! It reached Level 1.".into(),
        MessageKind::GuildBoostTier2 => "boosted the server! It reached Level 2.".into(),
        MessageKind::GuildBoostTier3 => "boosted the server! It reached Level 3.".into(),
        MessageKind::ChannelPinnedMessage => "pinned a message to this channel.".into(),
        MessageKind::RecipientAdd => "added someone to the group.".into(),
        MessageKind::RecipientRemove => "left the group.".into(),
        MessageKind::Call => "started a call.".into(),
        MessageKind::ChannelNameChange => format!("changed the channel name: {}", m.content),
        MessageKind::ChannelIconChange => "changed the channel icon.".into(),
        MessageKind::ThreadCreated => format!("started a thread: {}", m.content),
        MessageKind::ChannelFollowAdd => format!("added {} to this channel.", m.content),
        _ => m.content.clone(),
    }
}

fn message_toolbar(
    m: &Message,
    rcx: &RowCx,
    t: &Theme,
    me: Option<Snowflake>,
    guild: Option<Snowflake>,
) -> AnyElement {
    let id = m.id;
    let channel = rcx.channel;
    let own = me == Some(m.author.id);
    let content = m.content.clone();
    let state_reply = rcx.state.clone();
    let state_react = rcx.state.clone();
    let state_more = rcx.state.clone();
    let group = SharedString::from(format!("msg-{id}"));
    let button = |name: &'static str, which: Icon, tip: &'static str| {
        div()
            .id(gpui::ElementId::NamedInteger(name.into(), id.get()))
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .text_color(t.text_muted)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover).text_color(t.text))
            .tooltip(move |_, cx| Tooltip::view(tip, cx))
            .child(icon(which).size(px(18.)))
    };
    div()
        .absolute()
        .top(px(-16.))
        .right(px(16.))
        .invisible()
        .group_hover(group, |s| s.visible())
        .flex()
        .flex_row()
        .rounded(t.radius_small)
        .border_1()
        .border_color(t.border)
        .bg(t.floating)
        .shadow_sm()
        .overflow_hidden()
        .child(
            button("react", Icon::Smile, "Add Reaction").on_click(move |_, _, cx| {
                state_react.update(cx, |s, cx| {
                    s.react_to = Some((channel, id));
                    s.set_overlay(Some(Overlay::Expressions(ExpressionTab::Emoji)), cx);
                })
            }),
        )
        .child(
            button("reply", Icon::Reply, "Reply").on_click(move |_, _, cx| {
                state_reply.update(cx, |s, cx| {
                    let ping = s.settings.chat.reply_ping_default;
                    s.reply = Some(ReplyTo {
                        channel,
                        message: id,
                        ping,
                    });
                    cx.emit(StateEvent::Selection);
                    cx.notify();
                })
            }),
        )
        .child(
            button("more", Icon::More, "More").on_click(move |e, _, cx| {
                let at = e.position();
                let content = content.clone();
                let dev = state_more.read(cx).settings.advanced.developer_mode;
                let mut items = vec![MenuItem {
                    label: "Copy Text".into(),
                    danger: false,
                    action: Rc::new(move |cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(content.clone()))
                    }),
                }];
                let link = match guild {
                    Some(g) => format!("https://discord.com/channels/{g}/{channel}/{id}"),
                    None => format!("https://discord.com/channels/@me/{channel}/{id}"),
                };
                items.push(MenuItem {
                    label: "Copy Message Link".into(),
                    danger: false,
                    action: Rc::new(move |cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(link.clone()))
                    }),
                });
                if dev {
                    items.push(MenuItem {
                        label: "Copy Message ID".into(),
                        danger: false,
                        action: Rc::new(move |cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(id.to_string()))
                        }),
                    });
                }
                if own {
                    let st = state_more.clone();
                    items.push(MenuItem {
                        label: "Delete Message".into(),
                        danger: true,
                        action: Rc::new(move |cx| {
                            st.update(cx, |s, cx| s.delete_message(channel, id, cx))
                        }),
                    });
                }
                state_more.update(cx, |s, cx| {
                    s.set_overlay(Some(Overlay::Menu { at, items }), cx)
                });
            }),
        )
        .into_any_element()
}

/// Scales `w`×`h` to fit within the given box, keeping its aspect ratio.
fn fit(w: u32, h: u32, max_w: f32, max_h: f32) -> (Pixels, Pixels) {
    if w == 0 || h == 0 {
        return (px(max_w.min(400.)), px(max_h.min(225.)));
    }
    let scale = (max_w / w as f32).min(max_h / h as f32).min(1.0);
    (px(w as f32 * scale), px(h as f32 * scale))
}

fn attachment(a: &Attachment, max_h: f32, t: &Theme, message: Snowflake) -> AnyElement {
    if a.is_image() || a.is_video() {
        let (w, h) = fit(a.width.unwrap_or(0), a.height.unwrap_or(0), 520., max_h);
        let base = a.proxy_url.clone().unwrap_or_else(|| a.url.clone());
        let src = if a.is_video() {
            // The media proxy serves a still frame of a video when asked for an image.
            cdn::sized_proxy_url(
                &format!(
                    "{base}{}format=webp",
                    if base.contains('?') { "&" } else { "?" }
                ),
                (f32::from(w) * 2.) as u32,
                (f32::from(h) * 2.) as u32,
            )
        } else if a.filename.to_ascii_lowercase().ends_with(".gif") {
            // The media proxy's resized copy of a GIF is a still image; the original plays.
            a.url.clone()
        } else {
            cdn::sized_proxy_url(
                &base,
                (f32::from(w) * 2.) as u32,
                (f32::from(h) * 2.) as u32,
            )
        };
        let url = a.url.clone();
        let video = (a.is_video()
            && VideoSource::playable(&a.filename, a.content_type.as_deref(), a.size))
        .then(|| VideoSource {
            url: a.url.clone(),
            filename: a.filename.clone(),
            size: a.size,
            width: a.width.unwrap_or(0),
            height: a.height.unwrap_or(0),
        });
        let spoiler = a.is_spoiler();
        return div()
            .id(gpui::ElementId::NamedInteger(
                format!("att-{}", a.id).into(),
                message.get(),
            ))
            .relative()
            .w(w)
            .h(h)
            .rounded(t.radius)
            .overflow_hidden()
            .bg(t.hover)
            .cursor_pointer()
            .child(
                anim_img(src)
                    .w(w)
                    .h(h)
                    .object_fit(ObjectFit::Cover)
                    .when(spoiler, |i| i.grayscale(true)),
            )
            .when(a.is_video(), |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .size(px(48.))
                                .rounded_full()
                                .bg(t.scrim)
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(gpui::white())
                                .child(icon(Icon::Video).size(px(22.))),
                        ),
                )
            })
            .on_click(move |_, _, cx| match &video {
                Some(source) => match cx.try_global::<crate::MainState>().map(|m| m.0.clone()) {
                    Some(state) => state.update(cx, |s, cx| {
                        s.set_overlay(Some(Overlay::Video(source.clone())), cx)
                    }),
                    None => cx.open_url(&url),
                },
                None => cx.open_url(&url),
            })
            .into_any_element();
    }
    let url = a.url.clone();
    let size = human_size(a.size);
    div()
        .id(gpui::ElementId::NamedInteger(
            format!("file-{}", a.id).into(),
            message.get(),
        ))
        .max_w(px(432.))
        .p(px(12.))
        .flex()
        .items_center()
        .gap(px(10.))
        .rounded(t.radius)
        .border_1()
        .border_color(t.border)
        .bg(t.sidebar)
        .child(icon(Icon::Inbox).size(px(30.)).text_color(t.text_muted))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .truncate()
                        .text_color(t.link)
                        .cursor_pointer()
                        .child(a.filename.clone()),
                )
                .child(div().text_xs().text_color(t.text_faint).child(size)),
        )
        .on_click(move |_, _, cx| cx.open_url(&url))
        .into_any_element()
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["bytes", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{v:.2} {}", UNITS[unit])
    }
}

/// Tenor sends GIF links as MP4 video embeds. The same media is available as a GIF under
/// the same ID with the GIF format code, which GPUI can animate.
fn tenor_gif(e: &Embed) -> Option<String> {
    let video = e.video.as_ref()?.url.as_deref()?;
    // Giphy keeps the GIF beside the MP4: .../media/<id>/giphy.mp4 and .../giphy.gif.
    if video.contains("giphy.com/media/")
        && let Some(base) = video.strip_suffix(".mp4")
    {
        return Some(format!("{base}.gif"));
    }
    let rest = video.strip_prefix("https://media.tenor.com/")?;
    let (id, name) = rest.split_once('/')?;
    // The last four characters of the ID name the format: AAPo is MP4, AAAC is GIF.
    let base = id.strip_suffix("AAPo")?;
    let name = name.rsplit_once('.').map_or(name, |(n, _)| n);
    Some(format!("https://media.tenor.com/{base}AAAC/{name}.gif"))
}

fn embed(e: &Embed, mcx: &MarkupCx<'_>, max_h: f32, index: usize) -> AnyElement {
    let t = mcx.t;
    if e.is_gifv() {
        let media = e.video.as_ref().or(e.thumbnail.as_ref());
        let (w, h) = media.map_or((px(300.), px(200.)), |m| {
            fit(m.width.unwrap_or(0), m.height.unwrap_or(0), 420., max_h)
        });
        let src = tenor_gif(e)
            .or_else(|| {
                e.thumbnail
                    .as_ref()
                    .and_then(|th| th.proxy_url.clone().or(th.url.clone()))
            })
            .unwrap_or_default();
        return div()
            .w(w)
            .h(h)
            .rounded(t.radius)
            .overflow_hidden()
            .bg(t.hover)
            .child(anim_img(src).w(w).h(h).object_fit(ObjectFit::Cover))
            .into_any_element();
    }
    let accent = e.color.map_or(t.border, |c| gpui::rgb(c).into());
    let mut body = div().flex().flex_col().gap(px(6.)).min_w_0().flex_1();
    if let Some(p) = &e.provider
        && let Some(name) = &p.name
    {
        body = body.child(div().text_xs().text_color(t.text_muted).child(name.clone()));
    }
    if let Some(a) = &e.author {
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .when_some(
                    a.proxy_icon_url.clone().or(a.icon_url.clone()),
                    |el, url| el.child(anim_img(url).size(px(20.)).rounded_full()),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.header)
                        .child(a.name.clone()),
                ),
        );
    }
    if let Some(title) = &e.title {
        let url = e.url.clone();
        body = body.child(
            div()
                .id(gpui::ElementId::NamedInteger(
                    format!("embed-title-{index}").into(),
                    mcx.key,
                ))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if url.is_some() { t.link } else { t.header })
                .when_some(url, |el, url| {
                    el.cursor_pointer()
                        .on_click(move |_, _, cx| cx.open_url(&url))
                })
                .child(title.clone()),
        );
    }
    if let Some(desc) = &e.description {
        body = body.child(
            div()
                .text_sm()
                .text_color(t.text)
                .child(markup::blocks(&rcord_markup::parse(desc), mcx)),
        );
    }
    if !e.fields.is_empty() {
        let mut grid = div().flex().flex_row().flex_wrap().gap(px(8.));
        for f in &e.fields {
            grid = grid.child(
                div()
                    .min_w(px(if f.inline { 150. } else { 400. }))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.header)
                            .child(f.name.clone()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(t.text)
                            .child(markup::blocks(&rcord_markup::parse(&f.value), mcx)),
                    ),
            );
        }
        body = body.child(grid);
    }
    if let Some(image) = &e.image
        && let Some(src) = image.proxy_url.clone().or(image.url.clone())
    {
        let (w, h) = fit(
            image.width.unwrap_or(0),
            image.height.unwrap_or(0),
            400.,
            max_h,
        );
        body = body.child(
            anim_img(cdn::sized_proxy_url(
                &src,
                (f32::from(w) * 2.) as u32,
                (f32::from(h) * 2.) as u32,
            ))
            .w(w)
            .h(h)
            .rounded(t.radius_small)
            .object_fit(ObjectFit::Cover),
        );
    }
    if let Some(f) = &e.footer {
        body = body.child(
            div()
                .text_xs()
                .text_color(t.text_muted)
                .child(f.text.clone()),
        );
    }
    let thumb = e
        .thumbnail
        .as_ref()
        .filter(|_| e.image.is_none())
        .and_then(|th| th.proxy_url.clone().or(th.url.clone()))
        .map(|src| {
            anim_img(src)
                .size(px(80.))
                .rounded(t.radius_small)
                .object_fit(ObjectFit::Cover)
                .flex_none()
        });
    div()
        .max_w(px(520.))
        .flex()
        .flex_row()
        .rounded(t.radius_small)
        .overflow_hidden()
        .bg(t.sidebar)
        .child(div().w(px(4.)).flex_none().bg(accent))
        .child(
            div()
                .flex()
                .flex_row()
                .gap(px(12.))
                .p(px(12.))
                .flex_1()
                .min_w_0()
                .child(body)
                .children(thumb),
        )
        .into_any_element()
}

fn sticker(st: &rcord_model::StickerItem, t: &Theme, autoplay: bool) -> AnyElement {
    let _ = autoplay;
    match cdn::sticker(st, 320) {
        Some(url) => anim_img(url)
            .size(px(160.))
            .object_fit(ObjectFit::Contain)
            .into_any_element(),
        None => div()
            .size(px(160.))
            .rounded(t.radius)
            .bg(t.hover)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(4.))
            .text_color(t.text_muted)
            .child(icon(Icon::Sticker).size(px(40.)))
            .child(div().text_sm().child(st.name.clone()))
            .when(st.format_type == StickerFormat::Lottie, |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(t.text_faint)
                        .child("Animated sticker"),
                )
            })
            .into_any_element(),
    }
}

impl Focusable for ChatView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let has_channel = self.state.read(cx).channel.is_some();
        if !has_channel {
            let home = self.state.read(cx).guild.is_none();
            if home {
                return self.friends.clone().into_any_element();
            }
            return div()
                .size_full()
                .bg(t.surface)
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(12.))
                .text_color(t.text_muted)
                .child(
                    icon(if home { Icon::Users } else { Icon::Hash })
                        .size(px(64.))
                        .text_color(t.text_faint),
                )
                .child(if home {
                    "Pick a conversation from the list."
                } else {
                    "No text channels you can read here."
                })
                .into_any_element();
        }
        let header = self.header(cx);
        let typing = self.typing_line(cx);
        let composer = self.composer(cx);
        let rows = self.rows.clone();
        let rcx = Rc::new(RowCx {
            state: self.state.clone(),
            channel: self
                .channel
                .unwrap_or(Snowflake::new(1).unwrap_or_else(|| unreachable!())),
            revealed: self.revealed.clone(),
        });
        let messages = list(self.list.clone(), move |ix, window, cx| {
            match rows.get(ix) {
                Some(row) => render_row(row, &rcx, window, cx),
                None => div().into_any_element(),
            }
        })
        .size_full();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.surface)
            .track_focus(&self.focus)
            .child(header)
            .child(div().flex_1().min_h_0().pb(px(8.)).child(messages))
            .child(composer)
            .child(typing)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenor_mp4_maps_to_gif() {
        let e: Embed = serde_json::from_value(serde_json::json!({
            "type": "gifv",
            "video": {"url": "https://media.tenor.com/mhBlGaU-m7cAAAPo/cat-dance.mp4", "width": 498, "height": 280}
        }))
        .unwrap();
        assert_eq!(
            tenor_gif(&e).as_deref(),
            Some("https://media.tenor.com/mhBlGaU-m7cAAAAC/cat-dance.gif")
        );
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512 bytes");
        assert_eq!(human_size(1536), "1.50 KB");
        assert_eq!(fit(1000, 500, 500., 350.), (px(500.), px(250.)));
        assert_eq!(fit(100, 100, 500., 350.), (px(100.), px(100.)));
    }
}
