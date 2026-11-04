//! rcord: a native Discord client.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod build_number;
mod demo;
mod image_cache;
mod net;
mod state;
mod theme;
mod ui;
mod webview;

use std::borrow::Cow;
use std::sync::Arc;

use gpui::{
    App, AppContext, Application, Bounds, Context, Entity, FontWeight, Global, IntoElement, Render,
    Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use rcord_settings::{Keybinds, Paths, Settings};

use crate::image_cache::LruImageCache;
use crate::net::{DEFAULT_BUILD_NUMBER, ImageHttp, Net};
use crate::state::{AppState, Connection, Screen, StateEvent};
use crate::theme::theme;
use crate::ui::icons::{Assets, Icon, icon};
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

struct Root {
    state: Entity<AppState>,
    images: Entity<LruImageCache>,
    login: Option<Entity<LoginView>>,
    workspace: Option<Entity<Workspace>>,
    _subs: Vec<Subscription>,
}

impl Root {
    fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let budget = state.read(cx).settings.advanced.image_cache_mb as usize * 1024 * 1024;
        let images = LruImageCache::new(budget, cx);
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
                    let budget = s.settings.advanced.image_cache_mb as usize * 1024 * 1024;
                    window.set_rem_size(px(16.0 * s.settings.appearance.ui_scale));
                    this.images.update(cx, |c, _| c.set_budget(budget));
                }
                StateEvent::Store(_) if this.workspace.is_none() => cx.notify(),
                _ => {}
            },
        )];
        let mut this = Self {
            state: state.clone(),
            images,
            login: None,
            workspace: None,
            _subs: subs,
        };
        this.sync_screen(state, window, cx);
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
            .font_family("Inter")
            .child(icon(Icon::Logo).size(px(72.)).text_color(t.accent))
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        div()
            .size_full()
            .image_cache(self.images.clone())
            .child(body.unwrap_or_else(|| self.splash(cx).into_any_element()))
    }
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
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("RCORD_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("rcord=info,warn")),
        )
        .init();

    let paths = Paths::detect();
    let settings = load_settings(paths.as_ref());
    let demo = std::env::var_os("RCORD_DEMO").is_some();
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
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let scale = settings.appearance.ui_scale;
            let minimized = settings.advanced.start_minimized;
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("rcord".into()),
                    ..Default::default()
                }),
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
            let result = cx.open_window(options, move |window, cx| {
                window.set_rem_size(px(16.0 * scale));
                if minimized {
                    window.minimize_window();
                }
                let state = state::init(settings, paths, cx);
                cx.new(|cx| Root::new(state, window, cx))
            });
            if let Err(e) = result {
                eprintln!("rcord: could not open a window: {e}");
                cx.quit();
            }
            cx.activate(true);
        });
}
