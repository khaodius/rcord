//! The member list, from the gateway's lazy member list for the open channel, or the other
//! person's profile card in a DM.

use gpui::{
    AnyElement, Context, Entity, FontWeight, IntoElement, ListAlignment, ListState, Render,
    Subscription, Window, div, list, prelude::*, px,
};
use rcord_model::{ChannelKind, Snowflake, Status};
use rcord_store::{Change, MemberRow};

use crate::state::{AppState, StateEvent};
use crate::theme::theme;
use crate::ui::{avatar, section_label, snowflake_id, user_avatar_url};

pub struct MemberList {
    state: Entity<AppState>,
    list: ListState,
    rows: usize,
    guild: Option<Snowflake>,
    _sub: Subscription,
}

impl MemberList {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let sub = cx.subscribe(&state, |this, state, event, cx| {
            let guild = state.read(cx).guild;
            match event {
                StateEvent::Store(Change::Members(g)) if Some(*g) == guild => this.sync(cx),
                StateEvent::Store(Change::Presence(_)) if guild.is_none() => cx.notify(),
                StateEvent::Store(Change::Everything)
                | StateEvent::Selection
                | StateEvent::Settings => this.sync(cx),
                _ => {}
            }
        });
        let list = ListState::new(0, ListAlignment::Top, px(600.));
        let weak = cx.weak_entity();
        let scroll_state = state.clone();
        list.set_scroll_handler(move |event, _, cx| {
            let total = weak.upgrade().map_or(0, |v| v.read(cx).rows);
            if total > 0 && event.visible_range.end + 20 >= total {
                scroll_state.update(cx, |s, _| {
                    s.request_member_range(event.visible_range.end as u32)
                });
            }
        });
        let mut this = Self {
            state,
            list,
            rows: 0,
            guild: None,
            _sub: sub,
        };
        this.sync(cx);
        this
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let s = self.state.read(cx);
        let guild = s.guild;
        let count = guild
            .and_then(|g| s.store.member_list(g))
            .map_or(0, |l| l.rows.len());
        if guild != self.guild || count != self.rows {
            self.list.reset(count);
        }
        self.guild = guild;
        self.rows = count;
        cx.notify();
    }
}

fn group_title(id: &str, count: u32, guild: Option<&rcord_model::Guild>) -> String {
    let name = match id {
        "online" => "Online".to_owned(),
        "offline" => "Offline".to_owned(),
        other => other
            .parse::<u64>()
            .ok()
            .and_then(Snowflake::new)
            .and_then(|rid| guild.and_then(|g| g.roles.iter().find(|r| r.id == rid)))
            .map_or_else(|| other.to_owned(), |r| r.name.clone()),
    };
    format!("{name} — {count}")
}

fn member_row(
    state: &Entity<AppState>,
    guild: Snowflake,
    row: Option<&MemberRow>,
    cx: &mut gpui::App,
) -> AnyElement {
    let t = theme(cx).clone();
    let s = state.read(cx);
    match row {
        None => div()
            .h(px(44.))
            .mx(px(8.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(12.))
            .child(div().size(px(32.)).rounded_full().bg(t.hover))
            .child(div().w(px(100.)).h(px(10.)).rounded_full().bg(t.hover))
            .into_any_element(),
        Some(MemberRow::Group { id, count }) => {
            let g = s.store.guild(guild).map(|e| &e.guild);
            div()
                .pt(px(20.))
                .pb(px(4.))
                .px(px(16.))
                .child(section_label(group_title(id, *count, g), &t))
                .into_any_element()
        }
        Some(MemberRow::Member(user_id)) => {
            let user_id = *user_id;
            let Some(user) = s.store.user(user_id) else {
                return div().h(px(44.)).into_any_element();
            };
            let status = s.store.status(user_id);
            let offline = matches!(status, Status::Offline | Status::Invisible);
            let name = s
                .store
                .display_name(Some(guild), user_id)
                .unwrap_or(&user.username)
                .to_owned();
            let color = s
                .store
                .name_color(guild, user_id)
                .map(|c| t.role_color(c, s.settings.accessibility.role_color_contrast));
            let activity = s
                .store
                .presence(user_id)
                .and_then(|p| {
                    p.custom_status()
                        .filter(|c| c.state.is_some())
                        .or(p.primary_activity())
                })
                .map(|a| a.summary())
                .filter(|a| !a.is_empty());
            let bot = user.bot;
            let url = user_avatar_url(user, 32., false);
            let state = state.clone();
            div()
                .id(snowflake_id("member", user_id))
                .h(px(44.))
                .mx(px(8.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(12.))
                .rounded(t.radius_small)
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .when(offline, |el| el.opacity(0.45))
                .child(avatar(url, px(32.), Some((status, &t, t.sidebar))))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .child(
                                    div()
                                        .truncate()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(color.unwrap_or(t.text))
                                        .child(name),
                                )
                                .when(bot, |el| {
                                    el.child(
                                        div()
                                            .px(px(4.))
                                            .rounded(px(3.))
                                            .bg(t.accent)
                                            .text_color(t.accent_text)
                                            .text_size(px(10.))
                                            .font_weight(FontWeight::BOLD)
                                            .child("APP"),
                                    )
                                }),
                        )
                        .when_some(activity, |el, a| {
                            el.child(div().truncate().text_xs().text_color(t.text_faint).child(a))
                        }),
                )
                .on_click(move |e, _, cx| {
                    let at = e.position();
                    state.update(cx, |s, cx| s.open_profile(user_id, Some(guild), at, cx))
                })
                .into_any_element()
        }
    }
}

impl Render for MemberList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let s = self.state.read(cx);
        let Some(guild) = s.guild else {
            // In a DM, show who the conversation is with.
            let other = s
                .channel
                .and_then(|c| s.store.channel(c))
                .filter(|c| c.kind == ChannelKind::Dm)
                .and_then(|c| c.recipient_ids().next())
                .and_then(|u| s.store.user(u).cloned());
            return div()
                .size_full()
                .bg(t.sidebar)
                .p(px(16.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .when_some(other, |el, u| {
                    let status = s.store.status(u.id);
                    el.child(avatar(
                        user_avatar_url(&u, 80., true),
                        px(80.),
                        Some((status, &t, t.sidebar)),
                    ))
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(t.header)
                            .child(u.display_name().to_owned()),
                    )
                    .child(div().text_sm().text_color(t.text_muted).child(u.tag()))
                })
                .into_any_element();
        };
        let state = self.state.clone();
        let content = list(self.list.clone(), move |ix, _window, cx| {
            let row = state
                .read(cx)
                .store
                .member_list(guild)
                .and_then(|l| l.rows.get(ix).cloned())
                .flatten();
            member_row(&state, guild, row.as_ref(), cx)
        })
        .size_full();
        div()
            .size_full()
            .bg(t.sidebar)
            .pb(px(8.))
            .child(content)
            .into_any_element()
    }
}
