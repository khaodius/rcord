//! The sign-in screen, laid out like Discord's: email or phone and password on the left with
//! two-factor codes after, the QR code for the mobile app on the right, and a token or
//! Discord's own page as other ways in. Captchas open in a small Discord window.

use std::sync::Arc;

use gpui::{
    AnyElement, Bounds, Context, ElementId, Entity, FocusHandle, Focusable, FontWeight, Hsla,
    IntoElement, KeyDownEvent, Render, SharedString, Subscription, Task, Window, canvas, div, fill,
    point, prelude::*, px, size,
};
use rcord_auth::{
    Captcha, CaptchaSolution, LoginError, LoginOutcome, MfaChallenge, MfaMethod, RemoteAuth,
    RemoteAuthEvent, RemoteAuthFinish, ScannedUser,
};

use crate::net::Net;
use crate::state::{AppState, StateEvent};
use crate::theme::{Theme, theme};
use crate::ui::avatar;
use crate::ui::icons::{Icon, icon};
use crate::ui::text_input::{InputEvent, TextInput};
use crate::webview::{self, WebResult};

const REGISTER_URL: &str = "https://discord.com/register";

enum Qr {
    Starting,
    Code { modules: Vec<bool>, width: usize },
    Scanned(ScannedUser),
    Expired(String),
}

enum QrMsg {
    Event(RemoteAuthEvent),
    Finished(Arc<RemoteAuth>, RemoteAuthFinish),
    Failed(String),
}

/// A request that may have to be sent again once a captcha is solved.
#[derive(Clone)]
enum Attempt {
    Password {
        login: String,
        password: String,
    },
    Mfa {
        method: MfaMethod,
        ticket: String,
        code: String,
    },
    Ticket {
        auth: Arc<RemoteAuth>,
        ticket: String,
    },
    Forgot {
        login: String,
    },
}

enum Done {
    Token(String),
    Mfa(MfaChallenge),
    ResetSent,
}

struct MfaStep {
    challenge: MfaChallenge,
    method: MfaMethod,
    sent_to: Option<String>,
}

enum Step {
    Password,
    Mfa(MfaStep),
    Token,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Busy {
    Idle,
    Working,
    Captcha,
    WebLogin,
}

pub struct LoginView {
    state: Entity<AppState>,
    email: Entity<TextInput>,
    password: Entity<TextInput>,
    code: Entity<TextInput>,
    token: Entity<TextInput>,
    step: Step,
    busy: Busy,
    error: Option<String>,
    notice: Option<String>,
    qr: Qr,
    _qr_task: Option<Task<()>>,
    _task: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

async fn perform(
    rest: rcord_rest::Client,
    attempt: Attempt,
    captcha: Option<CaptchaSolution>,
) -> Result<Done, LoginError> {
    let captcha = captcha.as_ref();
    match attempt {
        Attempt::Password { login, password } => Ok(
            match rcord_auth::login(&rest, &login, &password, captcha).await? {
                LoginOutcome::Token(t) => Done::Token(t),
                LoginOutcome::Mfa(c) => Done::Mfa(c),
            },
        ),
        Attempt::Mfa {
            method,
            ticket,
            code,
        } => rcord_auth::verify_mfa(&rest, method, &ticket, &code, captcha)
            .await
            .map(Done::Token),
        Attempt::Ticket { auth, ticket } => auth
            .exchange(&rest, &ticket, captcha)
            .await
            .map(Done::Token),
        Attempt::Forgot { login } => rcord_auth::forgot_password(&rest, &login, captcha)
            .await
            .map(|()| Done::ResetSent),
    }
}

impl LoginView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let email = cx.new(|cx| TextInput::new("", cx));
        let password = cx.new(|cx| TextInput::new("", cx).masked());
        let code = cx.new(|cx| TextInput::new("6-digit authentication code", cx));
        let token = cx.new(|cx| TextInput::new("Paste your token", cx).masked());
        let subs = vec![
            cx.subscribe_in(&email, window, |this, _, e, window, cx| {
                if matches!(e, InputEvent::Submit) {
                    window.focus(&this.password.focus_handle(cx));
                }
            }),
            cx.subscribe_in(&password, window, |this, _, e, _, cx| {
                if matches!(e, InputEvent::Submit) {
                    this.submit_password(cx);
                }
            }),
            cx.subscribe_in(&code, window, |this, _, e, _, cx| {
                if matches!(e, InputEvent::Submit) {
                    this.submit_code(cx);
                }
            }),
            cx.subscribe_in(&token, window, |this, _, e, _, cx| {
                if matches!(e, InputEvent::Submit) {
                    this.submit_token(cx);
                }
            }),
            cx.subscribe(&state, |_, _, e, cx| {
                if matches!(e, StateEvent::Screen) {
                    cx.notify();
                }
            }),
            cx.observe(&state, |_, _, cx| cx.notify()),
        ];
        window.focus(&email.focus_handle(cx));
        let mut this = Self {
            state,
            email,
            password,
            code,
            token,
            step: Step::Password,
            busy: Busy::Idle,
            error: None,
            notice: None,
            qr: Qr::Starting,
            _qr_task: None,
            _task: None,
            _subs: subs,
        };
        this.start_qr(cx);
        this
    }

    fn clear_messages(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.notice = None;
        self.state.update(cx, |s, _| s.login_error = None);
    }

    fn sign_in(&mut self, token: String, cx: &mut Context<Self>) {
        if token.trim().is_empty() {
            return;
        }
        self.state.update(cx, |s, cx| s.sign_in(token, cx));
    }

    fn submit_password(&mut self, cx: &mut Context<Self>) {
        if self.busy != Busy::Idle {
            return;
        }
        let login = self.email.read(cx).text().trim().to_owned();
        let password = self.password.read(cx).text().to_owned();
        self.clear_messages(cx);
        if login.is_empty() || password.is_empty() {
            self.error = Some("Enter your email or phone number and password.".into());
            cx.notify();
            return;
        }
        self.attempt(Attempt::Password { login, password }, None, cx);
    }

    fn submit_code(&mut self, cx: &mut Context<Self>) {
        if self.busy != Busy::Idle {
            return;
        }
        let Step::Mfa(step) = &self.step else { return };
        let code = self.code.read(cx).text().trim().to_owned();
        let (method, ticket) = (step.method, step.challenge.ticket.clone());
        self.clear_messages(cx);
        if code.is_empty() {
            self.error = Some("Enter the code.".into());
            cx.notify();
            return;
        }
        self.attempt(
            Attempt::Mfa {
                method,
                ticket,
                code,
            },
            None,
            cx,
        );
    }

    fn submit_token(&mut self, cx: &mut Context<Self>) {
        let text = self.token.read(cx).text().to_owned();
        self.clear_messages(cx);
        self.sign_in(text, cx);
    }

    fn forgot_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy != Busy::Idle {
            return;
        }
        let login = self.email.read(cx).text().trim().to_owned();
        self.clear_messages(cx);
        if login.is_empty() {
            self.error = Some("Enter your email or phone number first.".into());
            window.focus(&self.email.focus_handle(cx));
            cx.notify();
            return;
        }
        self.attempt(Attempt::Forgot { login }, None, cx);
    }

    fn attempt(
        &mut self,
        attempt: Attempt,
        captcha: Option<CaptchaSolution>,
        cx: &mut Context<Self>,
    ) {
        self.busy = Busy::Working;
        let net = cx.global::<Net>();
        let run = net.run(perform(net.anonymous(), attempt.clone(), captcha));
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = run.await;
            this.update(cx, |this, cx| this.finish(attempt, result, cx))
                .ok();
        }));
        cx.notify();
    }

    fn finish(
        &mut self,
        attempt: Attempt,
        result: Option<Result<Done, LoginError>>,
        cx: &mut Context<Self>,
    ) {
        self.busy = Busy::Idle;
        let qr = matches!(attempt, Attempt::Ticket { .. });
        let failure = match result {
            Some(Ok(Done::Token(token))) => {
                self.sign_in(token, cx);
                None
            }
            Some(Ok(Done::Mfa(challenge))) => {
                let method = if challenge.totp {
                    MfaMethod::Totp
                } else if challenge.sms {
                    MfaMethod::Sms
                } else {
                    MfaMethod::Backup
                };
                self.code.update(cx, |i, cx| i.clear(cx));
                self.step = Step::Mfa(MfaStep {
                    challenge,
                    method,
                    sent_to: None,
                });
                self.set_mfa_method(method, cx);
                None
            }
            Some(Ok(Done::ResetSent)) => {
                self.notice =
                    Some("Check your email or phone for a link to reset your password.".into());
                None
            }
            Some(Err(LoginError::Captcha(captcha))) => {
                self.solve_captcha(attempt, captcha, cx);
                None
            }
            Some(Err(LoginError::Rejected(m) | LoginError::Network(m))) => Some(m),
            None => Some("Something went wrong. Try again.".into()),
        };
        if let Some(message) = failure {
            if qr {
                self.qr = Qr::Expired(message);
            } else {
                self.error = Some(message);
            }
        }
        cx.notify();
    }

    fn solve_captcha(&mut self, attempt: Attempt, captcha: Captcha, cx: &mut Context<Self>) {
        self.busy = Busy::Captcha;
        let (sitekey, rqdata) = (captcha.sitekey.clone(), captcha.rqdata.clone());
        let run = cx.global::<Net>().run(async move {
            tokio::task::spawn_blocking(move || webview::solve_captcha(&sitekey, rqdata.as_deref()))
                .await
                .unwrap_or(WebResult::Closed)
        });
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = run.await.unwrap_or(WebResult::Closed);
            this.update(cx, |this, cx| {
                this.busy = Busy::Idle;
                match result {
                    WebResult::CaptchaKey(key) => {
                        this.attempt(attempt, Some(captcha.solved(key)), cx);
                    }
                    WebResult::Failed(m) => this.error = Some(m),
                    WebResult::Closed | WebResult::Token(_) => {
                        this.error = Some("Complete Discord's check to continue.".into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn web_login(&mut self, cx: &mut Context<Self>) {
        if self.busy != Busy::Idle {
            return;
        }
        self.clear_messages(cx);
        self.busy = Busy::WebLogin;
        let run = cx.global::<Net>().run(async {
            tokio::task::spawn_blocking(webview::web_login)
                .await
                .unwrap_or(WebResult::Closed)
        });
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = run.await.unwrap_or(WebResult::Closed);
            this.update(cx, |this, cx| {
                this.busy = Busy::Idle;
                match result {
                    WebResult::Token(token) => this.sign_in(token, cx),
                    WebResult::Failed(m) => this.error = Some(m),
                    WebResult::Closed | WebResult::CaptchaKey(_) => {}
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn set_mfa_method(&mut self, method: MfaMethod, cx: &mut Context<Self>) {
        let Step::Mfa(step) = &mut self.step else {
            return;
        };
        step.method = method;
        self.error = None;
        let placeholder = match method {
            MfaMethod::Totp => "6-digit authentication code",
            MfaMethod::Sms => "6-digit code",
            MfaMethod::Backup => "8-digit backup code",
        };
        self.code.update(cx, |i, cx| {
            i.clear(cx);
            i.set_placeholder(placeholder, cx);
        });
        if method == MfaMethod::Sms && step.sent_to.is_none() {
            let ticket = step.challenge.ticket.clone();
            let net = cx.global::<Net>();
            let rest = net.anonymous();
            let run = net.run(async move { rcord_auth::send_sms(&rest, &ticket).await });
            self._task = Some(cx.spawn(async move |this, cx| {
                let result = run.await;
                this.update(cx, |this, cx| {
                    match result {
                        Some(Ok(phone)) => {
                            if let Step::Mfa(step) = &mut this.step {
                                step.sent_to = Some(phone);
                            }
                        }
                        Some(Err(e)) => this.error = Some(e.to_string()),
                        None => {}
                    }
                    cx.notify();
                })
                .ok();
            }));
        }
        cx.notify();
    }

    fn go_to(&mut self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_messages(cx);
        let focus = match step {
            Step::Password => self.email.focus_handle(cx),
            Step::Mfa(_) => self.code.focus_handle(cx),
            Step::Token => self.token.focus_handle(cx),
        };
        self.step = step;
        window.focus(&focus);
        cx.notify();
    }

    fn start_qr(&mut self, cx: &mut Context<Self>) {
        self.qr = Qr::Starting;
        let net = cx.global::<Net>();
        let (tx, rx) = async_channel::unbounded::<QrMsg>();
        let run = net.run(async move {
            let auth = match tokio::task::spawn_blocking(RemoteAuth::new).await {
                Ok(Ok(auth)) => Arc::new(auth),
                Ok(Err(e)) => {
                    let _ = tx.try_send(QrMsg::Failed(e.to_string()));
                    return;
                }
                Err(_) => return,
            };
            let events = tx.clone();
            let result = auth
                .run(move |e| {
                    let _ = events.try_send(QrMsg::Event(e));
                })
                .await;
            let _ = tx.try_send(match result {
                Ok(finish) => QrMsg::Finished(auth.clone(), finish),
                Err(e) => QrMsg::Failed(e.to_string()),
            });
        });
        self._qr_task = Some(cx.spawn(async move |this, cx| {
            while let Ok(msg) = rx.recv().await {
                let keep = this
                    .update(cx, |this, cx| this.on_qr(msg, cx))
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
            run.await;
        }));
        cx.notify();
    }

    /// Applies one QR step; returns whether more are expected.
    fn on_qr(&mut self, msg: QrMsg, cx: &mut Context<Self>) -> bool {
        let more = match msg {
            QrMsg::Event(RemoteAuthEvent::Code(url)) => {
                match qrcode::QrCode::new(url.as_bytes()) {
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
                }
                true
            }
            QrMsg::Event(RemoteAuthEvent::Scanned(user)) => {
                self.qr = Qr::Scanned(user);
                true
            }
            QrMsg::Event(RemoteAuthEvent::Token(_)) => true,
            QrMsg::Finished(_, RemoteAuthFinish::Token(token)) => {
                self.sign_in(token, cx);
                false
            }
            QrMsg::Finished(auth, RemoteAuthFinish::Ticket(ticket)) => {
                self.attempt(Attempt::Ticket { auth, ticket }, None, cx);
                false
            }
            QrMsg::Failed(e) => {
                tracing::info!("QR sign-in ended: {e}");
                self.qr = Qr::Expired(e);
                false
            }
        };
        cx.notify();
        more
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
            Qr::Scanned(u) if self.busy == Busy::Captcha => (
                "Verify it's you".to_owned(),
                format!(
                    "Complete Discord's check in the window that opened to log in as {}.",
                    u.username
                ),
            ),
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
                "QR login didn't finish".to_owned(),
                format!("{reason} Get a new code to try again."),
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

    fn password_form(
        &self,
        t: &Theme,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = self.busy;
        div()
            .flex()
            .flex_col()
            .child(header(
                "Welcome back!",
                "We're so excited to see you again!".into(),
                t,
            ))
            .child(div().h(px(20.)))
            .child(label("EMAIL OR PHONE NUMBER", true, error.as_deref(), t))
            .child(field(self.email.clone(), error.is_some(), t))
            .child(div().h(px(20.)))
            .child(label("PASSWORD", true, None, t))
            .child(field(self.password.clone(), error.is_some(), t))
            .child(div().flex().pt(px(4.)).child(link(
                "forgot",
                "Forgot your password?",
                t,
                cx.listener(|this, _, window, cx| this.forgot_password(window, cx)),
            )))
            .child(div().h(px(20.)))
            .child(button(
                "login-submit",
                match busy {
                    Busy::Working => "Logging in…",
                    Busy::Captcha => "Waiting for the check…",
                    _ => "Log In",
                },
                busy != Busy::Idle,
                t,
                cx.listener(|this, _, _, cx| this.submit_password(cx)),
            ))
            .child(
                div()
                    .pt(px(8.))
                    .flex()
                    .gap(px(4.))
                    .text_sm()
                    .text_color(t.text_faint)
                    .child("Need an account?")
                    .child(link(
                        "register",
                        "Register",
                        t,
                        cx.listener(|_, _, _, cx| cx.open_url(REGISTER_URL)),
                    )),
            )
            .child(div().h(px(16.)))
            .child(self.other_ways(t, cx))
            .into_any_element()
    }

    fn other_ways(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_wrap()
            .gap(px(4.))
            .text_sm()
            .text_color(t.text_faint)
            .child("Or log in")
            .child(link(
                "web-login",
                if self.busy == Busy::WebLogin {
                    "on Discord's page (open)"
                } else {
                    "on Discord's page"
                },
                t,
                cx.listener(|this, _, _, cx| this.web_login(cx)),
            ))
            .child("or")
            .child(link(
                "token-login",
                "with a token",
                t,
                cx.listener(|this, _, window, cx| this.go_to(Step::Token, window, cx)),
            ))
    }

    fn mfa_form(
        &self,
        step: &MfaStep,
        t: &Theme,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (body, label_text) = match step.method {
            MfaMethod::Totp => (
                "Enter the 6-digit code from your authenticator app.".to_owned(),
                "ENTER DISCORD AUTH CODE",
            ),
            MfaMethod::Sms => (
                match &step.sent_to {
                    Some(p) => format!("We sent a code to {p}. Enter it below."),
                    None => "Sending a code to your phone…".to_owned(),
                },
                "ENTER SMS CODE",
            ),
            MfaMethod::Backup => (
                "Enter one of the 8-digit backup codes you saved when you turned on two-factor authentication."
                    .to_owned(),
                "ENTER BACKUP CODE",
            ),
        };
        let c = &step.challenge;
        let mut others: Vec<(MfaMethod, &'static str)> = Vec::new();
        if c.totp && step.method != MfaMethod::Totp {
            others.push((MfaMethod::Totp, "Use your authenticator app"));
        }
        if c.sms && step.method != MfaMethod::Sms {
            others.push((MfaMethod::Sms, "Receive auth code from SMS"));
        }
        if c.backup && step.method != MfaMethod::Backup {
            others.push((MfaMethod::Backup, "Use a backup code"));
        }
        let busy = self.busy;
        div()
            .flex()
            .flex_col()
            .child(header("Multi-Factor Authentication", body, t))
            .child(div().h(px(20.)))
            .child(label(label_text, false, error.as_deref(), t))
            .child(field(self.code.clone(), error.is_some(), t))
            .child(div().h(px(20.)))
            .child(button(
                "mfa-submit",
                match busy {
                    Busy::Working => "Checking…",
                    Busy::Captcha => "Waiting for the check…",
                    _ => "Log In",
                },
                busy != Busy::Idle,
                t,
                cx.listener(|this, _, _, cx| this.submit_code(cx)),
            ))
            .child(
                div()
                    .pt(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .children(others.into_iter().map(|(method, text)| {
                        link(
                            ElementId::Name(SharedString::new_static(text)),
                            text,
                            t,
                            cx.listener(move |this, _, window, cx| {
                                this.set_mfa_method(method, cx);
                                window.focus(&this.code.focus_handle(cx));
                            }),
                        )
                    }))
                    .child(link(
                        "mfa-back",
                        "Go Back to Login",
                        t,
                        cx.listener(|this, _, window, cx| this.go_to(Step::Password, window, cx)),
                    )),
            )
            .into_any_element()
    }

    fn token_form(&self, t: &Theme, error: Option<String>, cx: &mut Context<Self>) -> AnyElement {
        let busy = matches!(self.state.read(cx).screen, crate::state::Screen::Connecting);
        div()
            .flex()
            .flex_col()
            .child(header(
                "Log in with a token",
                "For accounts you already have a token for. Most people should use their password or the QR code instead.".into(),
                t,
            ))
            .child(div().h(px(20.)))
            .child(label("TOKEN", false, error.as_deref(), t))
            .child(field(self.token.clone(), error.is_some(), t))
            .child(div().h(px(20.)))
            .child(button(
                "token-submit",
                if busy { "Logging in…" } else { "Log In" },
                busy,
                t,
                cx.listener(|this, _, _, cx| this.submit_token(cx)),
            ))
            .child(
                div()
                    .pt(px(12.))
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .text_xs()
                    .text_color(t.text_faint)
                    .child(icon(Icon::Shield).size(px(16.)).flex_none())
                    .child(div().flex_1().min_w_0().child(
                        "A token is the full key to an account: never paste one someone else gave you. It is checked with Discord, then kept in your system keychain, never in the settings file or the logs.",
                    )),
            )
            .child(div().pt(px(12.)).flex().child(link(
                "token-back",
                "Go Back to Login",
                t,
                cx.listener(|this, _, window, cx| this.go_to(Step::Password, window, cx)),
            )))
            .into_any_element()
    }
}

fn header(title: &'static str, body: String, t: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .text_2xl()
                .font_weight(FontWeight::BOLD)
                .text_color(t.header)
                .text_center()
                .child(title),
        )
        .child(div().text_center().text_color(t.text_muted).child(body))
}

/// A field label in Discord's style, which turns red and carries the error when there is one.
fn label(text: &'static str, required: bool, error: Option<&str>, t: &Theme) -> impl IntoElement {
    div()
        .pb(px(8.))
        .flex()
        .flex_wrap()
        .gap(px(4.))
        .text_xs()
        .font_weight(FontWeight::BOLD)
        .text_color(if error.is_some() {
            t.danger
        } else {
            t.text_muted
        })
        .child(text)
        .when(required && error.is_none(), |el| {
            el.child(div().text_color(t.danger).child("*"))
        })
        .when_some(error, |el, e| {
            el.child(
                div()
                    .font_weight(FontWeight::NORMAL)
                    .italic()
                    .child(format!("- {e}")),
            )
        })
}

fn field(input: Entity<TextInput>, error: bool, t: &Theme) -> impl IntoElement {
    div()
        .h(px(40.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded(t.radius_small)
        .bg(t.input)
        .border_1()
        .border_color(if error { t.danger } else { t.border })
        .child(input)
}

fn button(
    id: &'static str,
    text: &'static str,
    busy: bool,
    t: &Theme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
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
        .child(text)
        .on_click(on_click)
}

fn link(
    id: impl Into<ElementId>,
    text: &'static str,
    t: &Theme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(id.into())
        .text_sm()
        .text_color(t.link)
        .cursor_pointer()
        .hover(|s| s.underline())
        .child(text)
        .on_click(on_click)
}

impl Focusable for LoginView {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        match self.step {
            Step::Password => self.email.focus_handle(cx),
            Step::Mfa(_) => self.code.focus_handle(cx),
            Step::Token => self.token.focus_handle(cx),
        }
    }
}

impl Render for LoginView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let error = self
            .error
            .clone()
            .or_else(|| self.state.read(cx).login_error.clone());
        let notice = self.notice.clone();
        let form = match &self.step {
            Step::Password => self.password_form(&t, error, cx),
            Step::Mfa(step) => self.mfa_form(step, &t, error, cx),
            Step::Token => self.token_form(&t, error, cx),
        };
        let show_qr = matches!(self.step, Step::Password);
        let email_focused = self.email.focus_handle(cx).is_focused(window);
        let password_focus = self.password.focus_handle(cx);
        let email_focus = self.email.focus_handle(cx);

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.sidebar)
            .font_family(theme(cx).ui_font.clone())
            .text_color(t.text)
            .on_key_down(move |e: &KeyDownEvent, window, cx| {
                // Tab moves between the two fields, as it does on Discord's page.
                if e.keystroke.key == "tab" {
                    if email_focused {
                        window.focus(&password_focus);
                    } else {
                        window.focus(&email_focus);
                    }
                    cx.stop_propagation();
                }
            })
            .child(
                div()
                    .w(px(if show_qr { 784. } else { 480. }))
                    .max_w_full()
                    .p(px(32.))
                    .rounded(t.radius)
                    .bg(t.base)
                    .border_1()
                    .border_color(t.border)
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex()
                            .gap(px(64.))
                            .items_center()
                            .child(div().flex_1().min_w_0().child(form))
                            .when(show_qr, |el| {
                                el.child(div().w(px(1.)).h(px(260.)).bg(t.border))
                                    .child(self.qr_panel(cx))
                            }),
                    )
                    .when_some(notice, |el, n| {
                        el.child(
                            div()
                                .p(px(10.))
                                .rounded(t.radius_small)
                                .bg(t.surface)
                                .text_sm()
                                .text_color(t.text)
                                .child(n),
                        )
                    }),
            )
    }
}
