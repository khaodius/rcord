//! The channel list: a server's categories and channels, or the DM list, with the account
//! panel at the bottom.

use gpui::{
    AnyElement, ClipboardItem, Context, Entity, FontWeight, IntoElement, MouseButton, Render,
    ScrollHandle, Subscription, Window, div, prelude::*, px,
};
use rcord_model::{Channel, ChannelKind, Snowflake, Status};
use rcord_store::Change;

use crate::state::{AppState, Connection, MenuItem, Overlay, SettingsPage, StateEvent};
use crate::theme::{Theme, theme};
use crate::ui::icons::{Icon, icon};
use crate::ui::{
    Tooltip, avatar, icon_button, mention_badge, section_label, snowflake_id, user_avatar_url,
};

pub struct ChannelSidebar {
    state: Entity<AppState>,
    scroll: ScrollHandle,
    _sub: Subscription,
}

impl ChannelSidebar {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let sub = cx.subscribe(&state, |this, state, event, cx| {
            let guild = state.read(cx).guild;
            match event {
                StateEvent::Store(change) => match change {
                    Change::Everything
                    | Change::Guilds
                    | Change::Messages(_)
                    | Change::ReadState(_)
                    | Change::Me
                    | Change::Relationships
                    | Change::Voice(_) => cx.notify(),
                    Change::Guild(g) if Some(*g) == guild => cx.notify(),
                    Change::Channels(g) if *g == guild => cx.notify(),
                    Change::Presence(_) if guild.is_none() => cx.notify(),
                    _ => {}
                },
                StateEvent::Selection | StateEvent::Settings | StateEvent::Screen => cx.notify(),
                StateEvent::Overlay => {}
            }
            let _ = this;
        });
        Self {
            state,
            scroll: ScrollHandle::new(),
            _sub: sub,
        }
    }

    fn channel_icon(c: &Channel, private: bool) -> Icon {
        match c.kind {
            ChannelKind::Voice | ChannelKind::Stage => Icon::Speaker,
            ChannelKind::Announcement => Icon::Announcement,
            ChannelKind::Forum | ChannelKind::Media => Icon::Forum,
            k if k.is_thread() => Icon::Thread,
            _ if private => Icon::Lock,
            _ => Icon::Hash,
        }
    }

    fn channel_row(&self, id: Snowflake, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = theme(cx).clone();
        let s = self.state.read(cx);
        let c = s.store.channel(id)?;
        let selected = s.channel == Some(id);
        let unread = s.store.is_unread(id);
        let mentions = s.store.read_state(id).mentions;
        let private = c.permission_overwrites.iter().any(|o| {
            Some(o.id) == c.guild_id && o.deny.contains(rcord_model::Permissions::VIEW_CHANNEL)
        });
        let which = Self::channel_icon(c, private);
        let name = c.name.clone().unwrap_or_default();
        let is_voice = c.kind.is_voice();
        let dev = s.settings.advanced.developer_mode;
        let voice: Vec<(Snowflake, String, String, bool, bool)> = if is_voice {
            s.store
                .voice_states_in(id)
                .filter_map(|v| {
                    let user = s.store.user(v.user_id)?;
                    Some((
                        v.user_id,
                        s.store
                            .display_name(c.guild_id, v.user_id)
                            .unwrap_or(&user.username)
                            .to_owned(),
                        user_avatar_url(user, 24., false),
                        v.self_mute || v.mute,
                        v.self_deaf || v.deaf,
                    ))
                })
                .collect()
        } else {
            Vec::new()
        };
        let state = self.state.clone();
        let menu_state = self.state.clone();
        let row = div()
            .id(snowflake_id("channel", id))
            .mx(px(8.))
            .px(px(8.))
            .h(px(34.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .rounded(t.radius_small)
            .cursor_pointer()
            .text_color(if selected || unread {
                t.header
            } else {
                t.text_faint
            })
            .when(selected, |el| el.bg(t.selected))
            .when(!selected, |el| {
                el.hover(|s| s.bg(t.hover).text_color(t.text))
            })
            .child(icon(which).size(px(18.)).text_color(t.text_faint))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(if unread || selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .child(name),
            )
            .when(mentions > 0, |el| el.child(mention_badge(mentions, &t)))
            .on_click(move |_, _, cx| state.update(cx, |s, cx| s.select_channel(id, cx)))
            .on_mouse_up(MouseButton::Right, move |e, _, cx| {
                let at = e.position;
                let mut items = vec![MenuItem {
                    label: "Mark as read".into(),
                    danger: false,
                    action: std::rc::Rc::new({
                        let state = menu_state.clone();
                        move |cx| state.update(cx, |s, cx| s.ack(id, cx))
                    }),
                }];
                if dev {
                    items.push(MenuItem {
                        label: "Copy channel ID".into(),
                        danger: false,
                        action: std::rc::Rc::new(move |cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(id.to_string()))
                        }),
                    });
                }
                menu_state.update(cx, |s, cx| {
                    s.set_overlay(Some(Overlay::Menu { at, items }), cx)
                });
            });

        // The unread dot sits at the panel's edge, as in Discord.
        let mut wrapper =
            div()
                .relative()
                .flex()
                .flex_col()
                .child(row)
                .when(unread && !selected, |el| {
                    el.child(
                        div()
                            .absolute()
                            .left(px(0.))
                            .top(px(13.))
                            .w(px(4.))
                            .h(px(8.))
                            .rounded_r(px(4.))
                            .bg(t.header),
                    )
                });
        for (user, name, url, muted, deafened) in voice {
            let state = self.state.clone();
            wrapper = wrapper.child(
                div()
                    .id(snowflake_id("voice-user", user))
                    .ml(px(36.))
                    .mr(px(8.))
                    .px(px(8.))
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(t.radius_small)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .text_color(t.text_muted)
                    .child(avatar(url, px(24.), None))
                    .child(div().flex_1().min_w_0().truncate().text_sm().child(name))
                    .when(muted, |el| el.child(icon(Icon::MicOff).size(px(16.))))
                    .when(deafened, |el| {
                        el.child(icon(Icon::HeadphonesOff).size(px(16.)))
                    })
                    .on_click(move |e, _, cx| {
                        let at = e.position();
                        state.update(cx, |s, cx| {
                            let g = s.guild;
                            s.open_profile(user, g, at, cx)
                        })
                    }),
            );
        }
        Some(wrapper.into_any_element())
    }

    fn guild_list(&self, guild: Snowflake, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = theme(cx).clone();
        let s = self.state.read(cx);
        let selected = s.channel;
        // Resolve names and what a collapsed category still shows before building rows,
        // which need the context mutably.
        let groups: Vec<(Option<(Snowflake, String, bool)>, Vec<Snowflake>)> = s
            .store
            .channel_groups(guild)
            .into_iter()
            .map(|group| {
                let header = group.category.map(|cat| {
                    let name = s
                        .store
                        .channel(cat)
                        .and_then(|c| c.name.clone())
                        .unwrap_or_default();
                    (cat, name, s.collapsed_categories.contains(&cat))
                });
                let channels = match &header {
                    // A collapsed category still shows the selected channel and unread ones.
                    Some((_, _, true)) => group
                        .channels
                        .iter()
                        .copied()
                        .filter(|c| Some(*c) == selected || s.store.is_unread(*c))
                        .collect(),
                    _ => group.channels,
                };
                (header, channels)
            })
            .collect();
        let mut out = Vec::new();
        for (header, channels) in groups {
            if let Some((cat, name, is_collapsed)) = header {
                let state = self.state.clone();
                out.push(
                    div()
                        .id(snowflake_id("category", cat))
                        .mt(px(16.))
                        .mb(px(2.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .cursor_pointer()
                        .text_color(t.text_faint)
                        .hover(|s| s.text_color(t.text))
                        .child(
                            icon(if is_collapsed {
                                Icon::ChevronRight
                            } else {
                                Icon::ChevronDown
                            })
                            .size(px(12.)),
                        )
                        .child(section_label(name, &t))
                        .on_click(move |_, _, cx| {
                            state.update(cx, |s, cx| {
                                if !s.collapsed_categories.remove(&cat) {
                                    s.collapsed_categories.insert(cat);
                                }
                                cx.emit(StateEvent::Selection);
                            })
                        })
                        .into_any_element(),
                );
            }
            for c in channels {
                if let Some(row) = self.channel_row(c, cx) {
                    out.push(row);
                }
            }
        }
        out
    }

    fn dm_list(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = theme(cx).clone();
        let home_state = self.state.clone();
        let s = self.state.read(cx);
        let mut out = vec![
            div()
                .id("friends-home")
                .cursor_pointer()
                .hover(|st| st.bg(t.hover))
                .on_click(move |_, _, cx| home_state.update(cx, |s, cx| s.show_friends(cx)))
                .mx(px(8.))
                .px(px(8.))
                .h(px(42.))
                .flex()
                .items_center()
                .gap(px(12.))
                .rounded(t.radius_small)
                .bg(if s.channel.is_none() {
                    t.selected
                } else {
                    gpui::transparent_black()
                })
                .text_color(t.header)
                .font_weight(FontWeight::MEDIUM)
                .child(icon(Icon::Users).size(px(22.)))
                .child("Friends")
                .into_any_element(),
            div()
                .mt(px(16.))
                .mb(px(4.))
                .px(px(16.))
                .child(section_label("Direct Messages", &t))
                .into_any_element(),
        ];
        let rows: Vec<_> = s
            .store
            .private_channels()
            .map(|c| {
                let other = c.recipient_ids().next();
                let user = other.and_then(|u| s.store.user(u));
                let url = match (c.kind, user) {
                    (ChannelKind::Dm, Some(u)) => user_avatar_url(u, 32., false),
                    _ => c
                        .icon
                        .as_ref()
                        .map(|h| {
                            format!(
                                "{}/channel-icons/{}/{h}.webp?size=64",
                                rcord_model::cdn::CDN,
                                c.id
                            )
                        })
                        .unwrap_or_else(|| rcord_model::cdn::default_avatar(c.id.get() % 6)),
                };
                let status = (c.kind == ChannelKind::Dm)
                    .then(|| other.map(|u| s.store.status(u)))
                    .flatten();
                let subtitle = if c.kind == ChannelKind::GroupDm {
                    Some(format!("{} Members", c.recipient_ids().count() + 1))
                } else {
                    other
                        .and_then(|u| s.store.presence(u))
                        .and_then(|p| p.custom_status().or(p.primary_activity()))
                        .map(|a| a.summary())
                        .filter(|s| !s.is_empty())
                };
                (
                    c.id,
                    s.store.channel_name(c.id),
                    url,
                    status,
                    subtitle,
                    s.channel == Some(c.id),
                    s.store.is_unread(c.id),
                    s.store.read_state(c.id).mentions,
                )
            })
            .collect();
        for (id, name, url, status, subtitle, selected, unread, mentions) in rows {
            let state = self.state.clone();
            out.push(
                div()
                    .id(snowflake_id("dm", id))
                    .mx(px(8.))
                    .px(px(8.))
                    .h(px(44.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .rounded(t.radius_small)
                    .cursor_pointer()
                    .when(selected, |el| el.bg(t.selected))
                    .when(!selected, |el| el.hover(|s| s.bg(t.hover)))
                    .child(avatar(
                        url,
                        px(32.),
                        status.map(|st| (st, &t, if selected { t.selected } else { t.sidebar })),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(if unread {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::MEDIUM
                                    })
                                    .text_color(if selected || unread {
                                        t.header
                                    } else {
                                        t.text_muted
                                    })
                                    .child(name),
                            )
                            .when_some(subtitle, |el, sub| {
                                el.child(
                                    div()
                                        .truncate()
                                        .text_xs()
                                        .text_color(t.text_faint)
                                        .child(sub),
                                )
                            }),
                    )
                    .when(mentions > 0, |el| el.child(mention_badge(mentions, &t)))
                    .on_click(move |_, _, cx| state.update(cx, |s, cx| s.select_channel(id, cx)))
                    .into_any_element(),
            );
        }
        out
    }

    /// Shown while this account sits in a voice channel, from this or another device.
    fn voice_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = theme(cx).clone();
        let s = self.state.read(cx);
        let v = s.store.my_voice_state()?;
        let channel = v.channel_id?;
        let name = s.store.channel_name(channel);
        let place = v.guild_id.and_then(|g| s.store.guild(g)).map_or_else(
            || "Direct call".to_owned(),
            |g| format!("{name} / {}", g.guild.name),
        );
        let state = self.state.clone();
        Some(
            div()
                .flex_none()
                .px(px(10.))
                .py(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .bg(Theme::panel_footer(&t))
                .border_b_1()
                .border_color(t.border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.online)
                                .child("Voice Connected"),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(t.text_muted)
                                .child(place),
                        ),
                )
                .child(
                    icon_button("voice-leave", Icon::PhoneOff, &t)
                        .tooltip(|_, cx| Tooltip::view("Disconnect", cx))
                        .on_click(move |_, _, cx| state.update(cx, |s, cx| s.leave_voice(cx))),
                )
                .into_any_element(),
        )
    }

    fn user_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let s = self.state.read(cx);
        let Some(me) = s.store.me.clone() else {
            return div().into_any_element();
        };
        let status = s.status;
        let mute = s.self_mute;
        let deaf = s.self_deaf;
        let connection = s.connection;
        let custom = s
            .store
            .presence(me.id)
            .and_then(|p| p.custom_status())
            .and_then(|a| a.state.clone());
        let state = self.state.clone();
        let state2 = self.state.clone();
        let state3 = self.state.clone();
        let state4 = self.state.clone();
        let me_id = me.id;
        let subtitle = match connection {
            Connection::Reconnecting | Connection::Connecting => "Reconnecting…".to_owned(),
            _ => custom.unwrap_or_else(|| status.label().to_owned()),
        };
        div()
            .flex_none()
            .h(px(56.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .bg(Theme::panel_footer(&t))
            .child(
                div()
                    .id("me")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .p(px(4.))
                    .rounded(t.radius_small)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .child(avatar(
                        user_avatar_url(&me, 32., false),
                        px(32.),
                        Some((status, &t, Theme::panel_footer(&t))),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.header)
                                    .child(me.display_name().to_owned()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(t.text_faint)
                                    .child(subtitle),
                            ),
                    )
                    .on_click(move |e, _, cx| {
                        let at = e.position();
                        state4.update(cx, |s, cx| {
                            let items =
                                [Status::Online, Status::Idle, Status::Dnd, Status::Invisible]
                                    .into_iter()
                                    .map(|st| {
                                        let st_state = cx.entity();
                                        MenuItem {
                                            label: st.label().into(),
                                            danger: false,
                                            action: std::rc::Rc::new(move |cx| {
                                                st_state.update(cx, |s, cx| s.set_status(st, cx))
                                            }),
                                        }
                                    })
                                    .chain(std::iter::once({
                                        let p = cx.entity();
                                        MenuItem {
                                            label: "View profile".into(),
                                            danger: false,
                                            action: std::rc::Rc::new(move |cx| {
                                                p.update(cx, |s, cx| {
                                                    s.open_profile(me_id, None, at, cx)
                                                })
                                            }),
                                        }
                                    }))
                                    .collect();
                            s.set_overlay(Some(Overlay::Menu { at, items }), cx)
                        })
                    }),
            )
            .child(
                icon_button("mute", if mute { Icon::MicOff } else { Icon::Mic }, &t)
                    .when(mute, |el| el.text_color(t.danger))
                    .tooltip(move |_, cx| Tooltip::view(if mute { "Unmute" } else { "Mute" }, cx))
                    .on_click(move |_, _, cx| {
                        state.update(cx, |s, cx| {
                            let deaf = s.self_deaf && s.self_mute;
                            s.set_mute(!s.self_mute, if deaf { false } else { s.self_deaf }, cx)
                        })
                    }),
            )
            .child(
                icon_button(
                    "deafen",
                    if deaf {
                        Icon::HeadphonesOff
                    } else {
                        Icon::Headphones
                    },
                    &t,
                )
                .when(deaf, |el| el.text_color(t.danger))
                .tooltip(move |_, cx| Tooltip::view(if deaf { "Undeafen" } else { "Deafen" }, cx))
                .on_click(move |_, _, cx| {
                    state2.update(cx, |s, cx| s.set_mute(s.self_mute, !s.self_deaf, cx))
                }),
            )
            .child(
                icon_button("settings", Icon::Gear, &t)
                    .tooltip(|_, cx| Tooltip::view("User Settings", cx))
                    .on_click(move |_, _, cx| {
                        state3.update(cx, |s, cx| {
                            s.set_overlay(Some(Overlay::Settings(SettingsPage::Account)), cx)
                        })
                    }),
            )
            .into_any_element()
    }
}

impl Theme {
    pub fn panel_footer(t: &Theme) -> gpui::Hsla {
        gpui::Hsla {
            l: (t.sidebar.l - 0.025).max(0.0),
            ..t.sidebar
        }
    }
}

impl Render for ChannelSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let s = self.state.read(cx);
        let guild = s.guild;
        let header = match guild.and_then(|g| s.store.guild(g)) {
            Some(g) => g.guild.name.clone(),
            None => "Find or start a conversation".to_owned(),
        };
        let is_home = guild.is_none();
        let rows = match guild {
            Some(g) => self.guild_list(g, cx),
            None => self.dm_list(cx),
        };
        let voice = self.voice_bar(cx);
        let footer = self.user_panel(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.sidebar)
            .child(
                div()
                    .flex_none()
                    .h(px(48.))
                    .px(px(if is_home { 10. } else { 16. }))
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(t.border)
                    .child(if is_home {
                        div()
                            .w_full()
                            .h(px(28.))
                            .px(px(8.))
                            .rounded(t.radius_small)
                            .bg(t.base)
                            .flex()
                            .items_center()
                            .text_xs()
                            .text_color(t.text_faint)
                            .child(header)
                            .into_any_element()
                    } else {
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::BOLD)
                            .text_color(t.header)
                            .child(header)
                            .into_any_element()
                    }),
            )
            .child(
                div()
                    .id("channel-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .pt(px(8.))
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .children(rows),
            )
            .children(voice)
            .child(footer)
    }
}
