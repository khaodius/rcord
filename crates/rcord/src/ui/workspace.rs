//! The main window layout: the four panels in the user's order, width handles between them,
//! and the overlay layer on top.
//!
//! Panels move the way Discord's own layout reads, with one addition that stays out of the
//! way: hold the right mouse button on a panel for a moment and it lifts; move to where it
//! should go and release. A quick right click still opens the context menu underneath.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyView, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight,
    IntoElement, MouseButton, Pixels, Point, Render, Subscription, TitlebarOptions, Window,
    WindowBounds, WindowHandle, WindowOptions, actions, canvas, div, prelude::*, px, size,
};
use rcord_settings::PanelId;

use crate::state::{AppState, Overlay, SettingsPage, StateEvent};
use crate::theme::theme;
use crate::ui::channels::ChannelSidebar;
use crate::ui::chat::ChatView;
use crate::ui::icons::{Icon, icon};
use crate::ui::members::MemberList;
use crate::ui::overlays::OverlayLayer;
use crate::ui::panel_window::PanelWindow;
use crate::ui::servers::ServerRail;
use crate::ui::settings::SettingsView;

actions!(
    workspace,
    [
        QuickSwitcher,
        OpenSettings,
        PreviousChannel,
        NextChannel,
        PreviousUnread,
        NextUnread,
        PreviousServer,
        NextServer,
        ToggleMute,
        ToggleDeafen,
        ToggleMemberList,
        MarkRead,
        Search,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ResetLayout,
    ]
);

/// Binds the configurable shortcuts. Unknown action names in the settings file are ignored.
pub fn bind_keys(keybinds: &rcord_settings::Keybinds, cx: &mut gpui::App) {
    use gpui::KeyBinding;
    let ctx = Some("Workspace");
    let mut bindings = Vec::new();
    for (action, keys) in &keybinds.0 {
        let keys = keys.as_str();
        let binding = match action.as_str() {
            "quick-switcher" => KeyBinding::new(keys, QuickSwitcher, ctx),
            "settings" => KeyBinding::new(keys, OpenSettings, ctx),
            "previous-channel" => KeyBinding::new(keys, PreviousChannel, ctx),
            "next-channel" => KeyBinding::new(keys, NextChannel, ctx),
            "previous-unread" => KeyBinding::new(keys, PreviousUnread, ctx),
            "next-unread" => KeyBinding::new(keys, NextUnread, ctx),
            "previous-server" => KeyBinding::new(keys, PreviousServer, ctx),
            "next-server" => KeyBinding::new(keys, NextServer, ctx),
            "toggle-mute" => KeyBinding::new(keys, ToggleMute, ctx),
            "toggle-deafen" => KeyBinding::new(keys, ToggleDeafen, ctx),
            "toggle-member-list" => KeyBinding::new(keys, ToggleMemberList, ctx),
            "mark-read" => KeyBinding::new(keys, MarkRead, ctx),
            "search" => KeyBinding::new(keys, Search, ctx),
            "zoom-in" => KeyBinding::new(keys, ZoomIn, ctx),
            "zoom-out" => KeyBinding::new(keys, ZoomOut, ctx),
            "zoom-reset" => KeyBinding::new(keys, ZoomReset, ctx),
            "reset-layout" => KeyBinding::new(keys, ResetLayout, ctx),
            _ => continue,
        };
        bindings.push(binding);
    }
    cx.bind_keys(bindings);
}

/// How long the right button must be held before a panel lifts.
const HOLD: Duration = Duration::from_millis(280);
/// Moving this far with the button held lifts the panel early.
const HOLD_SLOP: f32 = 6.0;
const SERVER_RAIL_WIDTH: f32 = 72.0;
const HANDLE_WIDTH: f32 = 6.0;
/// How long a panel takes to fold away when it pops out, and to open when it docks.
const POP_OUT: Duration = Duration::from_millis(200);
const DOCK_IN: Duration = Duration::from_millis(380);

/// Panels that can live in their own window. Chat stays, so the main window is never empty.
fn detachable(panel: PanelId) -> bool {
    panel != PanelId::Chat
}

#[derive(Clone, Copy)]
struct Hold {
    panel: PanelId,
    origin: Point<Pixels>,
    generation: u64,
}

#[derive(Clone, Copy)]
struct Lift {
    panel: PanelId,
    cursor: Point<Pixels>,
}

#[derive(Clone, Copy)]
struct Resize {
    panel: PanelId,
    start_x: Pixels,
    start_width: f32,
    /// +1 when the handle is on the panel's right edge, -1 on its left.
    direction: f32,
    width: f32,
}

type PanelBounds = Rc<RefCell<HashMap<PanelId, Bounds<Pixels>>>>;

pub struct Workspace {
    state: Entity<AppState>,
    servers: Entity<ServerRail>,
    channels: Entity<ChannelSidebar>,
    chat: Entity<ChatView>,
    members: Entity<MemberList>,
    overlays: Entity<OverlayLayer>,
    settings: Entity<SettingsView>,
    focus: FocusHandle,
    bounds: PanelBounds,
    hold: Option<Hold>,
    lift: Option<Lift>,
    resize: Option<Resize>,
    generation: u64,
    /// Panels open in their own windows.
    detached: HashMap<PanelId, WindowHandle<PanelWindow>>,
    /// A panel folding away after popping out; its spot is drawn empty until it closes.
    leaving: Option<PanelId>,
    /// The panel that last docked, and a number that restarts its opening animation.
    arriving: Option<(PanelId, u64)>,
    /// Where the pop-out target was drawn while a panel is lifted.
    pop_zone: Rc<RefCell<Option<Bounds<Pixels>>>>,
    _subs: Vec<Subscription>,
}

impl Workspace {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let servers = cx.new(|cx| ServerRail::new(state.clone(), cx));
        let channels = cx.new(|cx| ChannelSidebar::new(state.clone(), cx));
        let chat = cx.new(|cx| ChatView::new(state.clone(), window, cx));
        let members = cx.new(|cx| MemberList::new(state.clone(), cx));
        let overlays = cx.new(|cx| OverlayLayer::new(state.clone(), chat.clone(), window, cx));
        let settings = cx.new(|cx| SettingsView::new(state.clone(), window, cx));
        let subs = vec![cx.subscribe_in(
            &state,
            window,
            |this, state, event, window, cx| match event {
                StateEvent::Settings => {
                    let s = state.read(cx);
                    window.set_rem_size(px(16.0 * s.settings.appearance.ui_scale));
                    cx.notify();
                }
                StateEvent::Overlay => {
                    let overlay = state.read(cx).overlay.clone();
                    match overlay {
                        Some(o @ (Overlay::Expressions(_) | Overlay::QuickSwitcher)) => {
                            this.overlays
                                .update(cx, |layer, cx| layer.focus_for(&o, window, cx));
                        }
                        Some(Overlay::Settings(_)) => {
                            window.focus(&this.settings.focus_handle(cx));
                        }
                        None => window.focus(&this.chat.focus_handle(cx)),
                        _ => {}
                    }
                    cx.notify();
                }
                StateEvent::Selection => window.focus(&this.chat.focus_handle(cx)),
                _ => {}
            },
        )];
        window.focus(&chat.focus_handle(cx));
        Self {
            state,
            servers,
            channels,
            chat,
            members,
            overlays,
            settings,
            focus: cx.focus_handle(),
            bounds: Rc::default(),
            hold: None,
            lift: None,
            resize: None,
            generation: 0,
            detached: HashMap::new(),
            leaving: None,
            arriving: None,
            pop_zone: Rc::default(),
            _subs: subs,
        }
    }

    fn panel_at(&self, at: Point<Pixels>) -> Option<PanelId> {
        self.bounds
            .borrow()
            .iter()
            .find(|(_, b)| b.contains(&at))
            .map(|(p, _)| *p)
    }

    fn begin_hold(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(panel) = self.panel_at(at) else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        self.hold = Some(Hold {
            panel,
            origin: at,
            generation,
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOLD).await;
            this.update(cx, |this, cx| {
                if let Some(h) = this.hold.filter(|h| h.generation == generation) {
                    this.lift_panel(h, h.origin, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn lift_panel(&mut self, hold: Hold, cursor: Point<Pixels>, cx: &mut Context<Self>) {
        self.hold = None;
        self.lift = Some(Lift {
            panel: hold.panel,
            cursor,
        });
        cx.notify();
    }

    /// Where the lifted panel would land: an index into the visible panels.
    fn drop_slot(&self, lift: Lift, visible: &[PanelId]) -> usize {
        let bounds = self.bounds.borrow();
        visible
            .iter()
            .filter(|p| **p != lift.panel)
            .take_while(|p| bounds.get(*p).is_some_and(|b| lift.cursor.x > b.center().x))
            .count()
    }

    /// Whether letting go here pops the lifted panel out: over the target at the top, or
    /// past the edge of the window.
    fn over_pop_out(&self, lift: Lift, window: &Window) -> bool {
        if !detachable(lift.panel) {
            return false;
        }
        if self
            .pop_zone
            .borrow()
            .is_some_and(|b| b.contains(&lift.cursor))
        {
            return true;
        }
        let size = window.viewport_size();
        let p = lift.cursor;
        p.x < px(0.) || p.y < px(0.) || p.x > size.width || p.y > size.height
    }

    fn drop_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(lift) = self.lift.take() else { return };
        if self.over_pop_out(lift, window) {
            self.pop_out(lift.panel, lift.cursor, window, cx);
            return;
        }
        let visible = self.visible(cx);
        let slot = self.drop_slot(lift, &visible);
        self.state.update(cx, |s, cx| {
            s.update_settings(cx, |settings| {
                let layout = &mut settings.layout;
                let others: Vec<PanelId> = visible
                    .iter()
                    .copied()
                    .filter(|p| *p != lift.panel)
                    .collect();
                let target = match others.get(slot) {
                    Some(next) => layout
                        .order
                        .iter()
                        .position(|p| p == next)
                        .unwrap_or(layout.order.len()),
                    None => layout.order.len(),
                };
                let from = layout
                    .order
                    .iter()
                    .position(|p| *p == lift.panel)
                    .unwrap_or(0);
                let target = if from < target { target - 1 } else { target };
                layout.move_panel(lift.panel, target);
            });
        });
        cx.notify();
    }

    fn visible(&self, cx: &gpui::App) -> Vec<PanelId> {
        let s = self.state.read(cx);
        let layout = &s.settings.layout;
        let in_guild = s.guild.is_some() || s.channel.is_some();
        layout
            .order
            .iter()
            .copied()
            .filter(|p| layout.is_visible(*p))
            .filter(|p| *p != PanelId::Members || in_guild)
            .filter(|p| !self.detached.contains_key(p) || self.leaving == Some(*p))
            .collect()
    }

    /// Moves `panel` into a window of its own, near where it was let go.
    fn pop_out(
        &mut self,
        panel: PanelId,
        cursor: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.detached.contains_key(&panel) {
            return;
        }
        let width = match panel {
            PanelId::Servers => 96.,
            p => self.panel_width(p, cx).unwrap_or(260.) + 40.,
        };
        let main = window.bounds();
        let height = f32::from(main.size.height).clamp(420., 900.);
        let origin = main.origin
            + gpui::point(
                (cursor.x - px(width / 2.)).max(px(0.)),
                (cursor.y - px(40.)).max(px(0.)),
            );
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some(format!("rcord · {}", panel.label()).into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin,
                size: size(px(width), px(height)),
            })),
            window_min_size: Some(size(px(72.), px(240.))),
            app_id: Some("rcord".into()),
            focus: true,
            show: true,
            ..Default::default()
        };
        let state = self.state.clone();
        let view = self.detached_view(panel);
        let workspace = cx.weak_entity();
        match cx.open_window(options, move |window, cx| {
            cx.new(|cx| PanelWindow::new(state, panel, view, workspace, window, cx))
        }) {
            Ok(handle) => {
                self.detached.insert(panel, handle);
                self.leaving = Some(panel);
                if self.arriving.is_some_and(|(p, _)| p == panel) {
                    self.arriving = None;
                }
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(POP_OUT).await;
                    this.update(cx, |this, cx| {
                        if this.leaving == Some(panel) {
                            this.leaving = None;
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
            Err(e) => {
                let message = format!("Could not open a window: {e}");
                self.state.update(cx, |s, cx| s.toast(message, cx));
            }
        }
        cx.notify();
    }

    /// Called by a popped-out window as it closes: the panel comes back to its spot.
    pub fn docked(&mut self, panel: PanelId, cx: &mut Context<Self>) {
        if self.detached.remove(&panel).is_none() {
            return;
        }
        if self.leaving == Some(panel) {
            self.leaving = None;
        }
        self.generation += 1;
        self.arriving = Some((panel, self.generation));
        cx.notify();
    }

    fn detached_view(&self, panel: PanelId) -> AnyView {
        let full = gpui::StyleRefinement::default().size_full();
        match panel {
            PanelId::Servers => AnyView::from(self.servers.clone()).cached(full),
            PanelId::Channels => AnyView::from(self.channels.clone()).cached(full),
            PanelId::Chat => self.chat.clone().into(),
            PanelId::Members => AnyView::from(self.members.clone()).cached(full),
        }
    }

    fn panel_width(&self, panel: PanelId, cx: &gpui::App) -> Option<f32> {
        if panel == PanelId::Servers {
            return Some(SERVER_RAIL_WIDTH);
        }
        if let Some(r) = self.resize.filter(|r| r.panel == panel) {
            return Some(r.width);
        }
        self.state.read(cx).settings.layout.width(panel)
    }

    fn view(&self, panel: PanelId) -> AnyView {
        match panel {
            PanelId::Servers => AnyView::from(self.servers.clone())
                .cached(gpui::StyleRefinement::default().size_full()),
            PanelId::Channels => AnyView::from(self.channels.clone())
                .cached(gpui::StyleRefinement::default().size_full()),
            PanelId::Chat => self.chat.clone().into(),
            PanelId::Members => AnyView::from(self.members.clone())
                .cached(gpui::StyleRefinement::default().size_full()),
        }
    }

    fn resize_handle(
        &self,
        panel: PanelId,
        direction: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = theme(cx).clone();
        let active = self.resize.is_some_and(|r| r.panel == panel);
        div()
            .id(gpui::ElementId::NamedInteger("resize".into(), panel as u64))
            .flex_none()
            .w(px(HANDLE_WIDTH))
            .mx(px(-HANDLE_WIDTH / 2.))
            .h_full()
            .relative()
            .cursor_col_resize()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(HANDLE_WIDTH / 2. - 1.))
                    .w(px(2.))
                    .when(active, |el| el.bg(t.accent))
                    .when(!active, |el| el.hover(|s| s.bg(t.border))),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                    let start_width = this.panel_width(panel, cx).unwrap_or(240.);
                    this.resize = Some(Resize {
                        panel,
                        start_x: e.position.x,
                        start_width,
                        direction,
                        width: start_width,
                    });
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                // Double click restores the default width.
                if e.click_count() >= 2 {
                    this.resize = None;
                    this.state.update(cx, |s, cx| {
                        s.update_settings(cx, |st| {
                            st.layout.widths.remove(&panel);
                        })
                    });
                }
            }))
    }

    fn on_mouse_move(&mut self, e: &gpui::MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some(r) = self.resize.as_mut() {
            if e.pressed_button != Some(MouseButton::Left) {
                self.commit_resize(cx);
                return;
            }
            let (_, min, max) = r.panel.width_range().unwrap_or((240., 180., 420.));
            let dx = f32::from(e.position.x - r.start_x) * r.direction;
            r.width = (r.start_width + dx).clamp(min, max);
            cx.notify();
            return;
        }
        if let Some(h) = self.hold {
            if e.pressed_button != Some(MouseButton::Right) {
                self.hold = None;
                return;
            }
            let d = e.position - h.origin;
            if f32::from(d.x).hypot(f32::from(d.y)) > HOLD_SLOP {
                self.lift_panel(h, e.position, cx);
            }
            return;
        }
        if let Some(l) = self.lift.as_mut() {
            l.cursor = e.position;
            cx.notify();
        }
    }

    fn commit_resize(&mut self, cx: &mut Context<Self>) {
        let Some(r) = self.resize.take() else { return };
        self.state.update(cx, |s, cx| {
            s.update_settings(cx, |st| {
                st.layout.widths.insert(r.panel, r.width.round());
            });
        });
        cx.notify();
    }

    // ---- actions --------------------------------------------------------------------------

    fn zoom(&mut self, step: Option<f32>, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| {
            s.update_settings(cx, |st| {
                let a = &mut st.appearance;
                a.ui_scale = match step {
                    Some(d) => ((a.ui_scale + d) * 20.).round() / 20.,
                    None => 1.0,
                }
                .clamp(0.75, 1.5);
            });
        });
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let pop_target = self.lift.is_some_and(|l| self.over_pop_out(l, window));
        let visible = self.visible(cx);
        let s = self.state.read(cx);
        let settings_page = match s.overlay {
            Some(Overlay::Settings(page)) => Some(page),
            _ => None,
        };
        let borders = s.settings.appearance.panel_borders;
        let reduced_motion = s.settings.accessibility.reduced_motion;
        let lift = self.lift;
        let over_home = lift.is_some_and(|l| {
            self.bounds
                .borrow()
                .get(&l.panel)
                .is_some_and(|b| b.contains(&l.cursor))
        });
        // Over its own spot the panel just goes back, so no insertion line is drawn.
        let slot = lift
            .filter(|_| !over_home && !pop_target)
            .map(|l| self.drop_slot(l, &visible));
        let leaving = self.leaving;
        let arriving = self.arriving;

        let chat_ix = visible
            .iter()
            .position(|p| *p == PanelId::Chat)
            .unwrap_or(0);
        let mut row = div().flex().flex_row().size_full().min_h_0();
        let mut slot_counter = 0usize;
        let indicator = || {
            div()
                .flex_none()
                .w(px(3.))
                .mx(px(-1.5))
                .h_full()
                .rounded_full()
                .bg(t.accent)
        };
        for (ix, panel) in visible.iter().copied().enumerate() {
            let lifted = lift.is_some_and(|l| l.panel == panel);
            if !lifted {
                if slot == Some(slot_counter) {
                    row = row.child(indicator());
                }
                slot_counter += 1;
            }
            // Handles sit on the edge that faces the chat, so dragging outward widens.
            let width = self.panel_width(panel, cx);
            let resizable = panel.width_range().is_some();
            if resizable && ix > chat_ix {
                row = row.child(self.resize_handle(panel, -1., cx));
            }
            let bounds = self.bounds.clone();
            let is_leaving = leaving == Some(panel);
            let mut cell = div()
                .relative()
                .h_full()
                .min_w_0()
                .overflow_hidden()
                .when(!is_leaving, |el| {
                    el.child(self.view(panel)).child(
                        canvas(
                            move |b, _, _| {
                                bounds.borrow_mut().insert(panel, b);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                })
                .when(is_leaving, |el| el.bg(t.sidebar));
            cell = match width {
                Some(w) => cell.flex_none().w(px(w)),
                None => cell.flex_1(),
            };
            if borders && ix + 1 < visible.len() {
                cell = cell.border_r_1().border_color(t.border);
            }
            if lifted {
                // The outline marks the panel's own spot only while the pointer is over it,
                // where letting go puts the panel back; elsewhere the drop indicator leads.
                let home = over_home;
                cell = cell
                    .opacity(if reduced_motion { 0.7 } else { 0.55 })
                    .when(home, |el| {
                        el.child(
                            div()
                                .absolute()
                                .inset_0()
                                .border_2()
                                .border_color(t.accent)
                                .rounded(t.radius_small),
                        )
                    });
            }
            let full = width.unwrap_or(240.);
            let cell = match (is_leaving, arriving.filter(|(p, _)| *p == panel)) {
                // Popping out: the emptied spot folds shut.
                (true, _) if !reduced_motion => cell
                    .with_animation(
                        gpui::ElementId::NamedInteger("leaving".into(), panel as u64),
                        Animation::new(POP_OUT).with_easing(gpui::ease_in_out),
                        move |el, d| el.w(px(full * (1. - d))).opacity(1. - d),
                    )
                    .into_any_element(),
                // Docking: the panel opens from nothing, fades in, and its outline glows
                // then fades so the eye lands on where it went.
                (false, Some((_, generation))) if !reduced_motion => cell
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .border_2()
                            .border_color(t.accent)
                            .rounded(t.radius_small)
                            .with_animation(
                                gpui::ElementId::NamedInteger("dock-glow".into(), generation),
                                Animation::new(DOCK_IN * 2).with_easing(gpui::ease_in_out),
                                |el, d| el.opacity(1. - d),
                            ),
                    )
                    .with_animation(
                        gpui::ElementId::NamedInteger("docking".into(), generation),
                        Animation::new(DOCK_IN).with_easing(gpui::ease_out_quint()),
                        move |el, d| el.w(px(full * d)).opacity(0.3 + 0.7 * d),
                    )
                    .into_any_element(),
                _ => cell.into_any_element(),
            };
            row = row.child(cell);
            if resizable && ix < chat_ix {
                row = row.child(self.resize_handle(panel, 1., cx));
            }
        }
        if slot == Some(slot_counter) {
            row = row.child(indicator());
        }

        let state = self.state.clone();
        let mut root = div()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .bg(t.base)
            .text_color(t.text)
            .font_family(theme(cx).ui_font.clone())
            .on_action(cx.listener(|this, _: &QuickSwitcher, _, cx| {
                this.state.update(cx, |s, cx| {
                    let open = matches!(s.overlay, Some(Overlay::QuickSwitcher));
                    s.set_overlay((!open).then_some(Overlay::QuickSwitcher), cx);
                });
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| {
                this.state.update(cx, |s, cx| {
                    s.set_overlay(Some(Overlay::Settings(SettingsPage::Account)), cx)
                });
            }))
            .on_action(cx.listener(|this, _: &PreviousChannel, _, cx| {
                this.state.update(cx, |s, cx| s.step_channel(-1, false, cx))
            }))
            .on_action(cx.listener(|this, _: &NextChannel, _, cx| {
                this.state.update(cx, |s, cx| s.step_channel(1, false, cx))
            }))
            .on_action(cx.listener(|this, _: &PreviousUnread, _, cx| {
                this.state.update(cx, |s, cx| s.step_channel(-1, true, cx))
            }))
            .on_action(cx.listener(|this, _: &NextUnread, _, cx| {
                this.state.update(cx, |s, cx| s.step_channel(1, true, cx))
            }))
            .on_action(cx.listener(|this, _: &PreviousServer, _, cx| {
                this.state.update(cx, |s, cx| s.step_guild(-1, cx))
            }))
            .on_action(cx.listener(|this, _: &NextServer, _, cx| {
                this.state.update(cx, |s, cx| s.step_guild(1, cx))
            }))
            .on_action(cx.listener(|this, _: &ToggleMute, _, cx| {
                this.state.update(cx, |s, cx| {
                    s.set_mute(!s.self_mute, s.self_deaf && !s.self_mute, cx)
                })
            }))
            .on_action(cx.listener(|this, _: &ToggleDeafen, _, cx| {
                this.state
                    .update(cx, |s, cx| s.set_mute(s.self_mute, !s.self_deaf, cx))
            }))
            .on_action(cx.listener(|this, _: &ToggleMemberList, _, cx| {
                this.state.update(cx, |s, cx| {
                    s.update_settings(cx, |st| {
                        let hidden = &mut st.layout.hidden;
                        if let Some(i) = hidden.iter().position(|p| *p == PanelId::Members) {
                            hidden.remove(i);
                        } else {
                            hidden.push(PanelId::Members);
                        }
                    })
                })
            }))
            .on_action(cx.listener(|this, _: &MarkRead, _, cx| {
                this.state.update(cx, |s, cx| {
                    if s.overlay.is_some() {
                        s.set_overlay(None, cx);
                    } else if let Some(c) = s.channel {
                        s.ack(c, cx);
                    }
                })
            }))
            .on_action(cx.listener(|this, _: &Search, _, cx| {
                this.state
                    .update(cx, |s, cx| s.set_overlay(Some(Overlay::QuickSwitcher), cx));
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom(Some(0.05), cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom(Some(-0.05), cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.zoom(None, cx)))
            .on_action(cx.listener(|this, _: &ResetLayout, _, cx| {
                this.state.update(cx, |s, cx| {
                    s.update_settings(cx, |st| st.layout = rcord_settings::Layout::default());
                    s.toast("Layout reset", cx);
                })
            }))
            // Right button held on a panel lifts it. Capture phase, so this sees the press
            // before any panel's own handlers, but it never stops a quick click.
            .capture_any_mouse_down(cx.listener(|this, e: &gpui::MouseDownEvent, _, cx| {
                if e.button == MouseButton::Right
                    && this.lift.is_none()
                    && this.state.read(cx).overlay.is_none()
                {
                    this.begin_hold(e.position, cx);
                } else if e.button == MouseButton::Left && this.lift.is_some() {
                    this.lift = None;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .capture_any_mouse_up(cx.listener(|this, e: &gpui::MouseUpEvent, window, cx| {
                match e.button {
                    MouseButton::Right => {
                        this.hold = None;
                        if this.lift.is_some() {
                            this.drop_panel(window, cx);
                            // The release ends the move; it must not also open a context menu.
                            cx.stop_propagation();
                        }
                    }
                    MouseButton::Left if this.resize.is_some() => {
                        this.commit_resize(cx);
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .on_mouse_move(
                cx.listener(|this, e: &gpui::MouseMoveEvent, _, cx| this.on_mouse_move(e, cx)),
            )
            .when(self.resize.is_some(), |el| el.cursor_col_resize())
            .when(lift.is_some(), |el| el.cursor_grabbing())
            .child(row);

        if let Some(l) = lift.filter(|l| detachable(l.panel)) {
            let zone = self.pop_zone.clone();
            root = root.child(
                div()
                    .absolute()
                    .top(px(12.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .relative()
                            .px(px(16.))
                            .h(px(40.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .rounded_full()
                            .border_2()
                            .border_color(if pop_target { t.accent } else { t.border })
                            .bg(if pop_target { t.accent } else { t.floating })
                            .text_color(if pop_target {
                                t.accent_text
                            } else {
                                t.text_muted
                            })
                            .shadow_md()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(icon(Icon::Screen).size(px(18.)))
                            .child(format!(
                                "Drop here to open the {} in its own window",
                                l.panel.label().to_lowercase()
                            ))
                            .child(
                                canvas(
                                    move |b, _, _| {
                                        *zone.borrow_mut() = Some(b);
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .inset_0(),
                            ),
                    ),
            );
        } else {
            *self.pop_zone.borrow_mut() = None;
        }
        if let Some(l) = lift {
            root = root.child(
                div()
                    .absolute()
                    .left(l.cursor.x + px(14.))
                    .top(l.cursor.y + px(10.))
                    .px(px(10.))
                    .py(px(5.))
                    .rounded(t.radius_small)
                    .bg(t.floating)
                    .border_1()
                    .border_color(t.border)
                    .shadow_md()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.header)
                    .child(l.panel.label()),
            );
        }

        if settings_page.is_some() {
            root = root.child(div().absolute().inset_0().child(self.settings.clone()));
        }
        let _ = state;
        root.child(self.overlays.clone())
    }
}
