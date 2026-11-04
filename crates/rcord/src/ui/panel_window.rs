//! A panel popped out of the main window into its own. Closing the window, or its Dock
//! button, puts the panel back where it was, with the main window animating it in.

use gpui::{
    AnyView, Context, Entity, FontWeight, IntoElement, Render, Subscription, WeakEntity, Window,
    div, prelude::*, px,
};
use rcord_settings::PanelId;

use crate::image_cache::SharedImages;
use crate::state::{AppState, Screen, StateEvent};
use crate::theme::theme;
use crate::ui::Tooltip;
use crate::ui::icons::{Icon, icon};
use crate::ui::workspace::Workspace;

pub struct PanelWindow {
    panel: PanelId,
    view: AnyView,
    workspace: WeakEntity<Workspace>,
    _subs: Vec<Subscription>,
}

impl PanelWindow {
    pub fn new(
        state: Entity<AppState>,
        panel: PanelId,
        view: AnyView,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        window.set_rem_size(px(16.0 * state.read(cx).settings.appearance.ui_scale));
        let closing = workspace.clone();
        window.on_window_should_close(cx, move |_, cx| {
            closing.update(cx, |w, cx| w.docked(panel, cx)).ok();
            true
        });
        let subs = vec![
            cx.subscribe_in(&state, window, |_, state, event, window, cx| {
                match event {
                    StateEvent::Settings => {
                        let scale = state.read(cx).settings.appearance.ui_scale;
                        window.set_rem_size(px(16.0 * scale));
                        cx.notify();
                    }
                    // Signing out takes the main window's panels with it.
                    StateEvent::Screen if state.read(cx).screen != Screen::Main => {
                        window.remove_window();
                    }
                    _ => {}
                }
            }),
        ];
        Self {
            panel,
            view,
            workspace,
            _subs: subs,
        }
    }
}

impl Render for PanelWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let panel = self.panel;
        let workspace = self.workspace.clone();
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.base)
            .text_color(t.text)
            .font_family(t.ui_font.clone());
        if let Some(images) = cx.try_global::<SharedImages>() {
            root = root.image_cache(images.0.clone());
        }
        root.child(crate::ui::title_bar::resize_grips(window))
            .child(
                div()
                    .flex_none()
                    .h(px(32.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(t.border)
                    .child(
                        // The header moves the window, since it may have no system title bar.
                        div()
                            .id("panel-drag")
                            .flex_1()
                            .h_full()
                            .flex()
                            .items_center()
                            .when(cfg!(windows), |el| {
                                el.window_control_area(gpui::WindowControlArea::Drag)
                            })
                            .when(!cfg!(windows), |el| {
                                el.on_mouse_down(gpui::MouseButton::Left, |_, window, _| {
                                    window.start_window_move()
                                })
                            })
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(t.text_faint)
                            .child(panel.label().to_uppercase()),
                    )
                    .child(
                        div()
                            .id("dock")
                            .h(px(22.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .rounded(t.radius_small)
                            .cursor_pointer()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text_muted)
                            .hover(|s| s.bg(t.hover).text_color(t.header))
                            .tooltip(|_, cx| Tooltip::view("Put back in the main window", cx))
                            .child(icon(Icon::Logout).size(px(14.)))
                            .child("Dock")
                            .on_click(move |_, window, cx| {
                                workspace.update(cx, |w, cx| w.docked(panel, cx)).ok();
                                window.remove_window();
                            }),
                    ),
            )
            .child(div().flex_1().min_h_0().child(self.view.clone()))
    }
}
