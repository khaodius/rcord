//! The sign-in screen: a token field and a QR code for the Discord mobile app.

use gpui::{
    Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, Hsla, IntoElement, Render,
    Subscription, Task, Window, canvas, div, fill, point, prelude::*, px, size,
};
use rcord_auth::{RemoteAuth, RemoteAuthEvent, ScannedUser};

use crate::net::Net;
use crate::state::{AppState, StateEvent};
use crate::theme::theme;
use crate::ui::avatar;
use crate::ui::icons::{Icon, icon};
use crate::ui::text_input::{InputEvent, TextInput};

enum Qr {
    Starting,
    Code { modules: Vec<bool>, width: usize },
    Scanned(ScannedUser),
    Expired(String),
}

pub struct LoginView {
    state: Entity<AppState>,
    token: Entity<TextInput>,
    qr: Qr,
    _qr_task: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl LoginView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let token = cx.new(|cx| TextInput::new("Paste your token", cx).masked());
        let subs = vec![
            cx.subscribe_in(&token, window, |this, input, e, _, cx| {
                if matches!(e, InputEvent::Submit) {
                    let t = input.read(cx).text().to_owned();
                    this.submit(t, cx);
                }
            }),
            cx.subscribe(&state, |_, _, e, cx| {
                if matches!(e, StateEvent::Screen) {
                    cx.notify();
                }
            }),
            cx.observe(&state, |_, _, cx| cx.notify()),
        ];
        window.focus(&token.focus_handle(cx));
        let mut this = Self {
            state,
            token,
            qr: Qr::Starting,
            _qr_task: None,
            _subs: subs,
        };
        this.start_qr(cx);
        this
    }

    fn submit(&mut self, token: String, cx: &mut Context<Self>) {
        if token.trim().is_empty() {
            return;
        }
        self.state.update(cx, |s, cx| s.sign_in(token, cx));
    }

    fn start_qr(&mut self, cx: &mut Context<Self>) {
        self.qr = Qr::Starting;
        let net = cx.global::<Net>();
        let rest = net.anonymous();
        let (tx, rx) = async_channel::unbounded::<Result<RemoteAuthEvent, String>>();
        let run = net.run(async move {
            let result = match RemoteAuth::new() {
                Ok(auth) => {
                    let events = tx.clone();
                    auth.run(&rest, move |e| {
                        let _ = events.try_send(Ok(e));
                    })
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
                }
                Err(e) => Err(e.to_string()),
            };
            if let Err(e) = result {
                let _ = tx.try_send(Err(e));
            }
        });
        self._qr_task = Some(cx.spawn(async move |this, cx| {
            while let Ok(event) = rx.recv().await {
                let keep = this
                    .update(cx, |this, cx| {
                        this.on_qr(event, cx);
                        !matches!(this.qr, Qr::Expired(_))
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
            run.await;
        }));
        cx.notify();
    }

    fn on_qr(&mut self, event: Result<RemoteAuthEvent, String>, cx: &mut Context<Self>) {
        match event {
            Ok(RemoteAuthEvent::Code(url)) => match qrcode::QrCode::new(url.as_bytes()) {
                Ok(code) => {
                    let width = code.width();
                    let modules = code
                        .to_colors()
                        .into_iter()
                        .map(|c| c == qrcode::Color::Dark)
                        .collect();
                    self.qr = Qr::Code { modules, width };
                }
                Err(e) => self.qr = Qr::Expired(e.to_string()),
            },
            Ok(RemoteAuthEvent::Scanned(user)) => self.qr = Qr::Scanned(user),
            Ok(RemoteAuthEvent::Token(token)) => self.submit(token, cx),
            Err(e) => {
                tracing::info!("QR sign-in ended: {e}");
                self.qr = Qr::Expired(e);
            }
        }
        cx.notify();
    }

    fn qr_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let box_size = px(176.);
        let code = match &self.qr {
            Qr::Code { modules, width } => {
                let (modules, width) = (modules.clone(), *width);
                let dark: Hsla = gpui::black();
                div()
                    .size(box_size)
                    .p(px(8.))
                    .rounded(px(8.))
                    .bg(gpui::white())
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds: Bounds<gpui::Pixels>, _, window, _| {
                                let cell = bounds.size.width / width as f32;
                                for (ix, on) in modules.iter().enumerate() {
                                    if *on {
                                        let (x, y) = ((ix % width) as f32, (ix / width) as f32);
                                        let origin = point(
                                            bounds.origin.x + cell * x,
                                            bounds.origin.y + cell * y,
                                        );
                                        // A hair of overlap avoids seams between modules.
                                        window.paint_quad(fill(
                                            Bounds::new(
                                                origin,
                                                size(cell + px(0.3), cell + px(0.3)),
                                            ),
                                            dark,
                                        ));
                                    }
                                }
                            },
                        )
                        .size_full(),
                    )
                    .into_any_element()
            }
            Qr::Scanned(user) => {
                let url = match &user.avatar {
                    Some(h) => format!(
                        "https://cdn.discordapp.com/avatars/{}/{h}.png?size=160",
                        user.id
                    ),
                    None => "https://cdn.discordapp.com/embed/avatars/0.png".to_owned(),
                };
                div()
                    .size(box_size)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(avatar(url, px(120.), None))
                    .into_any_element()
            }
            Qr::Starting => div()
                .size(box_size)
                .rounded(px(8.))
                .bg(t.surface)
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(t.text_faint)
                .child("Preparing code…")
                .into_any_element(),
            Qr::Expired(_) => div()
                .id("qr-refresh")
                .size(box_size)
                .rounded(px(8.))
                .bg(t.surface)
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .cursor_pointer()
                .text_color(t.text_muted)
                .hover(|s| s.text_color(t.header))
                .child(icon(Icon::Logo).size(px(32.)))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Get a new code"),
                )
                .on_click(cx.listener(|this, _, _, cx| this.start_qr(cx)))
                .into_any_element(),
        };
        let (title, body) = match &self.qr {
            Qr::Scanned(u) => (
                "Check your phone".to_owned(),
                format!(
                    "Logging in as {}. Approve on your phone to continue.",
                    u.username
                ),
            ),
            Qr::Expired(reason) if reason.contains("expired") || reason.contains("cancelled") => (
                "Code expired".to_owned(),
                "Codes last a couple of minutes. Get a new one to try again.".to_owned(),
            ),
            Qr::Expired(reason) if reason.starts_with("could not connect") => (
                "Can't reach Discord".to_owned(),
                "Check your connection, then get a new code.".to_owned(),
            ),
            Qr::Expired(reason) => (
                "QR sign-in unavailable".to_owned(),
                format!("{reason}. Use a token, or try a new code."),
            ),
            _ => (
                "Log in with QR Code".to_owned(),
                "Scan this with the Discord mobile app to log in instantly.".to_owned(),
            ),
        };
        div()
            .w(px(240.))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(24.))
            .child(code)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(t.header)
                            .child(title),
                    )
                    .child(div().text_center().text_color(t.text_muted).child(body)),
            )
    }
}

impl Focusable for LoginView {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.token.focus_handle(cx)
    }
}

impl Render for LoginView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let s = self.state.read(cx);
        let error = s.login_error.clone();
        let busy = matches!(s.screen, crate::state::Screen::Connecting);
        let token = self.token.clone();
        let form = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .child(div().text_2xl().font_weight(FontWeight::BOLD).text_color(t.header).child("Welcome back!"))
                    .child(div().text_color(t.text_muted).child("We're so excited to see you again!")),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(if error.is_some() { t.danger } else { t.text_muted })
                            .child(match &error {
                                Some(e) => format!("TOKEN — {e}"),
                                None => "TOKEN".to_owned(),
                            }),
                    )
                    .child(
                        div()
                            .h(px(40.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .rounded(t.radius_small)
                            .bg(t.input)
                            .border_1()
                            .border_color(if error.is_some() { t.danger } else { t.border })
                            .child(self.token.clone()),
                    ),
            )
            .child(
                div()
                    .id("login-submit")
                    .h(px(44.))
                    .rounded(t.radius_small)
                    .bg(t.accent)
                    .hover(|st| st.bg(t.accent_hover))
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.accent_text)
                    .cursor_pointer()
                    .when(busy, |el| el.opacity(0.6))
                    .child(if busy { "Logging in…" } else { "Log In" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let text = token.read(cx).text().to_owned();
                        this.submit(text, cx);
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .text_xs()
                    .text_color(t.text_faint)
                    .child(icon(Icon::Shield).size(px(16.)).flex_none())
                    .child(div().flex_1().min_w_0().child("The token is checked against Discord, then stored in your system keychain. It never touches the settings file or the logs.")),
            );

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.sidebar)
            .font_family("Inter")
            .text_color(t.text)
            .child(
                div()
                    .w(px(784.))
                    .max_w_full()
                    .p(px(32.))
                    .rounded(t.radius)
                    .bg(t.base)
                    .border_1()
                    .border_color(t.border)
                    .shadow_lg()
                    .flex()
                    .gap(px(64.))
                    .items_center()
                    .child(form)
                    .child(div().w(px(1.)).h(px(260.)).bg(t.border))
                    .child(self.qr_panel(cx)),
            )
    }
}
