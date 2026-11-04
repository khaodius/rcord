//! The server list on the far side: home, then every server as an icon with its unread pill.

use gpui::{
    Context, Entity, FontWeight, IntoElement, MouseButton, ObjectFit, Render, ScrollHandle,
    Subscription, Window, div, prelude::*, px,
};
use rcord_model::{Snowflake, cdn};
use rcord_store::Change;

use crate::state::{AppState, MenuItem, Overlay, StateEvent};
use crate::theme::theme;
use crate::ui::anim_img;
use crate::ui::icons::{Icon, icon};
use crate::ui::{Tooltip, mention_badge, should_animate, snowflake_id};

pub struct ServerRail {
    state: Entity<AppState>,
    scroll: ScrollHandle,
    hovered: Option<Option<Snowflake>>,
    _sub: Subscription,
}

impl ServerRail {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let sub = cx.subscribe(&state, |_, _, event, cx| match event {
            StateEvent::Store(
                Change::Everything
                | Change::Guilds
                | Change::Guild(_)
                | Change::Messages(_)
                | Change::ReadState(_)
                | Change::Channels(_),
            )
            | StateEvent::Selection
            | StateEvent::Settings => cx.notify(),
            _ => {}
        });
        Self {
            state,
            scroll: ScrollHandle::new(),
            hovered: None,
            _sub: sub,
        }
    }

    fn item(
        &self,
        key: Option<Snowflake>,
        selected: bool,
        unread: bool,
        mentions: u32,
        tooltip: String,
        content: gpui::AnyElement,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = theme(cx).clone();
        let hovered = self.hovered == Some(key);
        let pill_h = if selected {
            px(40.)
        } else if hovered {
            px(20.)
        } else if unread {
            px(8.)
        } else {
            px(0.)
        };
        let radius = if selected || hovered {
            px(16.)
        } else {
            px(24.)
        };
        let state = self.state.clone();
        let id = key.map_or_else(
            || gpui::ElementId::Name("home".into()),
            |g| snowflake_id("guild", g),
        );
        div()
            .id(id)
            .relative()
            .flex()
            .flex_none()
            .w_full()
            .h(px(48.))
            .justify_center()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.hovered = if *hovered {
                    Some(key)
                } else if this.hovered == Some(key) {
                    None
                } else {
                    this.hovered
                };
                cx.notify();
            }))
            .child(
                div()
                    .absolute()
                    .left(px(0.))
                    .top(px(24.) - pill_h / 2.)
                    .w(px(4.))
                    .h(pill_h)
                    .rounded_r(px(4.))
                    .bg(t.header),
            )
            .child(
                div()
                    .id("icon")
                    .size(px(48.))
                    .rounded(radius)
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(if selected || hovered {
                        t.accent
                    } else {
                        t.surface
                    })
                    .text_color(if selected || hovered {
                        t.accent_text
                    } else {
                        t.text
                    })
                    .cursor_pointer()
                    .child(content)
                    .tooltip(move |_, cx| Tooltip::view(tooltip.clone(), cx))
                    .on_click(move |_, _, cx| {
                        state.update(cx, |s, cx| s.select_guild(key, cx));
                    }),
            )
            .when(mentions > 0, |el| {
                el.child(
                    div()
                        .absolute()
                        .bottom(px(-2.))
                        .right(px(10.))
                        .p(px(3.))
                        .rounded_full()
                        .bg(t.base)
                        .child(mention_badge(mentions, &t)),
                )
            })
    }
}

impl Render for ServerRail {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let state = self.state.read(cx);
        let selected_guild = state.guild;
        let animate = state.settings.appearance.animate_avatars;
        let show_names = state.settings.appearance.server_names;
        let dev = state.settings.advanced.developer_mode;
        let dm_mentions: u32 = state
            .store
            .private_channels()
            .map(|c| state.store.read_state(c.id).mentions)
            .sum();
        let guilds: Vec<_> = state
            .store
            .guilds()
            .map(|g| {
                let (unread, mentions) = state.store.guild_unread(g.guild.id);
                let hovered = self.hovered == Some(Some(g.guild.id));
                (
                    g.guild.id,
                    g.guild.name.clone(),
                    g.guild.acronym(),
                    cdn::guild_icon(&g.guild, 96, should_animate(animate, hovered)),
                    unread,
                    mentions,
                )
            })
            .collect();

        let home = self.item(
            None,
            selected_guild.is_none(),
            false,
            dm_mentions,
            "Direct Messages".into(),
            icon(Icon::Logo).size(px(28.)).into_any_element(),
            cx,
        );

        let mut list = div()
            .id("servers")
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .py(px(12.))
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(home)
            .child(
                div()
                    .w(px(32.))
                    .h(px(2.))
                    .rounded_full()
                    .bg(t.border)
                    .flex_none(),
            );

        for (id, name, acronym, url, unread, mentions) in guilds {
            let content = match url {
                Some(url) => anim_img(url)
                    .size(px(48.))
                    .object_fit(ObjectFit::Cover)
                    .into_any_element(),
                None => div()
                    .text_size(px(if acronym.len() > 2 { 13. } else { 16. }))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(acronym)
                    .into_any_element(),
            };
            let state = self.state.clone();
            let menu_name = name.clone();
            let item = self.item(
                Some(id),
                selected_guild == Some(id),
                unread,
                mentions,
                name.clone(),
                content,
                cx,
            );
            let wrapped = div()
                .id(snowflake_id("guild-wrap", id))
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .child(item)
                .when(show_names, |el| {
                    el.child(
                        div()
                            .w(px(64.))
                            .text_xs()
                            .text_center()
                            .text_color(t.text_muted)
                            .truncate()
                            .child(name.clone()),
                    )
                })
                .on_mouse_up(MouseButton::Right, move |e, _, cx| {
                    let at = e.position;
                    let mut items = vec![MenuItem {
                        label: "Mark as read".into(),
                        danger: false,
                        action: std::rc::Rc::new({
                            let state = state.clone();
                            move |cx| state.update(cx, |s, cx| s.mark_guild_read(id, cx))
                        }),
                    }];
                    if dev {
                        items.push(MenuItem {
                            label: "Copy server ID".into(),
                            danger: false,
                            action: std::rc::Rc::new(move |cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    id.to_string(),
                                ))
                            }),
                        });
                    }
                    let _ = &menu_name;
                    state.update(cx, |s, cx| {
                        s.set_overlay(Some(Overlay::Menu { at, items }), cx)
                    });
                });
            list = list.child(wrapped);
        }

        list = list.child(
            div()
                .id("explore")
                .flex_none()
                .size(px(48.))
                .rounded(px(24.))
                .bg(t.surface)
                .text_color(t.online)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(t.online).text_color(gpui::white()).rounded(px(16.)))
                .tooltip(|_, cx| Tooltip::view("Add a Server", cx))
                .child(icon(Icon::Plus).size(px(22.))),
        );

        div().size_full().bg(t.base).child(list)
    }
}
