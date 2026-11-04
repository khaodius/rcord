//! The friends page on Home: friends by status, pending requests and blocks, adding a
//! friend by username, and an Active Now column with what friends are doing.

use gpui::{
    AnyElement, ClickEvent, Context, Entity, Focusable, FontWeight, IntoElement, ObjectFit, Render,
    SharedString, Subscription, Window, div, prelude::*, px,
};
use rcord_model::{Activity, ActivityKind, RelationshipKind, Snowflake, Status};

use crate::state::{AppState, StateEvent, now_ms};
use crate::theme::{Theme, theme};
use crate::ui::icons::{Icon, icon};
use crate::ui::text_input::{InputEvent, TextInput};
use crate::ui::{Tooltip, anim_img, avatar, section_label, snowflake_id, user_avatar_url};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Online,
    All,
    Pending,
    Blocked,
    Add,
}

impl Tab {
    const LISTS: [Self; 4] = [Self::Online, Self::All, Self::Pending, Self::Blocked];

    const fn label(self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::All => "All",
            Self::Pending => "Pending",
            Self::Blocked => "Blocked",
            Self::Add => "Add Friend",
        }
    }
}

pub struct FriendsView {
    state: Entity<AppState>,
    tab: Tab,
    search: Entity<TextInput>,
    add: Entity<TextInput>,
    /// The answer to the last friend request: the name it went to, or the error.
    add_result: Option<Result<String, String>>,
    sending: bool,
    _subs: Vec<Subscription>,
}

/// One person in the list, read out of the store for a frame.
struct Entry {
    id: Snowflake,
    name: SharedString,
    username: SharedString,
    avatar: String,
    status: Status,
    line: SharedString,
    kind: RelationshipKind,
}

/// One card in Active Now.
struct ActiveCard {
    id: Snowflake,
    name: SharedString,
    avatar: String,
    status: Status,
    headline: SharedString,
    title: SharedString,
    lines: Vec<SharedString>,
    image: Option<String>,
    icon: Icon,
}

impl FriendsView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new("Search", cx));
        let add =
            cx.new(|cx| TextInput::new("You can add friends with their Discord username.", cx));
        let subs = vec![
            cx.subscribe(&state, |_, _, event, cx| {
                if let StateEvent::Store(
                    rcord_store::Change::Relationships
                    | rcord_store::Change::Presence(_)
                    | rcord_store::Change::Voice(_)
                    | rcord_store::Change::Everything,
                ) = event
                {
                    cx.notify();
                }
            }),
            cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe_in(&add, window, |this, _, event, _, cx| match event {
                InputEvent::Submit => this.send_request(cx),
                InputEvent::Changed if this.add_result.take().is_some() => cx.notify(),
                _ => {}
            }),
        ];
        Self {
            state,
            tab: Tab::Online,
            search,
            add,
            add_result: None,
            sending: false,
            _subs: subs,
        }
    }

    fn send_request(&mut self, cx: &mut Context<Self>) {
        let name = self
            .add
            .read(cx)
            .text()
            .trim()
            .trim_start_matches('@')
            .to_owned();
        if name.is_empty() || self.sending {
            return;
        }
        self.sending = true;
        self.add_result = None;
        cx.notify();
        let this = cx.weak_entity();
        let input = self.add.clone();
        let sent = name.clone();
        self.state.update(cx, |s, cx| {
            s.send_friend_request(name, cx, move |result, cx| {
                this.update(cx, |v, cx| {
                    v.sending = false;
                    if result.is_ok() {
                        input.update(cx, |i, cx| i.clear(cx));
                    }
                    v.add_result = Some(result.map(|()| sent));
                    cx.notify();
                })
                .ok();
            })
        });
    }

    fn entries(&self, cx: &Context<Self>) -> Vec<Entry> {
        let store = &self.state.read(cx).store;
        let mut out: Vec<Entry> = store
            .relationships()
            .filter_map(|r| {
                let user = store.user(r.id)?;
                let status = match store.status(r.id) {
                    Status::Invisible => Status::Offline,
                    s => s,
                };
                let line = match r.kind {
                    RelationshipKind::IncomingRequest => "Incoming Friend Request".to_owned(),
                    RelationshipKind::OutgoingRequest => "Outgoing Friend Request".to_owned(),
                    RelationshipKind::Blocked => "Blocked".to_owned(),
                    _ => {
                        let presence = store.presence(r.id).filter(|_| status != Status::Offline);
                        presence
                            .and_then(|p| {
                                p.custom_status()
                                    .map(custom_line)
                                    .filter(|s| !s.is_empty())
                                    .or_else(|| p.primary_activity().map(Activity::summary))
                            })
                            .unwrap_or_else(|| status.label().to_owned())
                    }
                };
                Some(Entry {
                    id: r.id,
                    name: store
                        .display_name(None, r.id)
                        .unwrap_or(&user.username)
                        .to_owned()
                        .into(),
                    username: user.username.clone().into(),
                    avatar: user_avatar_url(user, 32., false),
                    status,
                    line: line.into(),
                    kind: r.kind,
                })
            })
            .collect();
        out.sort_by_cached_key(|e| e.name.to_lowercase());
        out
    }

    fn active_now(&self, cx: &Context<Self>) -> Vec<ActiveCard> {
        let store = &self.state.read(cx).store;
        let now = now_ms();
        let mut out = Vec::new();
        for r in store
            .relationships()
            .filter(|r| r.kind == RelationshipKind::Friend)
        {
            let Some(user) = store.user(r.id) else {
                continue;
            };
            let status = store.status(r.id);
            if matches!(status, Status::Offline | Status::Invisible) {
                continue;
            }
            let name: SharedString = store
                .display_name(None, r.id)
                .unwrap_or(&user.username)
                .to_owned()
                .into();
            let avatar = user_avatar_url(user, 40., false);
            if let Some(v) = store.voice_state(r.id)
                && let Some(channel) = v.channel_id.and_then(|c| store.channel(c))
            {
                let guild = v
                    .guild_id
                    .and_then(|g| store.guild(g))
                    .map(|g| g.guild.name.clone())
                    .unwrap_or_default();
                let others = store.voice_states_in(channel.id).count().saturating_sub(1);
                let mut lines = vec![SharedString::from(guild)];
                if others > 0 {
                    lines.push(
                        format!("with {others} other{}", if others == 1 { "" } else { "s" }).into(),
                    );
                }
                out.push(ActiveCard {
                    id: r.id,
                    name: name.clone(),
                    avatar: avatar.clone(),
                    status,
                    headline: "In a voice channel".into(),
                    title: channel.name.clone().unwrap_or_default().into(),
                    lines,
                    image: None,
                    icon: Icon::Speaker,
                });
            }
            let Some(activity) = store.presence(r.id).and_then(|p| p.primary_activity()) else {
                continue;
            };
            let mut lines: Vec<SharedString> = [&activity.details, &activity.state]
                .into_iter()
                .flatten()
                .filter(|l| !l.is_empty())
                .map(|l| l.clone().into())
                .collect();
            if let Some(since) = activity.timestamps.and_then(|t| t.start)
                && activity.kind != ActivityKind::Listening
            {
                lines.push(format!("for {}", elapsed(now.saturating_sub(since))).into());
            }
            out.push(ActiveCard {
                id: r.id,
                name,
                avatar,
                status,
                headline: activity.summary().into(),
                title: activity.name.clone().into(),
                lines,
                image: activity_image(activity),
                icon: match activity.kind {
                    ActivityKind::Listening => Icon::Headphones,
                    ActivityKind::Streaming | ActivityKind::Watching => Icon::Screen,
                    _ => Icon::Gamepad,
                },
            });
        }
        out.sort_by_cached_key(|c| c.name.to_lowercase());
        out
    }

    fn header(&self, entries: &[Entry], cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let pending = entries
            .iter()
            .filter(|e| e.kind == RelationshipKind::IncomingRequest)
            .count();
        let mut bar = div()
            .flex_none()
            .h(px(48.))
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_color(t.header)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(icon(Icon::Users).size(px(22.)).text_color(t.text_muted))
                    .child("Friends"),
            )
            .child(div().mx(px(8.)).w(px(1.)).h(px(24.)).bg(t.border));
        for tab in Tab::LISTS {
            let on = self.tab == tab;
            bar = bar.child(
                div()
                    .id(SharedString::from(format!("friends-tab-{}", tab.label())))
                    .px(px(10.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(t.radius_small)
                    .cursor_pointer()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if on { t.header } else { t.text_muted })
                    .when(on, |el| el.bg(t.selected))
                    .when(!on, |el| el.hover(|s| s.bg(t.hover).text_color(t.text)))
                    .child(tab.label())
                    .when(tab == Tab::Pending && pending > 0, |el| {
                        el.child(crate::ui::mention_badge(pending as u32, &t))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = tab;
                        cx.notify();
                    })),
            );
        }
        let on = self.tab == Tab::Add;
        bar.child(
            div()
                .id("friends-tab-add")
                .px(px(10.))
                .h(px(28.))
                .flex()
                .items_center()
                .rounded(t.radius_small)
                .cursor_pointer()
                .font_weight(FontWeight::MEDIUM)
                .when(on, |el| el.text_color(t.online))
                .when(!on, |el| el.bg(t.online).text_color(gpui::white()))
                .child("Add Friend")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.tab = Tab::Add;
                    let focus = this.add.focus_handle(cx);
                    window.focus(&focus);
                    cx.notify();
                })),
        )
        .into_any_element()
    }

    fn list(&self, entries: Vec<Entry>, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let query = self.search.read(cx).text().trim().to_lowercase();
        let shown: Vec<Entry> = entries
            .into_iter()
            .filter(|e| match self.tab {
                Tab::Online => e.kind == RelationshipKind::Friend && e.status != Status::Offline,
                Tab::All => e.kind == RelationshipKind::Friend,
                Tab::Pending => matches!(
                    e.kind,
                    RelationshipKind::IncomingRequest | RelationshipKind::OutgoingRequest
                ),
                Tab::Blocked => e.kind == RelationshipKind::Blocked,
                Tab::Add => false,
            })
            .filter(|e| {
                query.is_empty()
                    || e.name.to_lowercase().contains(&query)
                    || e.username.to_lowercase().contains(&query)
            })
            .collect();
        let title = match self.tab {
            Tab::Online => "Online",
            Tab::All => "All friends",
            Tab::Pending => "Pending",
            Tab::Blocked => "Blocked",
            Tab::Add => "",
        };
        let search = div()
            .flex_none()
            .h(px(36.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(t.radius_small)
            .bg(t.input)
            .child(div().flex_1().min_w_0().child(self.search.clone()))
            .child(icon(Icon::Search).size(px(18.)).text_color(t.text_faint));
        let mut col = div()
            .id("friends-list")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .px(px(20.))
            .pt(px(16.))
            .flex()
            .flex_col()
            .child(search)
            .child(
                div()
                    .mt(px(20.))
                    .mb(px(8.))
                    .child(section_label(format!("{title} — {}", shown.len()), &t)),
            );
        if shown.is_empty() {
            let text = match (self.tab, query.is_empty()) {
                (_, false) => "Nobody here matches that search.",
                (Tab::Online, _) => "No friends are online right now.",
                (Tab::Pending, _) => "There are no pending friend requests.",
                (Tab::Blocked, _) => "You haven't blocked anyone.",
                _ => "No friends yet. Add one with their username.",
            };
            return col
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(12.))
                        .text_color(t.text_faint)
                        .child(icon(Icon::Users).size(px(64.)))
                        .child(text),
                )
                .into_any_element();
        }
        for e in shown {
            col = col.child(self.row(e, cx));
        }
        col.into_any_element()
    }

    fn row(&self, e: Entry, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let id = e.id;
        let group = SharedString::from(format!("friend-{id}"));
        let state = self.state.clone();
        let mut buttons: Vec<AnyElement> = Vec::new();
        let mut add_button = |key: &'static str, which: Icon, tip: &'static str, danger: bool| {
            let state = self.state.clone();
            let el = div()
                .id(snowflake_id(key, id))
                .size(px(36.))
                .rounded_full()
                .bg(t.sidebar)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_color(t.text_muted)
                .hover(move |s| s.text_color(if danger { t.danger } else { t.header }))
                .tooltip(move |_, cx| Tooltip::view(tip, cx))
                .child(icon(which).size(px(18.)))
                .on_click(move |ev: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    let at = ev.position();
                    state.update(cx, |s, cx| match key {
                        "friend-msg" => s.open_dm(id, cx),
                        "friend-more" => s.open_profile(id, None, at, cx),
                        "friend-accept" => s.set_friend(id, true, cx),
                        _ => s.set_friend(id, false, cx),
                    });
                });
            buttons.push(el.into_any_element());
        };
        match e.kind {
            RelationshipKind::IncomingRequest => {
                add_button("friend-accept", Icon::Check, "Accept", false);
                add_button("friend-ignore", Icon::Close, "Ignore", true);
            }
            RelationshipKind::OutgoingRequest => {
                add_button("friend-cancel", Icon::Close, "Cancel", true)
            }
            RelationshipKind::Blocked => add_button("friend-unblock", Icon::Close, "Unblock", true),
            _ => {
                add_button("friend-msg", Icon::Message, "Message", false);
                add_button("friend-more", Icon::More, "Profile", false);
            }
        }
        let friend = e.kind == RelationshipKind::Friend;
        div()
            .id(snowflake_id("friend-row", id))
            .group(group.clone())
            .flex_none()
            .h(px(62.))
            .px(px(10.))
            .mx(px(-10.))
            .flex()
            .items_center()
            .gap(px(12.))
            .rounded(t.radius_small)
            .border_t_1()
            .border_color(t.border)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover).border_color(gpui::transparent_black()))
            .child(avatar(e.avatar, px(32.), Some((e.status, &t, t.surface))))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(6.))
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.header)
                                    .child(e.name),
                            )
                            .child(
                                div()
                                    .invisible()
                                    .group_hover(group, |s| s.visible())
                                    .text_sm()
                                    .text_color(t.text_muted)
                                    .child(e.username),
                            ),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .text_color(t.text_muted)
                            .child(e.line),
                    ),
            )
            .child(div().flex_none().flex().gap(px(10.)).children(buttons))
            .on_click(move |ev, _, cx| {
                if friend {
                    state.update(cx, |s, cx| s.open_dm(id, cx));
                } else {
                    let at = ev.position();
                    state.update(cx, |s, cx| s.open_profile(id, None, at, cx));
                }
            })
            .into_any_element()
    }

    fn add_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let ready = !self.add.read(cx).text().trim().is_empty() && !self.sending;
        let outline = match &self.add_result {
            Some(Ok(_)) => t.online,
            Some(Err(_)) => t.danger,
            None => t.border,
        };
        div()
            .flex_1()
            .min_w_0()
            .px(px(30.))
            .py(px(20.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .text_color(t.header)
                    .font_weight(FontWeight::BOLD)
                    .child("ADD FRIEND"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(t.text_muted)
                    .child("You can add friends with their Discord username."),
            )
            .child(
                div()
                    .mt(px(8.))
                    .h(px(52.))
                    .pl(px(14.))
                    .pr(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(t.radius)
                    .bg(t.input)
                    .border_1()
                    .border_color(outline)
                    .child(div().flex_1().min_w_0().child(self.add.clone()))
                    .child(
                        div()
                            .id("friend-send")
                            .flex_none()
                            .h(px(32.))
                            .px(px(16.))
                            .flex()
                            .items_center()
                            .rounded(t.radius_small)
                            .bg(t.accent)
                            .text_color(t.accent_text)
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .when(!ready, |el| el.opacity(0.5))
                            .when(ready, |el| el.cursor_pointer())
                            .child(if self.sending {
                                "Sending…"
                            } else {
                                "Send Friend Request"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.send_request(cx))),
                    ),
            )
            .when_some(self.add_result.clone(), |el, r| {
                el.child(
                    div().text_sm().child(match r {
                        Ok(name) => div()
                            .text_color(t.online)
                            .child(format!("Success! Your friend request to {name} was sent.")),
                        Err(e) => div().text_color(t.danger).child(format!(
                            "That didn't work. Check the username and try again. ({e})"
                        )),
                    }),
                )
            })
            .child(div().mt(px(24.)).h(px(1.)).bg(t.border))
            .child(
                div()
                    .mt(px(40.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.))
                    .text_color(t.text_faint)
                    .child(icon(Icon::UserPlus).size(px(72.)))
                    .child("Friends you add show up in the Online and All tabs."),
            )
            .into_any_element()
    }

    fn active_column(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let cards = self.active_now(cx);
        let mut col = div()
            .id("active-now")
            .flex_none()
            .w(px(360.))
            .h_full()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(t.border)
            .px(px(16.))
            .py(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::BOLD)
                    .text_color(t.header)
                    .child("Active Now"),
            );
        if cards.is_empty() {
            return col
                .child(
                    div()
                        .mt(px(8.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(4.))
                        .text_center()
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .text_color(t.header)
                                .child("It's quiet for now..."),
                        )
                        .child(div().text_sm().text_color(t.text_muted).child(
                            "When a friend plays a game, listens to music or joins voice, it shows up here.",
                        )),
                )
                .into_any_element();
        }
        for (ix, c) in cards.into_iter().enumerate() {
            col = col.child(active_card(c, ix, &self.state, &t));
        }
        col.into_any_element()
    }
}

fn active_card(c: ActiveCard, ix: usize, state: &Entity<AppState>, t: &Theme) -> AnyElement {
    let state = state.clone();
    let id = c.id;
    let art = match c.image {
        Some(url) => {
            let fallback_bg = t.hover;
            anim_img(url)
                .size(px(60.))
                .rounded(t.radius_small)
                .object_fit(ObjectFit::Cover)
                .with_fallback(move || div().size(px(60.)).bg(fallback_bg).into_any_element())
                .into_any_element()
        }
        None => div()
            .size(px(60.))
            .flex_none()
            .rounded(t.radius_small)
            .bg(t.hover)
            .flex()
            .items_center()
            .justify_center()
            .text_color(t.text_muted)
            .child(icon(c.icon).size(px(28.)))
            .into_any_element(),
    };
    div()
        .id(gpui::ElementId::NamedInteger(
            "active-card".into(),
            ix as u64,
        ))
        .p(px(16.))
        .rounded(t.radius)
        .bg(t.sidebar)
        .border_1()
        .border_color(t.border)
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(avatar(c.avatar, px(40.), Some((c.status, t, t.sidebar))))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.header)
                                .child(c.name),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .text_color(t.text_muted)
                                .child(c.headline),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(div().flex_none().child(art))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.text)
                                .child(c.title),
                        )
                        .children(
                            c.lines.into_iter().map(|l| {
                                div().truncate().text_xs().text_color(t.text_muted).child(l)
                            }),
                        ),
                ),
        )
        .on_click(move |ev, _, cx| {
            let at = ev.position();
            state.update(cx, |s, cx| s.open_profile(id, None, at, cx));
        })
        .into_any_element()
}

fn custom_line(a: &Activity) -> String {
    let text = a.state.clone().unwrap_or_default();
    match a
        .emoji
        .as_ref()
        .filter(|e| e.id.is_none())
        .and_then(|e| e.name.clone())
    {
        Some(emoji) if text.is_empty() => emoji,
        Some(emoji) => format!("{emoji} {text}"),
        None => text,
    }
}

/// The picture for an activity, from Discord's asset store, a proxied URL or Spotify.
fn activity_image(a: &Activity) -> Option<String> {
    let key = a.assets.as_ref()?.large_image.as_deref()?;
    if let Some(rest) = key.strip_prefix("mp:") {
        return Some(format!("https://media.discordapp.net/{rest}"));
    }
    if let Some(rest) = key.strip_prefix("spotify:") {
        return Some(format!("https://i.scdn.co/image/{rest}"));
    }
    if key.contains(':') {
        return None;
    }
    let app = a.application_id?;
    Some(format!(
        "https://cdn.discordapp.com/app-assets/{app}/{key}.png?size=160"
    ))
}

fn elapsed(ms: u64) -> String {
    let minutes = ms / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, 0) => "a moment".into(),
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

impl Render for FriendsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let entries = self.entries(cx);
        let header = self.header(&entries, cx);
        let main = if self.tab == Tab::Add {
            self.add_page(cx)
        } else {
            self.list(entries, cx)
        };
        div()
            .size_full()
            .bg(t.surface)
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(main)
                    // Active Now gives way to the list when the window is narrow.
                    .when(window.viewport_size().width >= px(1000.), |el| {
                        el.child(self.active_column(cx))
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_images() {
        let a: Activity = serde_json::from_value(serde_json::json!({
            "name": "Game", "type": 0, "application_id": "42",
            "assets": { "large_image": "123" }
        }))
        .unwrap();
        assert_eq!(
            activity_image(&a).as_deref(),
            Some("https://cdn.discordapp.com/app-assets/42/123.png?size=160")
        );
        let s: Activity = serde_json::from_value(serde_json::json!({
            "name": "Spotify", "type": 2, "assets": { "large_image": "spotify:abc" }
        }))
        .unwrap();
        assert_eq!(
            activity_image(&s).as_deref(),
            Some("https://i.scdn.co/image/abc")
        );
        assert_eq!(elapsed(0), "a moment");
        assert_eq!(elapsed(83 * 60_000), "1h 23m");
    }
}
