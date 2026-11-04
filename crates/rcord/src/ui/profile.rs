//! The profile card: banner, avatar with decoration, badges, bio, roles and member dates.

use gpui::{
    AnyElement, App, Entity, FontWeight, Hsla, IntoElement, ObjectFit, div, img, prelude::*, px,
};
use rcord_model::{Snowflake, User, cdn};

use crate::LocalTime;
use crate::state::{AppState, ProfileState};
use crate::theme::theme;
use crate::ui::markup::{self, MarkupAction, MarkupCx};
use crate::ui::{Tooltip, avatar, section_label, user_avatar_url};

struct Badge {
    id: String,
    description: String,
    icon_url: String,
    link: Option<String>,
}

fn date_from_snowflake(id: Snowflake, offset: time::UtcOffset) -> String {
    time::OffsetDateTime::from_unix_timestamp((id.timestamp_ms() / 1000) as i64)
        .map(|d| d.to_offset(offset))
        .ok()
        .and_then(|d| {
            d.format(time::macros::format_description!(
                "[month repr:short] [day padding:none], [year]"
            ))
            .ok()
        })
        .unwrap_or_default()
}

fn date_from_iso(iso: &str, offset: time::UtcOffset) -> Option<String> {
    let d =
        time::OffsetDateTime::parse(iso, &time::format_description::well_known::Rfc3339).ok()?;
    d.to_offset(offset)
        .format(time::macros::format_description!(
            "[month repr:short] [day padding:none], [year]"
        ))
        .ok()
}

pub fn profile_card(
    state: &Entity<AppState>,
    user_id: Snowflake,
    guild: Option<Snowflake>,
    cx: &mut App,
) -> AnyElement {
    let t = theme(cx).clone();
    let offset = cx.global::<LocalTime>().0;
    let s = state.read(cx);
    let profile = s.profiles.get(&(user_id, guild)).cloned();
    let cached: Option<User> = s.store.user(user_id).cloned();
    let loaded = match &profile {
        Some(ProfileState::Loaded(p)) => Some(p.clone()),
        _ => None,
    };
    let user = loaded.as_ref().map(|p| p.user.user.clone()).or(cached);
    let Some(user) = user else {
        return div()
            .w(px(300.))
            .p(px(16.))
            .rounded(t.radius)
            .bg(t.floating)
            .text_color(t.text_muted)
            .child(match profile {
                Some(ProfileState::Failed(e)) => format!("Could not load profile: {e}"),
                _ => "Loading…".into(),
            })
            .into_any_element();
    };

    let meta = loaded.as_ref().and_then(|p| p.user_profile.clone());
    let guild_meta = loaded.as_ref().and_then(|p| p.guild_member_profile.clone());
    let banner_hash = guild_meta
        .as_ref()
        .and_then(|m| m.banner.clone())
        .or_else(|| meta.as_ref().and_then(|m| m.banner.clone()))
        .or_else(|| user.banner.clone());
    let accent: Hsla = meta
        .as_ref()
        .and_then(|m| {
            m.theme_colors
                .as_ref()
                .and_then(|c| c.first().copied())
                .or(m.accent_color)
        })
        .or(user.accent_color)
        .map_or(t.accent, |c| gpui::rgb(c).into());
    let bio = guild_meta
        .as_ref()
        .and_then(|m| m.bio.clone())
        .filter(|b| !b.is_empty())
        .or_else(|| meta.as_ref().and_then(|m| m.bio.clone()))
        .or_else(|| loaded.as_ref().and_then(|p| p.user.bio.clone()))
        .filter(|b| !b.is_empty());
    let pronouns = guild_meta
        .as_ref()
        .and_then(|m| m.pronouns.clone())
        .filter(|p| !p.is_empty())
        .or_else(|| meta.as_ref().and_then(|m| m.pronouns.clone()))
        .filter(|p| !p.is_empty());

    let mut badges: Vec<Badge> = loaded
        .as_ref()
        .map(|p| {
            p.badges
                .iter()
                .chain(p.guild_badges.iter())
                .map(|b| Badge {
                    id: b.id.clone(),
                    description: b.description.clone(),
                    icon_url: cdn::badge_icon(&b.icon),
                    link: b.link.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    if badges.is_empty() {
        badges = user
            .flag_badges()
            .into_iter()
            .map(|b| Badge {
                id: format!("{b:?}"),
                description: b.description().into(),
                icon_url: cdn::badge_icon(b.icon()),
                link: None,
            })
            .collect();
    }

    let status = s.store.status(user_id);
    let custom = s
        .store
        .presence(user_id)
        .and_then(|p| p.custom_status())
        .and_then(|a| a.state.clone());
    let name = s
        .store
        .display_name(guild, user_id)
        .unwrap_or(user.display_name())
        .to_owned();
    let member = guild.and_then(|g| s.store.member(g, user_id)).cloned();
    let roles: Vec<(String, u32)> = match (guild.and_then(|g| s.store.guild(g)), &member) {
        (Some(g), Some(m)) => {
            let mut r: Vec<_> = g
                .guild
                .roles
                .iter()
                .filter(|r| m.roles.contains(&r.id))
                .collect();
            r.sort_by_key(|r| std::cmp::Reverse(r.position));
            r.into_iter().map(|r| (r.name.clone(), r.color)).collect()
        }
        _ => Vec::new(),
    };
    let guild_name = guild
        .and_then(|g| s.store.guild(g))
        .map(|g| g.guild.name.clone());
    let joined = member
        .as_ref()
        .and_then(|m| m.joined_at.as_deref())
        .and_then(|d| date_from_iso(d, offset));
    let mutual = loaded.as_ref().map(|p| p.mutual_guilds.len()).unwrap_or(0);
    let connections: Vec<String> = loaded
        .as_ref()
        .map(|p| {
            p.connected_accounts
                .iter()
                .map(|c| format!("{} · {}", c.kind, c.name))
                .collect()
        })
        .unwrap_or_default();
    let decoration = user
        .avatar_decoration_data
        .as_ref()
        .map(|d| cdn::avatar_decoration(&d.asset, 240));
    let tag = user.server_tag().and_then(|p| p.tag.clone());

    let state_for_bio = state.clone();
    let bio_el = bio.map(|bio| {
        let mcx = MarkupCx {
            store: &s.store,
            guild,
            t: &t,
            font_size: px(14.),
            animate_emoji: true,
            revealed: true,
            underline_links: true,
            hour24: s.settings.appearance.hour24,
            utc_offset: offset,
            now_secs: (crate::state::now_ms() / 1000) as i64,
            code_font: "JetBrains Mono".into(),
            key: user_id.get(),
            on_action: std::rc::Rc::new(move |a, window, cx| match a {
                MarkupAction::OpenUrl(u) => cx.open_url(&u),
                MarkupAction::User(u) => {
                    let at = window.mouse_position();
                    state_for_bio.update(cx, |s, cx| s.open_profile(u, guild, at, cx))
                }
                _ => {}
            }),
        };
        markup::blocks(&rcord_markup::parse(&bio), &mcx)
    });

    let banner_h = if banner_hash.is_some() { 120. } else { 64. };
    let banner = div()
        .h(px(banner_h))
        .w_full()
        .bg(accent)
        .when_some(banner_hash, |el, h| {
            el.child(
                img(cdn::user_banner(user_id, &h, 600, true))
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            )
        });

    let avatar_block = div()
        .absolute()
        .left(px(16.))
        .top(px(banner_h - 46.))
        .size(px(92.))
        .rounded_full()
        .bg(t.floating)
        .flex()
        .items_center()
        .justify_center()
        .child(avatar(
            user_avatar_url(&user, 80., true),
            px(80.),
            Some((status, &t, t.floating)),
        ))
        .when_some(decoration, |el, url| {
            el.child(
                img(url)
                    .absolute()
                    .left(px(-4.))
                    .top(px(-4.))
                    .size(px(100.))
                    .object_fit(ObjectFit::Contain),
            )
        });

    let badge_row = (!badges.is_empty()).then(|| {
        let mut row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(2.))
            .p(px(4.))
            .rounded(t.radius_small)
            .bg(t.surface);
        for b in badges {
            let desc = b.description.clone();
            let link = b.link.clone();
            row = row.child(
                div()
                    .id(gpui::ElementId::Name(format!("badge-{}", b.id).into()))
                    .size(px(22.))
                    .cursor_pointer()
                    .tooltip(move |_, cx| Tooltip::view(desc.clone(), cx))
                    .child(img(b.icon_url).size(px(22.)).object_fit(ObjectFit::Contain))
                    .when_some(link, |el, l| el.on_click(move |_, _, cx| cx.open_url(&l))),
            );
        }
        row
    });

    let section = |title: &str| section_label(title.to_owned(), &t).mb(px(6.));

    div()
        .id("profile-card")
        .occlude()
        .w(px(340.))
        .max_h(px(620.))
        .overflow_y_scroll()
        .rounded(t.radius)
        .bg(t.floating)
        .border_1()
        .border_color(t.border)
        .shadow_lg()
        .overflow_hidden()
        .relative()
        .child(banner)
        .child(avatar_block)
        .child(
            div()
                .pt(px(52.))
                .px(px(16.))
                .flex()
                .justify_end()
                .children(badge_row),
        )
        .child(
            div()
                .p(px(16.))
                .pt(px(12.))
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_xl()
                                .font_weight(FontWeight::BOLD)
                                .text_color(t.header)
                                .child(name),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_sm()
                                .text_color(t.text)
                                .child(user.tag())
                                .when_some(tag, |el, tag| {
                                    el.child(
                                        div()
                                            .px(px(5.))
                                            .rounded(px(4.))
                                            .bg(t.hover)
                                            .text_xs()
                                            .font_weight(FontWeight::BOLD)
                                            .child(tag),
                                    )
                                })
                                .when_some(pronouns, |el, p| {
                                    el.child(div().text_color(t.text_muted).child(format!("· {p}")))
                                }),
                        )
                        .when_some(custom, |el, c| {
                            el.child(div().mt(px(6.)).text_sm().text_color(t.text).child(c))
                        }),
                )
                .child(div().h(px(1.)).bg(t.border))
                .when_some(bio_el, |el, bio| {
                    el.child(
                        div()
                            .flex()
                            .flex_col()
                            .child(section("About Me"))
                            .child(bio),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(section("Member Since"))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .text_sm()
                                .text_color(t.text)
                                .child(format!("Discord: {}", date_from_snowflake(user_id, offset)))
                                .when_some(joined.zip(guild_name), |el, (j, g)| {
                                    el.child(div().text_color(t.text_faint).child("·"))
                                        .child(format!("{g}: {j}"))
                                }),
                        ),
                )
                .when(!roles.is_empty(), |el| {
                    let mut chips = div().flex().flex_row().flex_wrap().gap(px(4.));
                    for (name, color) in roles {
                        let c: Hsla = if color == 0 {
                            t.text_faint
                        } else {
                            gpui::rgb(color).into()
                        };
                        chips = chips.child(
                            div()
                                .h(px(24.))
                                .px(px(8.))
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .rounded(t.radius_small)
                                .bg(t.surface)
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.text)
                                .child(div().size(px(10.)).rounded_full().bg(c))
                                .child(name),
                        );
                    }
                    el.child(div().flex().flex_col().child(section("Roles")).child(chips))
                })
                .when(!connections.is_empty(), |el| {
                    el.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(section("Connections"))
                            .children(
                                connections
                                    .into_iter()
                                    .map(|c| div().text_sm().text_color(t.text).child(c)),
                            ),
                    )
                })
                .when(mutual > 0, |el| {
                    el.child(div().text_sm().text_color(t.text_muted).child(format!(
                        "{mutual} mutual server{}",
                        if mutual == 1 { "" } else { "s" }
                    )))
                }),
        )
        .into_any_element()
}
