//! A volume bar from 0 to 200%, as Discord uses for people and streams. Clicking or
//! dragging along it sets the volume; it snaps to 100% near the middle.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{App, Bounds, ElementId, MouseButton, Pixels, Stateful, canvas, div, prelude::*, px};

use crate::theme::Theme;

pub const MAX_VOLUME: f32 = 2.0;

pub fn volume_bar(
    id: impl Into<ElementId>,
    volume: f32,
    t: &Theme,
    on_change: Rc<dyn Fn(f32, &mut App)>,
) -> Stateful<gpui::Div> {
    let fraction = (volume / MAX_VOLUME).clamp(0., 1.);
    let bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::default();
    let pick = {
        let bounds = bounds.clone();
        move |x: Pixels| -> Option<f32> {
            let b = bounds.get()?;
            let f = (f32::from(x - b.origin.x) / f32::from(b.size.width)).clamp(0., 1.);
            let v = (f * MAX_VOLUME * 100.).round() / 100.;
            Some(if (v - 1.0).abs() < 0.03 { 1.0 } else { v })
        }
    };
    let pick_down = pick.clone();
    let (down, moved) = (on_change.clone(), on_change);
    div()
        .id(id)
        .h(px(20.))
        .flex()
        .items_center()
        .cursor_pointer()
        .relative()
        .child(
            div().w_full().h(px(6.)).rounded_full().bg(t.base).child(
                div()
                    .h_full()
                    .w(gpui::relative(fraction))
                    .rounded_full()
                    .bg(t.accent),
            ),
        )
        .child(
            div()
                .absolute()
                .left(gpui::relative(fraction))
                .ml(px(-7.))
                .size(px(14.))
                .rounded_full()
                .bg(gpui::white())
                .shadow_sm(),
        )
        .child(
            canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {})
                .absolute()
                .inset_0(),
        )
        .on_mouse_down(MouseButton::Left, move |e, _, cx| {
            cx.stop_propagation();
            if let Some(v) = pick_down(e.position.x) {
                down(v, cx);
            }
        })
        .on_mouse_move(move |e, _, cx| {
            if e.pressed_button == Some(MouseButton::Left)
                && let Some(v) = pick(e.position.x)
                && (v - volume).abs() >= 0.01
            {
                moved(v, cx);
            }
        })
}
