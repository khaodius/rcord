//! The server list on the far side: home, then every server as an icon with its unread pill.

use gpui::{
    Context, Entity, FontWeight, IntoElement, MouseButton, ObjectFit, Render, ScrollHandle,
    Subscription, Window, div, prelude::*, px,
};
use rcord_model::{Snowflake, cdn};
use rcord_store::Change;

use crate::state::{AppState, GuildMenuPage, Overlay, StateEvent};
use crate::theme::theme;
use crate::ui::anim_img;
use crate::ui::icons::{Icon, icon};
use crate::ui::{Tooltip, mention_badge, should_animate, snowflake_id};

/// A server icon being dragged to a new place in the list.
#[derive(Clone)]
struct DraggedGuild {
    id: Snowflake,
    content: DragPicture,
}

#[derive(Clone)]
enum DragPicture {
    Image(String),
    Acronym(String),
}

/// What follows the pointer while a server is dragged.
struct GuildDragPreview(DragPicture);

impl Render for GuildDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let inner = match &self.0 {
            DragPicture::Image(url) => anim_img(url.clone())
                .size(px(48.))
                .rounded_full()
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
            DragPicture::Acronym(a) => div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(a.clone())
                .into_any_element(),
        };
        div()
            .size(px(48.))
            .rounded_full()
            .bg(t.surface)
            .text_color(t.text)
            .flex()
            .items_center()
            .justify_center()
            .opacity(0.85)
            .shadow_lg()
            .child(inner)
    }
}

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
                    .rounded_full()
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
                        state.update(cx, |s, cx| {
                            // Home again from Home goes to the friends page.
                            if key.is_none() && s.guild.is_none() {
                                s.show_friends(cx);
                            } else {
                                s.select_guild(key, cx);
                            }
                        });
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
        let dm_mentions: u32 = state
            .store
            .private_channels()
            .map(|c| state.store.read_state(c.id).mentions)
            .sum();
        let order = &state.settings.layout.guild_order;
        let mut entries: Vec<_> = state.store.guilds().collect();
        entries.sort_by_key(|g| {
            order
                .iter()
                .position(|id| *id == g.guild.id.get())
                .unwrap_or(usize::MAX)
        });
        let order_now: std::rc::Rc<Vec<Snowflake>> =
            std::rc::Rc::new(entries.iter().map(|g| g.guild.id).collect());
        let home_icon = state.settings.appearance.home_icon;
        let home_path = state.settings.appearance.home_icon_path.clone();
        let me_avatar = state
            .store
            .me
            .as_ref()
            .map(|u| crate::ui::user_avatar_url(u, 48., false));
        let guilds: Vec<_> = entries
            .into_iter()
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
            home_content(home_icon, home_path, me_avatar),
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
            let drag_picture = match &url {
                Some(u) => DragPicture::Image(u.clone()),
                None => DragPicture::Acronym(acronym.clone()),
            };
            let content = match url {
                // A parent's rounded corners do not clip an image, so the image is rounded
                // itself; otherwise its square corners show.
                Some(url) => anim_img(url.clone())
                    .size(px(48.))
                    .rounded_full()
                    .object_fit(ObjectFit::Cover)
                    .into_any_element(),
                None => div()
                    .text_size(px(if acronym.len() > 2 { 13. } else { 16. }))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(acronym)
                    .into_any_element(),
            };
            let drag = DraggedGuild {
                id,
                content: drag_picture,
            };
            let state = self.state.clone();
            let drop_state = self.state.clone();
            let drop_order = order_now.clone();
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
                .on_drag(drag, |d, _, _, cx| {
                    cx.new(|_| GuildDragPreview(d.content.clone()))
                })
                .drag_over::<DraggedGuild>(move |style, _, _, cx| {
                    let t = theme(cx);
                    style.border_t_2().border_color(t.accent)
                })
                .on_drop(move |d: &DraggedGuild, _, cx| {
                    move_guild(&drop_state, &drop_order, d.id, Some(id), cx);
                })
                .on_mouse_up(MouseButton::Right, move |e, _, cx| {
                    let at = e.position;
                    state.update(cx, |s, cx| {
                        s.set_overlay(
                            Some(Overlay::GuildMenu {
                                guild: id,
                                at,
                                page: GuildMenuPage::Main,
                            }),
                            cx,
                        )
                    });
                });
            list = list.child(wrapped);
        }

        let end_state = self.state.clone();
        let end_order = order_now.clone();
        list = list.child(
            div()
                .id("explore")
                .drag_over::<DraggedGuild>(|style, _, _, cx| {
                    let t = theme(cx);
                    style.border_2().border_color(t.accent)
                })
                .on_drop(move |d: &DraggedGuild, _, cx| {
                    move_guild(&end_state, &end_order, d.id, None, cx);
                })
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

/// Puts `dragged` just above `target`, or at the end, and saves the whole order.
fn move_guild(
    state: &Entity<AppState>,
    order: &[Snowflake],
    dragged: Snowflake,
    target: Option<Snowflake>,
    cx: &mut gpui::App,
) {
    if target == Some(dragged) {
        return;
    }
    let mut ids: Vec<Snowflake> = order.iter().copied().filter(|g| *g != dragged).collect();
    let at = target
        .and_then(|t| ids.iter().position(|g| *g == t))
        .unwrap_or(ids.len());
    ids.insert(at, dragged);
    state.update(cx, |s, cx| {
        s.update_settings(cx, |st| {
            st.layout.guild_order = ids.iter().map(|g| g.get()).collect();
        })
    });
}

fn home_content(
    which: rcord_settings::HomeIcon,
    path: Option<String>,
    avatar: Option<String>,
) -> gpui::AnyElement {
    use rcord_settings::HomeIcon;
    let picture = |src: gpui::ImageSource| {
        gpui::img(src)
            .size(px(48.))
            .rounded_full()
            .object_fit(ObjectFit::Cover)
            .with_fallback(|| icon(Icon::Logo).size(px(28.)).into_any_element())
            .into_any_element()
    };
    match (which, path, avatar) {
        (HomeIcon::Friends, ..) => icon(Icon::Users).size(px(26.)).into_any_element(),
        (HomeIcon::Avatar, _, Some(url)) => picture(url.into()),
        (HomeIcon::Image, Some(p), _) => picture(std::path::PathBuf::from(p).into()),
        _ => icon(Icon::Logo).size(px(28.)).into_any_element(),
    }
}
