//! rcord: a native Discord client.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod build_number;
mod demo;
mod image_cache;
mod memory;
mod net;
mod state;
mod theme;
mod tray;
mod ui;
mod update;
mod voice;
mod webview;

use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext, Application, Bounds, Context, Entity, FontWeight, Global, IntoElement, Render,
    Subscription, Task, TitlebarOptions, Window, WindowBounds, WindowOptions, div, prelude::*, px,
    size,
};
use rcord_settings::{Keybinds, Paths, Settings};

use crate::image_cache::LruImageCache;
use crate::net::{DEFAULT_BUILD_NUMBER, ImageHttp, Net};
use crate::state::{AppState, Connection, Screen, StateEvent};
use crate::theme::theme;
use crate::ui::icons::Assets;
use crate::ui::login::LoginView;
use crate::ui::workspace::Workspace;

/// The machine's UTC offset, read once at startup: reading it later, with threads running,
/// is not reliable on Linux.
#[derive(Clone, Copy)]
pub struct LocalTime(pub time::UtcOffset);

impl Global for LocalTime {}

const FONTS: [&[u8]; 6] = [
    include_bytes!("../../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/Inter-Bold.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-Bold.ttf"),
];

/// Replaces every key binding: text fields first, then the user's shortcuts.
pub fn rebind_keys(keybinds: &Keybinds, cx: &mut App) {
    cx.clear_key_bindings();
    ui::text_input::bind_keys(cx);
    ui::workspace::bind_keys(keybinds, cx);
}

enum UpdateUi {
    None,
    Available(update::Release),
    Installing(String),
    Failed(String),
}

struct Root {
    state: Entity<AppState>,
    images: Entity<LruImageCache>,
    login: Option<Entity<LoginView>>,
    workspace: Option<Entity<Workspace>>,
    update: UpdateUi,
    _update_task: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl Root {
    fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let budget = state.read(cx).settings.advanced.image_cache_mb as usize * 1024 * 1024;
        let images = LruImageCache::new(budget, cx);
        cx.set_global(crate::image_cache::SharedImages(images.clone()));
        let subs = vec![cx.subscribe_in(
            &state,
            window,
            |this, state, event, window, cx| match event {
                StateEvent::Screen => {
                    this.sync_screen(state.clone(), window, cx);
                    cx.notify();
                }
                StateEvent::Settings => {
                    let s = state.read(cx);
                    window.request_decorations(decorations(&s.settings));
                    let budget = s.settings.advanced.image_cache_mb as usize * 1024 * 1024;
                    window.set_rem_size(px(16.0 * s.settings.appearance.ui_scale));
                    this.images.update(cx, |c, _| c.set_budget(budget));
                }
                StateEvent::Store(_) if this.workspace.is_none() => cx.notify(),
                _ => {}
            },
        )];
        // Once per run: the window can be reopened from the tray.
        static CHECKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let check = state.read(cx).settings.advanced.check_for_updates
            && std::env::var_os("RCORD_DEMO").is_none()
            && !CHECKED.swap(true, std::sync::atomic::Ordering::Relaxed);
        let mut this = Self {
            state: state.clone(),
            images,
            login: None,
            workspace: None,
            update: UpdateUi::None,
            _update_task: None,
            _subs: subs,
        };
        this.sync_screen(state, window, cx);
        if check {
            this.check_for_update(cx);
        }
        this
    }

    fn sync_screen(
        &mut self,
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match state.read(cx).screen {
            Screen::Main => {
                if self.workspace.is_none() {
                    self.workspace = Some(cx.new(|cx| Workspace::new(state.clone(), window, cx)));
                }
                self.login = None;
            }
            Screen::Login => {
                // Signing out drops the workspace so the next account starts clean.
                self.workspace = None;
                if self.login.is_none() {
                    self.login = Some(cx.new(|cx| LoginView::new(state.clone(), window, cx)));
                }
            }
            Screen::Starting | Screen::Connecting => {}
        }
    }

    fn check_for_update(&mut self, cx: &mut Context<Self>) {
        let run = cx.global::<Net>().run(update::check());
        self._update_task = Some(cx.spawn(async move |this, cx| match run.await {
            Some(Ok(Some(release))) => {
                this.update(cx, |this, cx| {
                    this.update = UpdateUi::Available(release);
                    cx.notify();
                })
                .ok();
            }
            Some(Ok(None)) => {}
            Some(Err(e)) => tracing::info!("could not check for updates: {e}"),
            None => {}
        }));
    }

    fn install_update(&mut self, cx: &mut Context<Self>) {
        let UpdateUi::Available(release) = std::mem::replace(&mut self.update, UpdateUi::None)
        else {
            return;
        };
        self.update = UpdateUi::Installing(release.version.clone());
        let run = cx.global::<Net>().run(update::apply(release));
        self._update_task = Some(cx.spawn(async move |this, cx| {
            let result = run.await;
            this.update(cx, |this, cx| match result {
                Some(Ok(update::Applied::Restart(exe))) => {
                    match std::process::Command::new(&exe).spawn() {
                        Ok(_) => cx.quit(),
                        Err(e) => {
                            this.update = UpdateUi::Failed(format!(
                                "Updated, but could not restart rcord ({e}). Start it again yourself."
                            ));
                            cx.notify();
                        }
                    }
                }
                Some(Ok(update::Applied::Quit)) => cx.quit(),
                Some(Err(e)) => {
                    tracing::warn!("update failed: {e:#}");
                    this.update = UpdateUi::Failed(format!("The update did not install: {e}"));
                    cx.notify();
                }
                None => {
                    this.update = UpdateUi::Failed("The update did not install.".into());
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }

    fn update_prompt(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let t = theme(cx).clone();
        let (title, body, actions): (String, String, bool) = match &self.update {
            UpdateUi::None => return None,
            UpdateUi::Available(r) => (
                "Update available".into(),
                format!(
                    "rcord {} is out; you have {}. It installs in the background and rcord restarts on its own.",
                    r.version,
                    update::CURRENT
                ),
                true,
            ),
            UpdateUi::Installing(v) => (
                format!("Updating to {v}"),
                "Downloading and installing. rcord restarts when it is done.".into(),
                false,
            ),
            UpdateUi::Failed(m) => ("Update failed".into(), m.clone(), false),
        };
        let notes = match &self.update {
            UpdateUi::Available(r) => Some(r.url.clone()),
            _ => None,
        };
        let failed = matches!(self.update, UpdateUi::Failed(_));
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .h(px(36.))
                .px(px(16.))
                .flex()
                .items_center()
                .rounded(t.radius_small)
                .cursor_pointer()
                .font_weight(FontWeight::MEDIUM)
                .text_sm()
                .when(primary, |el| {
                    el.bg(t.accent)
                        .text_color(t.accent_text)
                        .hover(|s| s.bg(t.accent_hover))
                })
                .when(!primary, |el| {
                    el.text_color(t.text).hover(|s| s.underline())
                })
                .child(label)
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::hsla(0., 0., 0., 0.6))
                .font_family(theme(cx).ui_font.clone())
                .child(
                    div()
                        .id("update-prompt")
                        .w(px(440.))
                        .max_w_full()
                        .p(px(24.))
                        .rounded(t.radius)
                        .bg(t.base)
                        .border_1()
                        .border_color(t.border)
                        .shadow_lg()
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .text_xl()
                                .font_weight(FontWeight::BOLD)
                                .text_color(t.header)
                                .child(title),
                        )
                        .child(div().text_color(t.text_muted).child(body))
                        .when(actions, |el| {
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(t.text_faint)
                                    .child("Turn these checks off in Settings > Advanced."),
                            )
                        })
                        .child(
                            div()
                                .pt(px(8.))
                                .flex()
                                .justify_end()
                                .gap(px(8.))
                                .when_some(notes, |el, url| {
                                    el.child(
                                        button("update-notes", "What's new", false)
                                            .on_click(move |_, _, cx| cx.open_url(&url)),
                                    )
                                })
                                .when(actions, |el| {
                                    el.child(button("update-later", "Later", false).on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.update = UpdateUi::None;
                                            cx.notify();
                                        }),
                                    ))
                                    .child(
                                        button("update-now", "Update and restart", true).on_click(
                                            cx.listener(|this, _, _, cx| this.install_update(cx)),
                                        ),
                                    )
                                })
                                .when(failed, |el| {
                                    el.child(button("update-close", "Close", true).on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.update = UpdateUi::None;
                                            cx.notify();
                                        }),
                                    ))
                                }),
                        ),
                )
                .into_any_element(),
        )
    }

    fn splash(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let s = self.state.read(cx);
        let detail = match (&s.screen, s.connection) {
            (Screen::Starting, _) => "Starting",
            (_, Connection::Reconnecting) => "Reconnecting",
            _ => "Connecting",
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(16.))
            .bg(t.sidebar)
            .font_family(theme(cx).ui_font.clone())
            .child(
                gpui::img(std::sync::Arc::new(gpui::Image::from_bytes(
                    gpui::ImageFormat::Png,
                    include_bytes!("../../../assets/icons/rcord-256.png").to_vec(),
                )))
                .size(px(88.)),
            )
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(t.text_faint)
                    .child(detail.to_uppercase()),
            )
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = self.state.read(cx).screen.clone();
        let body = match screen {
            Screen::Main => self.workspace.clone().map(|w| w.into_any_element()),
            // The login screen stays up while a token is checked, so errors land beside it.
            Screen::Login => self.login.clone().map(|l| l.into_any_element()),
            Screen::Connecting
                if self.login.is_some()
                    && self.state.read(cx).connection == Connection::Offline =>
            {
                self.login.clone().map(|l| l.into_any_element())
            }
            _ => None,
        };
        let prompt = self.update_prompt(cx);
        let custom_bar = self.state.read(cx).settings.appearance.custom_title_bar;
        let bar = custom_bar.then(|| {
            let t = theme(cx).clone();
            ui::title_bar::title_bar("rcord", window, &t, Rc::new(close_main_window))
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .image_cache(self.images.clone())
            .children(bar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(body.unwrap_or_else(|| self.splash(cx).into_any_element()))
                    .children(prompt),
            )
            .when(custom_bar, |el| {
                el.child(ui::title_bar::resize_grips(window))
            })
    }
}

/// The app state, for reopening the main window from the tray.
pub struct MainState(pub Entity<AppState>);

impl Global for MainState {}

struct TraySender(async_channel::Sender<tray::TrayEvent>);

impl Global for TraySender {}

/// Shows the tray icon while "Close to tray" is on, and removes it when it is turned off.
fn sync_tray(cx: &mut App) {
    let wanted = cx
        .try_global::<MainState>()
        .is_some_and(|m| m.0.read(cx).settings.advanced.close_to_tray);
    let running = cx.has_global::<tray::Tray>();
    if wanted && !running {
        let tx = cx.global::<TraySender>().0.clone();
        if let Some(t) = tray::start(tx) {
            cx.set_global(t);
        }
    } else if !wanted && running {
        cx.remove_global::<tray::Tray>();
    }
}

/// rcord's own title bar asks the compositor for client-side decorations on Linux.
pub fn decorations(settings: &Settings) -> gpui::WindowDecorations {
    if settings.appearance.custom_title_bar && cfg!(target_os = "linux") {
        gpui::WindowDecorations::Client
    } else {
        gpui::WindowDecorations::Server
    }
}

/// Popped-out panels close with the main window.
fn close_panel_windows(cx: &mut App) {
    for w in cx.windows() {
        if let Some(p) = w.downcast::<ui::panel_window::PanelWindow>() {
            p.update(cx, |_, window, _| window.remove_window()).ok();
        }
    }
}

/// The close button of rcord's own title bar, which does what the system's would.
fn close_main_window(window: &mut Window, cx: &mut App) {
    close_panel_windows(cx);
    window.remove_window();
}

/// Brings the main window forward, opening it again if it was closed to the tray.
fn show_main_window(cx: &mut App) {
    if let Some(w) = cx.windows().into_iter().find_map(|w| w.downcast::<Root>()) {
        w.update(cx, |_, window, _| window.activate_window()).ok();
        return;
    }
    let Some(state) = cx.try_global::<MainState>().map(|m| m.0.clone()) else {
        return;
    };
    if let Err(e) = open_main_window(state, false, cx) {
        tracing::error!("could not open the window: {e}");
    }
}

fn open_main_window(
    state: Entity<AppState>,
    minimized: bool,
    cx: &mut App,
) -> anyhow::Result<gpui::WindowHandle<Root>> {
    let settings = &state.read(cx).settings;
    let scale = settings.appearance.ui_scale;
    let custom_bar = settings.appearance.custom_title_bar;
    let window_decorations = Some(decorations(settings));
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("rcord".into()),
            appears_transparent: custom_bar,
            ..Default::default()
        }),
        window_decorations,
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1280.), px(800.)),
            cx,
        ))),
        window_min_size: Some(size(px(940.), px(520.))),
        app_id: Some("rcord".into()),
        focus: !minimized,
        show: true,
        ..Default::default()
    };
    let handle = cx.open_window(options, move |window, cx| {
        window.set_rem_size(px(16.0 * scale));
        if minimized {
            window.minimize_window();
        }
        window.on_window_should_close(cx, |_, cx| {
            close_panel_windows(cx);
            true
        });
        cx.new(|cx| Root::new(state, window, cx))
    })?;
    cx.activate(true);
    Ok(handle)
}

/// Writes the panic message and a backtrace to `path` before the process ends, so a crash
/// on a desktop without a terminal still leaves something to report.
fn install_crash_log(path: std::path::PathBuf) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let text = format!(
            "rcord {} crashed at {}\n{info}\n\n{backtrace}\n",
            env!("CARGO_PKG_VERSION"),
            time::OffsetDateTime::now_utc(),
        );
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, text);
        default(info);
    }));
}

fn load_settings(paths: Option<&Paths>) -> Settings {
    let Some(p) = paths else {
        return Settings::default();
    };
    match Settings::load(&p.settings_file()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("{e}; starting with defaults");
            Settings::default()
        }
    }
}

fn main() {
    memory::init();
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("RCORD_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("rcord=info,warn")),
        )
        .init();

    let paths = Paths::detect();
    if let Some(p) = &paths {
        install_crash_log(p.config_dir.join("crash.log"));
    }
    let settings = load_settings(paths.as_ref());
    let demo = std::env::var_os("RCORD_DEMO").is_some();
    std::thread::Builder::new()
        .name("rcord-update-clean".into())
        .spawn(update::clean_up)
        .ok();
    let build = match settings.advanced.client_build_number {
        0 => paths
            .as_ref()
            .and_then(|p| build_number::cached(&p.cache_dir))
            .unwrap_or(DEFAULT_BUILD_NUMBER),
        n => n,
    };
    let net = match Net::new(build, &settings.language.locale) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("rcord: could not start networking: {e}");
            std::process::exit(1);
        }
    };
    if let Some(p) = paths.as_ref().filter(|_| !demo) {
        net.spawn(build_number::refresh(
            p.cache_dir.clone(),
            net::user_agent(),
        ));
    }
    let image_dir = paths.as_ref().map(Paths::image_cache);
    if let Some(dir) = image_dir.clone() {
        let max = u64::from(settings.advanced.disk_cache_mb) * 1024 * 1024;
        std::thread::Builder::new()
            .name("rcord-cache-prune".into())
            .spawn(move || net::prune_disk_cache(&dir, max))
            .ok();
    }
    let image_dir = image_dir.filter(|_| settings.advanced.disk_cache_mb > 0);
    let http = match ImageHttp::new(net.handle(), image_dir) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("rcord: could not start the image loader: {e}");
            std::process::exit(1);
        }
    };

    Application::new()
        .with_assets(Assets)
        .with_http_client(Arc::new(http))
        .run(move |cx: &mut App| {
            cx.set_global(LocalTime(offset));
            cx.set_global(net);
            if let Err(e) = cx
                .text_system()
                .add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect())
            {
                tracing::warn!("could not load bundled fonts: {e}");
            }
            rebind_keys(&settings.keybinds, cx);
            let minimized = settings.advanced.start_minimized;
            let state = state::init(settings, paths, cx);
            cx.set_global(MainState(state.clone()));
            let (tray_tx, tray_rx) = async_channel::unbounded();
            cx.set_global(TraySender(tray_tx));
            cx.spawn(async move |cx| {
                while let Ok(event) = tray_rx.recv().await {
                    let quit = cx
                        .update(|cx| match event {
                            tray::TrayEvent::Show => show_main_window(cx),
                            tray::TrayEvent::Quit => cx.quit(),
                        })
                        .is_err();
                    if quit {
                        break;
                    }
                }
            })
            .detach();
            sync_tray(cx);
            cx.observe(&state, |_, cx| sync_tray(cx)).detach();
            // With no main window left, rcord quits, unless it lives on in the tray.
            cx.on_window_closed(|cx| {
                let main_open = cx.windows().iter().any(|w| w.downcast::<Root>().is_some());
                if !main_open && !cx.has_global::<tray::Tray>() {
                    cx.quit();
                }
            })
            .detach();
            if let Err(e) = open_main_window(state, minimized, cx) {
                eprintln!("rcord: could not open a window: {e}");
                cx.quit();
            }
            cx.activate(true);
        });
}
