//! The menu for someone in a voice channel, opened by right-clicking them: their profile and
//! DMs, how loud they play for you, their stream, and the moderator actions your
//! permissions allow. It is drawn from live state, so its checkboxes and volume follow
//! changes while it is open.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ClipboardItem, Entity, FontWeight, IntoElement, Pixels, Point, SharedString,
    Window, anchored, deferred, div, prelude::*, px,
};
use rcord_model::{Permissions, RelationshipKind, Snowflake};
use rcord_rest::routes::MemberVoiceChange;

use crate::state::{AppState, Overlay, UserMenuPage};
use crate::theme::{Theme, theme};
use crate::ui::chat::ChatView;
use crate::ui::icons::{Icon, icon};
use crate::ui::volume::volume_bar;

type Action = Rc<dyn Fn(&mut Window, &mut App)>;

/// Everything the menu shows, read once per frame.
struct Facts {
    me: bool,
    guild: Option<Snowflake>,
    name: String,
    volume: f32,
    local_mute: bool,
    live: bool,
    watching: bool,
    friend: Option<RelationshipKind>,
    server_mute: bool,
    server_deaf: bool,
    perms: Permissions,
    can_mention: bool,
    developer: bool,
}

pub fn user_menu(
    state: &Entity<AppState>,
    chat: &Entity<ChatView>,
    user: Snowflake,
    channel: Snowflake,
    at: Point<Pixels>,
    page: UserMenuPage,
    cx: &mut App,
) -> AnyElement {
    let t = theme(cx).clone();
    let f = {
        let s = state.read(cx);
        let guild = s.store.channel(channel).and_then(|c| c.guild_id);
        let vs = s.store.voice_state(user);
        let id = user.to_string();
        Facts {
            me: s.store.me_id() == Some(user),
            guild,
            name: s
                .store
                .display_name(guild, user)
                .map(str::to_owned)
                .or_else(|| s.store.user(user).map(|u| u.display_name().to_owned()))
                .unwrap_or_else(|| "this person".into()),
            volume: s
                .settings
                .voice
                .user_volumes
                .get(&id)
                .copied()
                .unwrap_or(1.0),
            local_mute: s.settings.voice.muted_users.contains(&id),
            live: vs.is_some_and(|v| v.self_stream),
            watching: s.is_watching(user),
            friend: s
                .store
                .relationships()
                .find(|r| r.id == user)
                .map(|r| r.kind),
            server_mute: vs.is_some_and(|v| v.mute),
            server_deaf: vs.is_some_and(|v| v.deaf),
            perms: if guild.is_some() {
                s.store.permissions(channel)
            } else {
                Permissions::empty()
            },
            can_mention: s.channel.is_some(),
            developer: s.settings.advanced.developer_mode,
        }
    };

    let close = {
        let state = state.clone();
        move |cx: &mut App| state.update(cx, |s, cx| s.set_overlay(None, cx))
    };
    let go = {
        let state = state.clone();
        move |page: UserMenuPage| -> Action {
            let state = state.clone();
            Rc::new(move |_, cx| {
                state.update(cx, |s, cx| {
                    s.set_overlay(
                        Some(Overlay::UserMenu {
                            user,
                            channel,
                            at,
                            page,
                        }),
                        cx,
                    )
                })
            })
        }
    };
    let act = {
        let state = state.clone();
        let close = close.clone();
        move |f: Rc<dyn Fn(&mut AppState, &mut gpui::Context<AppState>)>| -> Action {
            let state = state.clone();
            let close = close.clone();
            Rc::new(move |_, cx| {
                close(cx);
                let f = f.clone();
                state.update(cx, |s, cx| f(s, cx));
            })
        }
    };

    let mut col = div()
        .id("user-menu")
        .occlude()
        .w(px(220.))
        .p(px(6.))
        .flex()
        .flex_col()
        .rounded(t.radius_small)
        .bg(t.floating)
        .border_1()
        .border_color(t.border)
        .shadow_lg();
    let mut n = 0usize;
    let mut next = || {
        n += 1;
        n
    };

    match page {
        UserMenuPage::Main => {
            col = col.child(item(next(), "Profile", false, None, &t, {
                let state = state.clone();
                let close = close.clone();
                Rc::new(move |_, cx| {
                    close(cx);
                    state.update(cx, |s, cx| {
                        let g = s.store.channel(channel).and_then(|c| c.guild_id);
                        s.open_profile(user, g, at, cx)
                    })
                })
            }));
            if f.can_mention {
                let chat = chat.clone();
                let close = close.clone();
                col = col.child(item(
                    next(),
                    "Mention",
                    false,
                    None,
                    &t,
                    Rc::new(move |window, cx| {
                        close(cx);
                        let text = format!("<@{user}> ");
                        chat.update(cx, |c, cx| c.insert_into_composer(&text, window, cx));
                    }),
                ));
            }
            if !f.me {
                col = col.child(item(
                    next(),
                    "Message",
                    false,
                    None,
                    &t,
                    act(Rc::new(move |s, cx| s.open_dm(user, cx))),
                ));
                col = col.child(separator(&t));
                col = col.child(volume_slider(state, user, f.volume, &t));
                col = col.child(item(next(), "Mute", false, Some(f.local_mute), &t, {
                    let state = state.clone();
                    let muted = f.local_mute;
                    Rc::new(move |_, cx| {
                        state.update(cx, |s, cx| s.set_local_mute(user, !muted, cx))
                    })
                }));
            }
            if f.live && !f.me {
                let watching = f.watching;
                col = col.child(item(
                    next(),
                    if watching {
                        "Stop Watching"
                    } else {
                        "Watch Stream"
                    },
                    false,
                    None,
                    &t,
                    act(Rc::new(move |s, cx| {
                        if watching {
                            s.stop_watching(user, cx)
                        } else {
                            s.watch_stream(channel, user, cx)
                        }
                    })),
                ));
            }
            if !f.me {
                let friend = matches!(f.friend, Some(RelationshipKind::Friend));
                let pending = matches!(f.friend, Some(RelationshipKind::OutgoingRequest));
                let blocked = matches!(f.friend, Some(RelationshipKind::Blocked));
                if !pending && !blocked {
                    col = col.child(separator(&t)).child(item(
                        next(),
                        if friend {
                            "Remove Friend"
                        } else if f.friend == Some(RelationshipKind::IncomingRequest) {
                            "Accept Friend Request"
                        } else {
                            "Add Friend"
                        },
                        false,
                        None,
                        &t,
                        act(Rc::new(move |s, cx| s.set_friend(user, !friend, cx))),
                    ));
                }
            }

            if let Some(guild) = f.guild {
                let mute = f.perms.contains(Permissions::MUTE_MEMBERS);
                let deafen = f.perms.contains(Permissions::DEAFEN_MEMBERS);
                let mv = f.perms.contains(Permissions::MOVE_MEMBERS);
                let kick = !f.me && f.perms.contains(Permissions::KICK_MEMBERS);
                let ban = !f.me && f.perms.contains(Permissions::BAN_MEMBERS);
                if mute || deafen || mv || kick || ban {
                    col = col.child(separator(&t));
                }
                if mv {
                    col = col.child(item(
                        next(),
                        "Move To",
                        false,
                        None,
                        &t,
                        go(UserMenuPage::MoveTo),
                    ));
                }
                if mute {
                    let on = f.server_mute;
                    col = col.child(item(next(), "Server Mute", true, Some(on), &t, {
                        let state = state.clone();
                        Rc::new(move |_, cx| {
                            state.update(cx, |s, cx| {
                                s.moderate_voice(
                                    guild,
                                    user,
                                    MemberVoiceChange {
                                        mute: Some(!on),
                                        ..Default::default()
                                    },
                                    cx,
                                )
                            })
                        })
                    }));
                }
                if deafen {
                    let on = f.server_deaf;
                    col = col.child(item(next(), "Server Deafen", true, Some(on), &t, {
                        let state = state.clone();
                        Rc::new(move |_, cx| {
                            state.update(cx, |s, cx| {
                                s.moderate_voice(
                                    guild,
                                    user,
                                    MemberVoiceChange {
                                        deaf: Some(!on),
                                        ..Default::default()
                                    },
                                    cx,
                                )
                            })
                        })
                    }));
                }
                if mv {
                    col = col.child(item(
                        next(),
                        "Disconnect",
                        true,
                        None,
                        &t,
                        act(Rc::new(move |s, cx| {
                            s.moderate_voice(
                                guild,
                                user,
                                MemberVoiceChange {
                                    channel: Some(None),
                                    ..Default::default()
                                },
                                cx,
                            )
                        })),
                    ));
                }
                if kick {
                    col = col.child(item(
                        next(),
                        &format!("Kick {}", f.name),
                        true,
                        None,
                        &t,
                        go(UserMenuPage::ConfirmKick),
                    ));
                }
                if ban {
                    col = col.child(item(
                        next(),
                        &format!("Ban {}", f.name),
                        true,
                        None,
                        &t,
                        go(UserMenuPage::ConfirmBan),
                    ));
                }
            }
            if f.developer {
                col = col.child(separator(&t)).child(item(
                    next(),
                    "Copy User ID",
                    false,
                    None,
                    &t,
                    {
                        let close = close.clone();
                        Rc::new(move |_, cx| {
                            close(cx);
                            cx.write_to_clipboard(ClipboardItem::new_string(user.to_string()));
                        })
                    },
                ));
            }
        }
        UserMenuPage::MoveTo => {
            col = col.child(item(
                next(),
                "Back",
                false,
                None,
                &t,
                go(UserMenuPage::Main),
            ));
            col = col.child(separator(&t));
            let targets: Vec<(Snowflake, String)> = {
                let s = state.read(cx);
                let current = s.store.voice_state(user).and_then(|v| v.channel_id);
                f.guild
                    .map(|g| {
                        s.store
                            .channel_groups(g)
                            .into_iter()
                            .flat_map(|grp| grp.channels)
                            .filter(|c| Some(*c) != current)
                            .filter_map(|c| {
                                let ch = s.store.channel(c)?;
                                let voice = ch.kind.is_voice();
                                (voice
                                    && s.store
                                        .permissions(c)
                                        .contains(Permissions::CONNECT | Permissions::MOVE_MEMBERS))
                                .then(|| (c, ch.name.clone().unwrap_or_default()))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            if targets.is_empty() {
                col = col.child(note("There is no other voice channel to move them to.", &t));
            }
            let guild = f.guild;
            for (target, name) in targets {
                col = col.child(item(
                    next(),
                    &name,
                    false,
                    None,
                    &t,
                    act(Rc::new(move |s, cx| {
                        if let Some(g) = guild {
                            s.moderate_voice(
                                g,
                                user,
                                MemberVoiceChange {
                                    channel: Some(Some(target)),
                                    ..Default::default()
                                },
                                cx,
                            )
                        }
                    })),
                ));
            }
        }
        UserMenuPage::ConfirmKick | UserMenuPage::ConfirmBan => {
            let ban = page == UserMenuPage::ConfirmBan;
            col = col.child(note(
                &if ban {
                    format!(
                        "Ban {} from the server? They can't rejoin until the ban is lifted.",
                        f.name
                    )
                } else {
                    format!(
                        "Kick {} from the server? They can rejoin with an invite.",
                        f.name
                    )
                },
                &t,
            ));
            let guild = f.guild;
            col = col
                .child(item(
                    next(),
                    if ban { "Ban" } else { "Kick" },
                    true,
                    None,
                    &t,
                    act(Rc::new(move |s, cx| {
                        if let Some(g) = guild {
                            if ban {
                                s.ban_member(g, user, cx)
                            } else {
                                s.kick_member(g, user, cx)
                            }
                        }
                    })),
                ))
                .child(item(
                    next(),
                    "Cancel",
                    false,
                    None,
                    &t,
                    go(UserMenuPage::Main),
                ));
        }
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

/// One row. `check` draws a checkbox on the right; rows with one stay open when clicked, as
/// in Discord.
pub(crate) fn item(
    n: usize,
    label: &str,
    danger: bool,
    check: Option<bool>,
    t: &Theme,
    action: Action,
) -> AnyElement {
    let fg = if danger { t.danger } else { t.text };
    let hover_bg = if danger { t.danger } else { t.accent };
    div()
        .id(("user-menu-item", n))
        .group("user-menu-item")
        .px(px(8.))
        .h(px(32.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .rounded(px(4.))
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .text_color(fg)
        .hover(|s| s.bg(hover_bg).text_color(gpui::white()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(SharedString::from(label.to_owned())),
        )
        .when_some(check, |el, on| {
            el.child(
                div()
                    .flex_none()
                    .size(px(18.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(if on { t.accent } else { t.text_muted })
                    .when(on, |el| el.bg(t.accent))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(on, |el| {
                        el.child(icon(Icon::Check).size(px(14.)).text_color(gpui::white()))
                    }),
            )
        })
        .on_click(move |_, window, cx| action(window, cx))
        .into_any_element()
}

pub(crate) fn separator(t: &Theme) -> AnyElement {
    div()
        .mx(px(4.))
        .my(px(4.))
        .h(px(1.))
        .bg(t.border)
        .into_any_element()
}

pub(crate) fn note(text: &str, t: &Theme) -> AnyElement {
    div()
        .px(px(8.))
        .py(px(6.))
        .text_sm()
        .text_color(t.text_muted)
        .child(SharedString::from(text.to_owned()))
        .into_any_element()
}

/// User Volume, 0 to 200%, as in Discord. Dragging updates the call as it moves.
fn volume_slider(state: &Entity<AppState>, user: Snowflake, volume: f32, t: &Theme) -> AnyElement {
    let state = state.clone();
    div()
        .px(px(8.))
        .py(px(6.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .justify_between()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.text)
                .child("User Volume")
                .child(
                    div()
                        .text_color(t.text_muted)
                        .child(format!("{:.0}%", volume * 100.)),
                ),
        )
        .child(volume_bar(
            "user-volume",
            volume,
            t,
            Rc::new(move |v, cx| state.update(cx, |s, cx| s.set_user_volume(user, v, cx))),
        ))
        .into_any_element()
}
