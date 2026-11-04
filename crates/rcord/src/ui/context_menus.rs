//! The right-click menus for channels and servers, laid out like Discord's. They are drawn
//! from live state each frame, so checkmarks and mute labels follow changes while open.
//! Submenus open in place, with a Back row, and naming a new channel happens in a text box
//! inside the menu.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ClipboardItem, Entity, Focusable, FontWeight, IntoElement, Pixels, Point,
    SharedString, Window, anchored, deferred, div, prelude::*, px,
};
use rcord_model::{ChannelKind, NotifyLevel, Permissions, Snowflake, cdn};

use crate::guild_actions::MUTE_LENGTHS;
use crate::state::{AppState, ChannelMenuPage, GuildMenuPage, Overlay};
use crate::theme::{Theme, theme};
use crate::ui::icons::{Icon, icon};
use crate::ui::text_input::TextInput;
use crate::ui::user_menu::{item, note, separator};

type Action = Rc<dyn Fn(&mut Window, &mut App)>;
type StateFn = Rc<dyn Fn(&mut AppState, &mut gpui::Context<AppState>)>;

fn frame(t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id("context-menu")
        .occlude()
        .w(px(220.))
        .p(px(6.))
        .flex()
        .flex_col()
        .rounded(t.radius_small)
        .bg(t.floating)
        .border_1()
        .border_color(t.border)
        .shadow_lg()
}

fn place(at: Point<Pixels>, col: impl IntoElement) -> AnyElement {
    deferred(
        anchored()
            .position(at)
            .snap_to_window_with_margin(px(8.))
            .child(col),
    )
    .with_priority(2)
    .into_any_element()
}

/// A row that opens a submenu: its current value under the label and a chevron.
fn sub_item(n: usize, label: &str, hint: Option<String>, t: &Theme, action: Action) -> AnyElement {
    div()
        .id(("context-sub", n))
        .px(px(8.))
        .min_h(px(32.))
        .py(px(4.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(4.))
        .cursor_pointer()
        .text_color(t.text)
        .hover(|s| s.bg(t.accent).text_color(gpui::white()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(SharedString::from(label.to_owned())),
                )
                .when_some(hint, |el, h| {
                    el.child(div().text_xs().opacity(0.7).child(SharedString::from(h)))
                }),
        )
        .child(icon(Icon::ChevronRight).size(px(16.)))
        .on_click(move |_, window, cx| action(window, cx))
        .into_any_element()
}

/// A greyed-out row, for something that does not apply right now.
fn disabled(label: &str, t: &Theme) -> AnyElement {
    div()
        .px(px(8.))
        .h(px(32.))
        .flex()
        .items_center()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.text_faint)
        .child(SharedString::from(label.to_owned()))
        .into_any_element()
}

fn header(text: &str, t: &Theme) -> AnyElement {
    div()
        .px(px(8.))
        .pt(px(4.))
        .pb(px(2.))
        .text_xs()
        .font_weight(FontWeight::BOLD)
        .text_color(t.text_faint)
        .child(SharedString::from(text.to_uppercase()))
        .into_any_element()
}

fn input_box(input: &Entity<TextInput>, t: &Theme) -> AnyElement {
    div()
        .mx(px(4.))
        .my(px(4.))
        .px(px(8.))
        .h(px(34.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .bg(t.input)
        .text_sm()
        .child(input.clone())
        .into_any_element()
}

/// How long a mute lasts, for the label under Unmute.
fn mute_hint(settings_end: Option<&str>) -> Option<String> {
    let end = settings_end?;
    let end =
        time::OffsetDateTime::parse(end, &time::format_description::well_known::Rfc3339).ok()?;
    let left = end - time::OffsetDateTime::now_utc();
    let mins = left.whole_minutes().max(1);
    Some(if mins < 60 {
        format!("Muted for {mins} more min")
    } else if mins < 60 * 24 {
        format!("Muted for {} more h", (mins + 59) / 60)
    } else {
        format!("Muted for {} more days", (mins + 60 * 24 - 1) / (60 * 24))
    })
}

struct Helpers {
    close: Rc<dyn Fn(&mut App)>,
    act: Rc<dyn Fn(StateFn) -> Action>,
}

fn helpers(state: &Entity<AppState>) -> Helpers {
    let close: Rc<dyn Fn(&mut App)> = {
        let state = state.clone();
        Rc::new(move |cx| state.update(cx, |s, cx| s.set_overlay(None, cx)))
    };
    let act: Rc<dyn Fn(StateFn) -> Action> = {
        let state = state.clone();
        let close = close.clone();
        Rc::new(move |f: StateFn| -> Action {
            let state = state.clone();
            let close = close.clone();
            Rc::new(move |_, cx| {
                close(cx);
                let f = f.clone();
                state.update(cx, |s, cx| f(s, cx));
            })
        })
    };
    Helpers { close, act }
}

fn copy(text: String, close: Rc<dyn Fn(&mut App)>) -> Action {
    Rc::new(move |_, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        close(cx);
    })
}

pub fn channel_menu(
    state: &Entity<AppState>,
    input: &Entity<TextInput>,
    channel: Snowflake,
    at: Point<Pixels>,
    page: ChannelMenuPage,
    cx: &mut App,
) -> AnyElement {
    let t = theme(cx).clone();
    let h = helpers(state);
    let go = {
        let state = state.clone();
        let input = input.clone();
        move |page: ChannelMenuPage, prefill: Option<String>| -> Action {
            let state = state.clone();
            let input = input.clone();
            Rc::new(move |window, cx| {
                if let Some(text) = &prefill {
                    input.update(cx, |i, cx| i.set_text(text.clone(), cx));
                    window.focus(&input.focus_handle(cx));
                }
                state.update(cx, |s, cx| {
                    s.set_overlay(Some(Overlay::ChannelMenu { channel, at, page }), cx)
                })
            })
        }
    };
    let s = state.read(cx);
    let Some(c) = s.store.channel(channel) else {
        return div().into_any_element();
    };
    let guild = c.guild_id;
    let name = c.name.clone().unwrap_or_default();
    let category = c.kind == ChannelKind::Category;
    let perms = s.store.permissions(channel);
    let manage = perms.contains(Permissions::MANAGE_CHANNELS);
    let unread = s.store.is_unread(channel) || s.store.read_state(channel).mentions > 0;
    let gs = s.store.guild_settings(guild);
    let muted = s.store.is_channel_muted(channel);
    let mute_end = gs
        .and_then(|g| g.channel(channel))
        .and_then(|o| o.mute_config.as_ref())
        .and_then(|m| m.end_time.clone());
    let level = s.store.channel_notify_level(channel);
    let server_level = guild.map(|g| s.store.guild_notify_level(g));
    let developer = s.settings.advanced.developer_mode;
    let parent = if category { Some(channel) } else { c.parent_id };
    let mut n = 0usize;
    let mut next = || {
        n += 1;
        n
    };
    let mut col = frame(&t);
    let back =
        |label: &str,
         page: ChannelMenuPage,
         go: &dyn Fn(ChannelMenuPage, Option<String>) -> Action,
         n: usize| { item(n, &format!("‹ {label}"), false, None, &t, go(page, None)) };
    match page {
        ChannelMenuPage::Main => {
            if unread {
                col = col.child(item(
                    next(),
                    "Mark As Read",
                    false,
                    None,
                    &t,
                    (h.act)(Rc::new(move |s, cx| s.ack(channel, cx))),
                ));
            } else {
                col = col.child(disabled("Mark As Read", &t));
            }
            col = col.child(separator(&t));
            if !category && perms.contains(Permissions::CREATE_INSTANT_INVITE) {
                col = col.child(item(
                    next(),
                    "Invite to Channel",
                    false,
                    None,
                    &t,
                    (h.act)(Rc::new(move |s, cx| s.copy_invite(channel, cx))),
                ));
            }
            if let Some(g) = guild
                && !category
            {
                col = col.child(item(
                    next(),
                    "Copy Link",
                    false,
                    None,
                    &t,
                    copy(
                        format!("https://discord.com/channels/{g}/{channel}"),
                        h.close.clone(),
                    ),
                ));
            }
            col = col.child(separator(&t));
            if muted {
                col = col.child(sub_item(
                    next(),
                    if category {
                        "Unmute Category"
                    } else {
                        "Unmute Channel"
                    },
                    mute_hint(mute_end.as_deref()),
                    &t,
                    (h.act)(Rc::new(move |s, cx| s.unmute_channel(channel, cx))),
                ));
            } else {
                col = col.child(sub_item(
                    next(),
                    if category {
                        "Mute Category"
                    } else {
                        "Mute Channel"
                    },
                    None,
                    &t,
                    go(ChannelMenuPage::Mute, None),
                ));
            }
            let shown = match level {
                NotifyLevel::Inherit => {
                    server_level.map(|l| format!("Server default: {}", l.label()))
                }
                l => Some(l.label().to_owned()),
            };
            col = col.child(sub_item(
                next(),
                "Notification Settings",
                shown,
                &t,
                go(ChannelMenuPage::Notifications, None),
            ));
            if manage && guild.is_some() {
                col = col.child(separator(&t));
                col = col.child(item(
                    next(),
                    if category {
                        "Rename Category"
                    } else {
                        "Edit Channel"
                    },
                    false,
                    None,
                    &t,
                    go(ChannelMenuPage::Rename, Some(name.clone())),
                ));
                if !category {
                    col = col.child(item(
                        next(),
                        "Duplicate Channel",
                        false,
                        None,
                        &t,
                        (h.act)(Rc::new(move |s, cx| s.duplicate_channel(channel, cx))),
                    ));
                }
                col = col.child(item(
                    next(),
                    "Create Text Channel",
                    false,
                    None,
                    &t,
                    go(ChannelMenuPage::CreateText, Some(String::new())),
                ));
                col = col.child(item(
                    next(),
                    if category {
                        "Delete Category"
                    } else {
                        "Delete Channel"
                    },
                    true,
                    None,
                    &t,
                    go(ChannelMenuPage::ConfirmDelete, None),
                ));
            }
            if developer {
                col = col.child(separator(&t)).child(item(
                    next(),
                    "Copy Channel ID",
                    false,
                    None,
                    &t,
                    copy(channel.to_string(), h.close.clone()),
                ));
            }
        }
        ChannelMenuPage::Mute => {
            col = col.child(back("Back", ChannelMenuPage::Main, &go, next()));
            for (label, minutes) in MUTE_LENGTHS {
                col = col.child(item(
                    next(),
                    label,
                    false,
                    None,
                    &t,
                    (h.act)(Rc::new(move |s, cx| s.mute_channel(channel, minutes, cx))),
                ));
            }
        }
        ChannelMenuPage::Notifications => {
            col = col.child(back("Back", ChannelMenuPage::Main, &go, next()));
            for l in [
                NotifyLevel::Inherit,
                NotifyLevel::All,
                NotifyLevel::Mentions,
                NotifyLevel::Nothing,
            ] {
                let label = match (l, server_level) {
                    (NotifyLevel::Inherit, Some(sl)) => {
                        format!("Use Server Default ({})", sl.label())
                    }
                    _ => l.label().to_owned(),
                };
                col = col.child(item(
                    next(),
                    &label,
                    false,
                    Some(level == l),
                    &t,
                    (h.act)(Rc::new(move |s, cx| s.set_channel_notify(channel, l, cx))),
                ));
            }
        }
        ChannelMenuPage::Rename | ChannelMenuPage::CreateText => {
            let rename = page == ChannelMenuPage::Rename;
            col = col
                .child(back("Back", ChannelMenuPage::Main, &go, next()))
                .child(header(
                    if rename {
                        "Channel name"
                    } else {
                        "New text channel"
                    },
                    &t,
                ))
                .child(input_box(input, &t));
            let input = input.clone();
            let submit: StateFn = Rc::new(move |s, cx| {
                let text = input.read(cx).text().to_owned();
                match (rename, guild) {
                    (true, _) => s.rename_channel(channel, text, cx),
                    (false, Some(g)) => s.create_channel(g, text, false, parent, cx),
                    _ => {}
                }
            });
            col = col.child(item(
                next(),
                if rename { "Save" } else { "Create Channel" },
                false,
                None,
                &t,
                (h.act)(submit),
            ));
        }
        ChannelMenuPage::ConfirmDelete => {
            col = col
                .child(note(
                    &format!(
                        "Delete {}{name}? This cannot be undone.",
                        if category { "" } else { "#" }
                    ),
                    &t,
                ))
                .child(item(
                    next(),
                    "Delete",
                    true,
                    None,
                    &t,
                    (h.act)(Rc::new(move |s, cx| s.delete_channel(channel, cx))),
                ))
                .child(item(
                    next(),
                    "Cancel",
                    false,
                    None,
                    &t,
                    go(ChannelMenuPage::Main, None),
                ));
        }
    }
    place(at, col)
}

pub fn guild_menu(
    state: &Entity<AppState>,
    input: &Entity<TextInput>,
    guild: Snowflake,
    at: Point<Pixels>,
    page: GuildMenuPage,
    cx: &mut App,
) -> AnyElement {
    let t = theme(cx).clone();
    let h = helpers(state);
    let go = {
        let state = state.clone();
        let input = input.clone();
        move |page: GuildMenuPage, prefill: Option<String>| -> Action {
            let state = state.clone();
            let input = input.clone();
            Rc::new(move |window, cx| {
                if let Some(text) = &prefill {
                    input.update(cx, |i, cx| i.set_text(text.clone(), cx));
                    window.focus(&input.focus_handle(cx));
                }
                state.update(cx, |s, cx| {
                    s.set_overlay(Some(Overlay::GuildMenu { guild, at, page }), cx)
                })
            })
        }
    };
    let s = state.read(cx);
    let Some(entry) = s.store.guild(guild) else {
        return div().into_any_element();
    };
    let name = entry.guild.name.clone();
    let icon_url = cdn::guild_icon(&entry.guild, 512, true);
    let me = s.store.me_id();
    let owner = me.is_some() && entry.guild.owner_id == me;
    let (unread, mentions) = s.store.guild_unread(guild);
    let invite_to = s.invite_channel(guild);
    let can_invite = invite_to.is_some_and(|c| {
        s.store
            .permissions(c)
            .contains(Permissions::CREATE_INSTANT_INVITE)
    });
    let manage = s
        .store
        .guild_permissions(guild)
        .contains(Permissions::MANAGE_CHANNELS);
    let muted = s.store.is_guild_muted(guild);
    let mute_end = s
        .store
        .guild_settings(Some(guild))
        .and_then(|g| g.mute_config.as_ref())
        .and_then(|m| m.end_time.clone());
    let level = s.store.guild_notify_level(guild);
    let hide_muted = s.store.hides_muted_channels(guild);
    let order = s.ordered_guilds();
    let position = order.iter().position(|g| *g == guild).unwrap_or(0);
    let developer = s.settings.advanced.developer_mode;
    let mut n = 0usize;
    let mut next = || {
        n += 1;
        n
    };
    let mut col = frame(&t);
    let back = |page: GuildMenuPage,
                go: &dyn Fn(GuildMenuPage, Option<String>) -> Action,
                n: usize| { item(n, "‹ Back", false, None, &t, go(page, None)) };
    match page {
        GuildMenuPage::Main => {
            if unread || mentions > 0 {
                col = col.child(item(
                    next(),
                    "Mark As Read",
                    false,
                    None,
                    &t,
                    (h.act)(Rc::new(move |s, cx| s.mark_guild_read(guild, cx))),
                ));
            } else {
                col = col.child(disabled("Mark As Read", &t));
            }
            if can_invite && let Some(c) = invite_to {
                col = col.child(separator(&t)).child(item(
                    next(),
                    "Invite to Server",
                    false,
                    None,
                    &t,
                    (h.act)(Rc::new(move |s, cx| s.copy_invite(c, cx))),
                ));
            }
            col = col.child(separator(&t));
            if muted {
                col = col.child(sub_item(
                    next(),
                    "Unmute Server",
                    mute_hint(mute_end.as_deref()),
                    &t,
                    (h.act)(Rc::new(move |s, cx| s.unmute_guild(guild, cx))),
                ));
            } else {
                col = col.child(sub_item(
                    next(),
                    "Mute Server",
                    None,
                    &t,
                    go(GuildMenuPage::Mute, None),
                ));
            }
            col = col
                .child(sub_item(
                    next(),
                    "Notification Settings",
                    Some(level.label().to_owned()),
                    &t,
                    go(GuildMenuPage::Notifications, None),
                ))
                .child(item(
                    next(),
                    "Hide Muted Channels",
                    false,
                    Some(hide_muted),
                    &t,
                    {
                        let state = state.clone();
                        Rc::new(move |_, cx| {
                            state.update(cx, |s, cx| s.set_hide_muted(guild, !hide_muted, cx))
                        })
                    },
                ));
            if manage {
                col = col
                    .child(separator(&t))
                    .child(item(
                        next(),
                        "Create Channel",
                        false,
                        None,
                        &t,
                        go(GuildMenuPage::CreateText, Some(String::new())),
                    ))
                    .child(item(
                        next(),
                        "Create Category",
                        false,
                        None,
                        &t,
                        go(GuildMenuPage::CreateCategory, Some(String::new())),
                    ));
            }
            col = col
                .child(separator(&t))
                .child(sub_item(
                    next(),
                    "Move to…",
                    None,
                    &t,
                    go(GuildMenuPage::MoveTo, None),
                ))
                .child(sub_item(
                    next(),
                    "Copy Server Info",
                    None,
                    &t,
                    go(GuildMenuPage::CopyInfo, None),
                ));
            if !owner {
                col = col.child(separator(&t)).child(item(
                    next(),
                    "Leave Server",
                    true,
                    None,
                    &t,
                    go(GuildMenuPage::ConfirmLeave, None),
                ));
            }
        }
        GuildMenuPage::Mute => {
            col = col.child(back(GuildMenuPage::Main, &go, next()));
            for (label, minutes) in MUTE_LENGTHS {
                col = col.child(item(
                    next(),
                    label,
                    false,
                    None,
                    &t,
                    (h.act)(Rc::new(move |s, cx| s.mute_guild(guild, minutes, cx))),
                ));
            }
        }
        GuildMenuPage::Notifications => {
            col = col.child(back(GuildMenuPage::Main, &go, next()));
            for l in [
                NotifyLevel::All,
                NotifyLevel::Mentions,
                NotifyLevel::Nothing,
            ] {
                col = col.child(item(
                    next(),
                    l.label(),
                    false,
                    Some(level == l),
                    &t,
                    (h.act)(Rc::new(move |s, cx| s.set_guild_notify(guild, l, cx))),
                ));
            }
        }
        GuildMenuPage::CreateText | GuildMenuPage::CreateCategory => {
            let category = page == GuildMenuPage::CreateCategory;
            col = col
                .child(back(GuildMenuPage::Main, &go, next()))
                .child(header(
                    if category {
                        "New category"
                    } else {
                        "New text channel"
                    },
                    &t,
                ))
                .child(input_box(input, &t));
            let input = input.clone();
            col = col.child(item(
                next(),
                if category {
                    "Create Category"
                } else {
                    "Create Channel"
                },
                false,
                None,
                &t,
                (h.act)(Rc::new(move |s, cx| {
                    let text = input.read(cx).text().to_owned();
                    s.create_channel(guild, text, category, None, cx)
                })),
            ));
        }
        GuildMenuPage::MoveTo => {
            col = col.child(back(GuildMenuPage::Main, &go, next()));
            let last = order.len().saturating_sub(1);
            let moves: [(&str, Option<usize>); 4] = [
                ("To the Top", (position > 0).then_some(0)),
                ("Up One", position.checked_sub(1)),
                ("Down One", (position < last).then_some(position + 1)),
                ("To the Bottom", (position < last).then_some(last)),
            ];
            for (label, to) in moves {
                col = col.child(match to {
                    Some(to) => item(
                        next(),
                        label,
                        false,
                        None,
                        &t,
                        (h.act)(Rc::new(move |s, cx| s.move_guild_to(guild, to, cx))),
                    ),
                    None => disabled(label, &t),
                });
            }
        }
        GuildMenuPage::CopyInfo => {
            col = col
                .child(back(GuildMenuPage::Main, &go, next()))
                .child(item(
                    next(),
                    "Copy Server Name",
                    false,
                    None,
                    &t,
                    copy(name.clone(), h.close.clone()),
                ))
                .child(item(
                    next(),
                    "Copy Server ID",
                    false,
                    None,
                    &t,
                    copy(guild.to_string(), h.close.clone()),
                ));
            if let Some(url) = icon_url {
                col = col.child(item(
                    next(),
                    "Copy Icon Link",
                    false,
                    None,
                    &t,
                    copy(url, h.close.clone()),
                ));
            }
            let _ = developer;
        }
        GuildMenuPage::ConfirmLeave => {
            col = col
                .child(note(
                    &format!("Leave {name}? You can't rejoin without a new invite."),
                    &t,
                ))
                .child(item(
                    next(),
                    "Leave Server",
                    true,
                    None,
                    &t,
                    (h.act)(Rc::new(move |s, cx| s.leave_guild(guild, cx))),
                ))
                .child(item(
                    next(),
                    "Cancel",
                    false,
                    None,
                    &t,
                    go(GuildMenuPage::Main, None),
                ));
        }
    }
    place(at, col)
}
