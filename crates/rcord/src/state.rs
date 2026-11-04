//! Application state: the signed-in session, the cache, what is selected, and the overlays
//! on top of the main window. Views subscribe to [`StateEvent`] and redraw only for the
//! changes they show.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{App, AppContext, Context, Entity, EventEmitter, Pixels, Point, Task};
use rcord_auth::Keychain;
use rcord_gateway::{
    Command, Config as GatewayConfig, Event as GatewayEvent, GuildSubscription,
    GuildSubscriptionsBulk, PresenceUpdate, Token,
};
use rcord_model::{Emoji, Snowflake, Status, UserProfile};
use rcord_rest::routes::{Around, CreateMessage, Reference};
use rcord_settings::{Paths, Settings};
use rcord_store::{Change, Store, Update};

use crate::net::Net;
use crate::theme::Theme;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// Checking the keychain for a saved session.
    Starting,
    Login,
    /// Signed in, waiting for READY.
    Connecting,
    Main,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connection {
    Offline,
    Connecting,
    Connected,
    Reconnecting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsPage {
    Account,
    Profile,
    Privacy,
    Appearance,
    Accessibility,
    Voice,
    Chat,
    Notifications,
    Keybinds,
    Language,
    Streamer,
    Layout,
    Advanced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpressionTab {
    Gifs,
    Stickers,
    Emoji,
}

#[derive(Clone)]
pub struct MenuItem {
    pub label: String,
    pub danger: bool,
    pub action: std::rc::Rc<dyn Fn(&mut App)>,
}

#[derive(Clone)]
pub enum Overlay {
    Settings(SettingsPage),
    Profile {
        user: Snowflake,
        guild: Option<Snowflake>,
        at: Point<Pixels>,
    },
    Menu {
        at: Point<Pixels>,
        items: Vec<MenuItem>,
    },
    Expressions(ExpressionTab),
    QuickSwitcher,
    /// Choosing what to share and at what quality.
    GoLive {
        sources: Vec<rcord_capture::Source>,
        selected: usize,
        preset: crate::golive::StreamPreset,
    },
}

#[derive(Debug, Clone)]
pub enum ProfileState {
    Loading,
    Loaded(Box<UserProfile>),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub text: String,
}

#[derive(Debug, Clone)]
pub enum StateEvent {
    Store(Change),
    Selection,
    Settings,
    Overlay,
    Screen,
}

#[derive(Debug, Clone, Copy)]
pub struct ReplyTo {
    pub channel: Snowflake,
    pub message: Snowflake,
    pub ping: bool,
}

pub struct AppState {
    pub settings: Settings,
    pub paths: Option<Paths>,
    pub screen: Screen,
    pub connection: Connection,
    pub latency: Option<Duration>,
    pub store: Store,
    /// `None` is the Direct Messages home.
    pub guild: Option<Snowflake>,
    pub channel: Option<Snowflake>,
    last_channel: HashMap<Option<Snowflake>, Snowflake>,
    pub collapsed_categories: HashSet<Snowflake>,
    pub overlay: Option<Overlay>,
    pub profiles: HashMap<(Snowflake, Option<Snowflake>), ProfileState>,
    pub toasts: Vec<Toast>,
    pub reply: Option<ReplyTo>,
    pub loading_history: HashSet<Snowflake>,
    /// Channels whose history is kept in memory, most recently opened first.
    recent_channels: std::collections::VecDeque<Snowflake>,
    pub self_mute: bool,
    pub self_deaf: bool,
    pub status: Status,
    pub(crate) gateway: Option<rcord_gateway::Handle>,
    /// The call this client is in, if any.
    pub call: Option<crate::voice::Call>,
    /// This client's own Go Live stream.
    pub go_live: Option<crate::golive::GoLive>,
    /// The stream this client is watching.
    pub watching: Option<crate::golive::Watching>,
    /// Voice participants whose profiles were requested, so each is asked for once.
    pub(crate) voice_members_requested: HashSet<Snowflake>,
    pump: Option<Task<()>>,
    subscribed: BTreeMap<Snowflake, Snowflake>,
    last_typing_sent: HashMap<Snowflake, u64>,
    save_task: Option<Task<()>>,
    next_toast: u64,
    pub login_error: Option<String>,
    /// The message the emoji picker adds a reaction to, when opened from a message.
    pub react_to: Option<(Snowflake, Snowflake)>,
    pub sticker_packs: Option<Vec<rcord_model::StickerPack>>,
    pub gifs: GifState,
}

#[derive(Debug, Default, Clone)]
pub struct GifState {
    pub query: String,
    pub loading: bool,
    pub results: Vec<rcord_rest::routes::Gif>,
    pub categories: Vec<rcord_rest::routes::GifCategory>,
}

impl EventEmitter<StateEvent> for AppState {}

/// Gateway output after parsing on the network thread.
enum NetEvent {
    Update(Box<Update>),
    Connecting {
        resuming: bool,
    },
    Disconnected {
        reason: String,
        reconnecting: bool,
        code: Option<u16>,
    },
    Latency(Duration),
    /// READY could not be read, so there is nothing to show.
    ReadyFailed(String),
}

impl AppState {
    pub fn new(settings: Settings, paths: Option<Paths>) -> Self {
        let cap = settings.chat.messages_per_channel as usize;
        Self {
            settings,
            paths,
            screen: Screen::Starting,
            connection: Connection::Offline,
            latency: None,
            store: Store::new(cap),
            guild: None,
            channel: None,
            last_channel: HashMap::new(),
            collapsed_categories: HashSet::new(),
            overlay: None,
            profiles: HashMap::new(),
            toasts: Vec::new(),
            reply: None,
            loading_history: HashSet::new(),
            recent_channels: std::collections::VecDeque::new(),
            self_mute: false,
            self_deaf: false,
            status: Status::Online,
            gateway: None,
            call: None,
            go_live: None,
            watching: None,
            voice_members_requested: HashSet::new(),
            pump: None,
            subscribed: BTreeMap::new(),
            last_typing_sent: HashMap::new(),
            save_task: None,
            next_toast: 0,
            login_error: None,
            react_to: None,
            sticker_packs: None,
            gifs: GifState::default(),
        }
    }

    // ---- session ------------------------------------------------------------------------

    /// Signs back in with the token saved for the last account, or shows the login screen.
    pub fn resume_session(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.settings.session.last_account.clone() else {
            self.set_screen(Screen::Login, cx);
            return;
        };
        let net = cx.global::<Net>();
        let lookup = net.run(async move {
            tokio::task::spawn_blocking(move || Keychain::default().load(&account))
                .await
                .ok()
                .and_then(|r| r.map_err(|e| tracing::warn!("{e}")).ok())
                .flatten()
        });
        cx.spawn(async move |this, cx| {
            let token = lookup.await.flatten();
            this.update(cx, |state, cx| match token {
                Some(token) => state.connect(token, cx),
                None => state.set_screen(Screen::Login, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Fills the cache with offline sample data instead of signing in. See [`crate::demo`].
    pub fn start_demo(&mut self, cx: &mut Context<Self>) {
        use crate::demo;
        let events = [
            ("READY", demo::ready()),
            ("READY_SUPPLEMENTAL", demo::presences()),
            ("GUILD_MEMBER_LIST_UPDATE", demo::member_list()),
        ];
        for (name, value) in events {
            let update = serde_json::value::to_raw_value(&value)
                .map_err(|e| e.to_string())
                .and_then(|raw| rcord_store::parse(name, &raw).map_err(|e| e.to_string()));
            match update {
                Ok(u) => self.handle_net(NetEvent::Update(Box::new(u)), cx),
                Err(e) => tracing::error!("demo {name}: {e}"),
            }
        }
        if let Some(general) = Snowflake::new(demo::GENERAL) {
            self.store
                .insert_history(general, demo::general_history(), 50, true);
            self.select_channel(general, cx);
        }
        // `RCORD_DEMO=settings` opens straight onto the appearance settings.
        if std::env::var("RCORD_DEMO").is_ok_and(|v| v == "settings") {
            self.set_overlay(Some(Overlay::Settings(SettingsPage::Appearance)), cx);
        }
        // `RCORD_DEMO=friends` opens on the friends page.
        if std::env::var("RCORD_DEMO").is_ok_and(|v| v == "friends") {
            self.show_friends(cx);
        }
    }

    /// Checks a token against `/users/@me`, saves it and connects.
    pub fn sign_in(&mut self, token: String, cx: &mut Context<Self>) {
        let token = token.trim().trim_matches('"').to_owned();
        if token.is_empty() {
            return;
        }
        self.login_error = None;
        let net = cx.global::<Net>();
        let client = match net.sign_in(&token) {
            Ok(c) => c,
            Err(e) => {
                self.login_error = Some(e.to_string());
                cx.notify();
                return;
            }
        };
        let check = net.run(async move { client.current_user().await });
        self.set_screen(Screen::Connecting, cx);
        cx.spawn(async move |this, cx| {
            let result = check.await;
            this.update(cx, |state, cx| match result {
                Some(Ok(user)) => {
                    let account = user.id.to_string();
                    let to_save = token.clone();
                    let save_account = account.clone();
                    let net = cx.global::<Net>();
                    net.spawn(async move {
                        tokio::task::spawn_blocking(move || {
                            Keychain::default().save(&save_account, &to_save)
                        })
                        .await
                    });
                    state.settings.session.last_account = Some(account);
                    state.save_settings(cx);
                    state.connect(token, cx);
                }
                Some(Err(e)) => {
                    cx.global::<Net>().sign_out();
                    state.login_error = Some(if e.is_unauthorized() {
                        "That token is not valid.".into()
                    } else {
                        e.to_string()
                    });
                    state.set_screen(Screen::Login, cx);
                }
                None => state.set_screen(Screen::Login, cx),
            })
            .ok();
        })
        .detach();
    }

    fn connect(&mut self, token: String, cx: &mut Context<Self>) {
        let net = cx.global::<Net>();
        if net.rest().is_none()
            && let Err(e) = net.sign_in(&token)
        {
            self.login_error = Some(e.to_string());
            self.set_screen(Screen::Login, cx);
            return;
        }
        let config = GatewayConfig {
            token: Token::new(token),
            properties: net.properties.clone(),
            presence: PresenceUpdate {
                status: self.status,
                ..PresenceUpdate::default()
            },
            capabilities: rcord_gateway::Capabilities::default(),
            url: rcord_gateway::DEFAULT_URL.to_owned(),
        };
        let (tx, rx) = async_channel::unbounded::<NetEvent>();
        let handle = {
            let _guard = net.handle().enter();
            let (handle, events) = rcord_gateway::spawn(config);
            net.handle().spawn(async move {
                while let Ok(event) = events.recv().await {
                    let out = match event {
                        GatewayEvent::Dispatch(d) => match rcord_store::parse(&d.name, &d.data) {
                            Ok(Update::Ignored) => continue,
                            Ok(update) => NetEvent::Update(Box::new(update)),
                            Err(e) if &*d.name == "READY" => {
                                tracing::error!("could not parse READY: {e}");
                                NetEvent::ReadyFailed(e.to_string())
                            }
                            Err(e) => {
                                tracing::warn!("could not parse {}: {e}", d.name);
                                continue;
                            }
                        },
                        GatewayEvent::Connecting { resuming } => NetEvent::Connecting { resuming },
                        GatewayEvent::Disconnected {
                            code,
                            reason,
                            reconnecting,
                        } => NetEvent::Disconnected {
                            reason,
                            reconnecting,
                            code,
                        },
                        GatewayEvent::Latency(d) => NetEvent::Latency(d),
                    };
                    if tx.send(out).await.is_err() {
                        break;
                    }
                }
            });
            handle
        };
        self.gateway = Some(handle);
        self.connection = Connection::Connecting;
        if self.screen != Screen::Main {
            self.set_screen(Screen::Connecting, cx);
        }
        self.pump = Some(cx.spawn(async move |this, cx| {
            while let Ok(first) = rx.recv().await {
                let ok = this.update(cx, |state, cx| {
                    state.handle_net(first, cx);
                    // Apply whatever else already arrived in one go, so a burst of
                    // events costs one redraw.
                    for _ in 0..512 {
                        match rx.try_recv() {
                            Ok(next) => state.handle_net(next, cx),
                            Err(_) => break,
                        }
                    }
                });
                if ok.is_err() {
                    break;
                }
            }
        }));
    }

    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        self.end_streams(cx);
        self.call = None;
        self.gateway = None;
        self.pump = None;
        cx.global::<Net>().sign_out();
        if let Some(account) = self.settings.session.last_account.take() {
            cx.global::<Net>().spawn(async move {
                tokio::task::spawn_blocking(move || Keychain::default().delete(&account)).await
            });
        }
        self.save_settings(cx);
        self.store = Store::new(self.settings.chat.messages_per_channel as usize);
        self.guild = None;
        self.channel = None;
        self.overlay = None;
        self.connection = Connection::Offline;
        self.set_screen(Screen::Login, cx);
    }

    fn handle_net(&mut self, event: NetEvent, cx: &mut Context<Self>) {
        match event {
            NetEvent::Update(update) => {
                self.voice_gateway_update(&update, cx);
                self.stream_gateway_update(&update, cx);
                let is_ready = matches!(*update, Update::Ready(_));
                let big = is_ready || matches!(*update, Update::ReadySupplemental(_));
                let change = self.store.apply(*update, now_ms());
                if big {
                    // Parsing READY leaves tens of megabytes of freed heap behind.
                    crate::memory::trim_soon();
                }
                if is_ready {
                    self.connection = Connection::Connected;
                    self.subscribed.clear();
                    self.loading_history.clear();
                    self.set_screen(Screen::Main, cx);
                    self.restore_selection(cx);
                }
                if matches!(
                    change,
                    Change::Voice(_) | Change::Everything | Change::Guild(_)
                ) {
                    self.request_voice_members();
                }
                if change != Change::None {
                    cx.emit(StateEvent::Store(change));
                }
            }
            NetEvent::Connecting { resuming } => {
                self.connection = if resuming {
                    Connection::Reconnecting
                } else {
                    Connection::Connecting
                };
                cx.emit(StateEvent::Screen);
            }
            NetEvent::Disconnected {
                reason,
                reconnecting,
                code,
            } => {
                tracing::info!("gateway disconnected ({code:?}): {reason}");
                if reconnecting {
                    self.connection = Connection::Reconnecting;
                    cx.emit(StateEvent::Screen);
                } else {
                    self.login_error = Some(if code == Some(4004) {
                        "Discord signed this session out. Sign in again.".into()
                    } else {
                        format!("Disconnected: {reason}")
                    });
                    self.sign_out(cx);
                }
            }
            NetEvent::Latency(d) => self.latency = Some(d),
            NetEvent::ReadyFailed(e) => {
                // The token is fine, so it stays in the keychain for the next start.
                self.gateway = None;
                self.pump = None;
                self.connection = Connection::Offline;
                self.login_error = Some(format!(
                    "Signed in, but rcord could not read your account data ({e}). Update rcord, or report this."
                ));
                self.set_screen(Screen::Login, cx);
            }
        }
    }

    fn set_screen(&mut self, screen: Screen, cx: &mut Context<Self>) {
        if self.screen != screen {
            self.screen = screen;
            cx.emit(StateEvent::Screen);
            cx.notify();
        }
    }

    fn restore_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(g) = self.guild
            && self.store.guild(g).is_none()
        {
            self.guild = None;
            self.channel = None;
        }
        if let Some(c) = self.channel
            && self.store.channel(c).is_none()
        {
            self.channel = None;
        }
        if let Some(c) = self.channel {
            self.ensure_history(c, cx);
            self.subscribe_current();
        }
        cx.emit(StateEvent::Selection);
    }

    // ---- selection ----------------------------------------------------------------------

    pub fn select_guild(&mut self, guild: Option<Snowflake>, cx: &mut Context<Self>) {
        if self.guild == guild && self.channel.is_some() {
            return;
        }
        self.guild = guild;
        let next = self
            .last_channel
            .get(&guild)
            .copied()
            .or_else(|| match guild {
                Some(g) => self
                    .store
                    .channel_groups(g)
                    .iter()
                    .flat_map(|grp| grp.channels.iter().copied())
                    .find(|c| {
                        self.store
                            .channel(*c)
                            .is_some_and(|ch| ch.kind.has_messages() && !ch.kind.is_voice())
                    }),
                None => None,
            });
        self.channel = next;
        self.reply = None;
        if let Some(c) = next {
            self.ensure_history(c, cx);
        }
        self.subscribe_current();
        cx.emit(StateEvent::Selection);
        cx.notify();
    }

    pub fn select_channel(&mut self, channel: Snowflake, cx: &mut Context<Self>) {
        let Some(c) = self.store.channel(channel) else {
            return;
        };
        if c.kind.is_voice() {
            self.join_voice(channel, cx);
            return;
        }
        self.guild = c.guild_id;
        self.channel = Some(channel);
        self.last_channel.insert(self.guild, channel);
        self.remember_recent(channel);
        self.reply = None;
        self.ensure_history(channel, cx);
        self.subscribe_current();
        self.ack(channel, cx);
        cx.emit(StateEvent::Selection);
        cx.notify();
    }

    /// Text channels of the current guild (or DMs) in sidebar order.
    fn navigable_channels(&self) -> Vec<Snowflake> {
        match self.guild {
            Some(g) => self
                .store
                .channel_groups(g)
                .iter()
                .filter(|grp| {
                    grp.category
                        .is_none_or(|c| !self.collapsed_categories.contains(&c))
                })
                .flat_map(|grp| grp.channels.iter().copied())
                .filter(|c| {
                    self.store
                        .channel(*c)
                        .is_some_and(|ch| ch.kind.has_messages() && !ch.kind.is_voice())
                })
                .collect(),
            None => self.store.private_channels().map(|c| c.id).collect(),
        }
    }

    /// Moves the selection `delta` channels up or down, optionally only between unread ones.
    pub fn step_channel(&mut self, delta: isize, unread_only: bool, cx: &mut Context<Self>) {
        let list = self.navigable_channels();
        if list.is_empty() {
            return;
        }
        let here = self.channel.and_then(|c| list.iter().position(|x| *x == c));
        let len = list.len() as isize;
        let mut ix = here.map_or(if delta > 0 { -1 } else { len }, |i| i as isize);
        for _ in 0..len {
            ix += delta.signum();
            if !(0..len).contains(&ix) {
                return;
            }
            let id = list[ix as usize];
            if !unread_only || self.store.is_unread(id) {
                self.select_channel(id, cx);
                return;
            }
        }
    }

    /// Moves to the next or previous server in the server list; Home sits before the first.
    pub fn step_guild(&mut self, delta: isize, cx: &mut Context<Self>) {
        let mut order: Vec<Option<Snowflake>> = vec![None];
        order.extend(self.store.guilds().map(|g| Some(g.guild.id)));
        let here = order.iter().position(|g| *g == self.guild).unwrap_or(0) as isize;
        let next = (here + delta.signum()).rem_euclid(order.len() as isize) as usize;
        self.select_guild(order[next], cx);
    }

    /// Tells the gateway which guild and channel are on screen, so it sends that guild's
    /// typing, presences and the member list for the channel.
    fn subscribe_current(&mut self) {
        let (Some(guild), Some(channel), Some(gw)) = (self.guild, self.channel, &self.gateway)
        else {
            return;
        };
        if self.subscribed.get(&guild) == Some(&channel) {
            return;
        }
        self.subscribed.insert(guild, channel);
        let mut sub = GuildSubscription {
            typing: true,
            activities: true,
            threads: true,
            member_updates: true,
            channels: BTreeMap::new(),
        };
        sub.channels.insert(channel, vec![[0, 99]]);
        gw.send(Command::SubscribeGuilds(GuildSubscriptionsBulk {
            subscriptions: [(guild, sub)].into(),
        }));
    }

    /// Asks for more of the member list once it scrolls near the end of what is loaded.
    pub fn request_member_range(&mut self, upto: u32) {
        let (Some(guild), Some(channel), Some(gw)) = (self.guild, self.channel, &self.gateway)
        else {
            return;
        };
        // The first hundred stay subscribed, as in Discord's client: dropping them makes Discord
        // invalidate the top of the list, which then shows blank until it is asked for again.
        let block = (upto / 100).max(1);
        let mut ranges: Vec<[u32; 2]> = vec![[0, 99]];
        ranges.extend((block..=block + 1).map(|b| [b * 100, b * 100 + 99]));
        let mut sub = GuildSubscription {
            typing: true,
            activities: true,
            threads: true,
            member_updates: true,
            channels: BTreeMap::new(),
        };
        sub.channels.insert(channel, ranges);
        gw.send(Command::SubscribeGuilds(GuildSubscriptionsBulk {
            subscriptions: [(guild, sub)].into(),
        }));
    }

    pub fn ensure_history(&mut self, channel: Snowflake, cx: &mut Context<Self>) {
        let loaded = self.store.messages(channel).is_some_and(|m| m.live);
        if !loaded {
            self.load_history(channel, Around::Latest, cx);
        }
    }

    pub fn load_older(&mut self, channel: Snowflake, cx: &mut Context<Self>) {
        let Some(list) = self.store.messages(channel) else {
            return;
        };
        if list.reached_start {
            return;
        }
        if let Some(oldest) = list.messages.front().map(|m| m.id) {
            self.load_history(channel, Around::Before(oldest), cx);
        }
    }

    fn load_history(&mut self, channel: Snowflake, at: Around, cx: &mut Context<Self>) {
        if !self.loading_history.insert(channel) {
            return;
        }
        let Some(rest) = cx.global::<Net>().rest() else {
            return;
        };
        const PAGE: u8 = 50;
        let fetch = cx
            .global::<Net>()
            .run(async move { rest.messages(channel, at, PAGE).await });
        cx.spawn(async move |this, cx| {
            let result = fetch.await;
            this.update(cx, |state, cx| {
                state.loading_history.remove(&channel);
                match result {
                    Some(Ok(page)) => {
                        state
                            .store
                            .insert_history(channel, page, PAGE, at == Around::Latest);
                        cx.emit(StateEvent::Store(Change::Messages(channel)));
                    }
                    Some(Err(e)) => {
                        if e.status().is_some_and(|s| s.as_u16() == 403) {
                            state.toast("You do not have access to this channel's history.", cx);
                        } else {
                            state.toast(format!("Could not load messages: {e}"), cx);
                        }
                    }
                    None => {}
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn ack(&mut self, channel: Snowflake, cx: &mut Context<Self>) {
        if !self.store.is_unread(channel) && self.store.read_state(channel).mentions == 0 {
            return;
        }
        let last = self.store.channel(channel).and_then(|c| c.last_message_id);
        self.store.mark_read(channel);
        cx.emit(StateEvent::Store(Change::ReadState(channel)));
        if let (Some(last), Some(rest)) = (last, cx.global::<Net>().rest()) {
            cx.global::<Net>()
                .spawn(async move { rest.ack(channel, last).await });
        }
    }

    /// Keeps the history of the last few channels and drops the rest, which is most of
    /// what grows while the app sits open. A dropped channel loads again when reopened.
    fn remember_recent(&mut self, channel: Snowflake) {
        const KEEP: usize = 8;
        self.recent_channels.retain(|c| *c != channel);
        self.recent_channels.push_front(channel);
        while self.recent_channels.len() > KEEP {
            if let Some(old) = self.recent_channels.pop_back()
                && !self.loading_history.contains(&old)
            {
                self.store.forget_messages(old);
            }
        }
    }

    /// Shows the friends page: Home with no conversation open.
    pub fn show_friends(&mut self, cx: &mut Context<Self>) {
        self.guild = None;
        self.channel = None;
        self.last_channel.remove(&None);
        self.reply = None;
        self.subscribe_current();
        cx.emit(StateEvent::Selection);
        cx.notify();
    }

    /// Opens the DM with `user`, creating it when there is none yet.
    pub fn open_dm(&mut self, user: Snowflake, cx: &mut Context<Self>) {
        let existing = self
            .store
            .private_channels()
            .find(|c| {
                c.kind == rcord_model::ChannelKind::Dm && c.recipient_ids().any(|r| r == user)
            })
            .map(|c| c.id);
        if let Some(id) = existing {
            self.select_channel(id, cx);
            return;
        }
        let Some(rest) = cx.global::<Net>().rest() else {
            return;
        };
        let task = cx
            .global::<Net>()
            .run(async move { rest.create_dm(user).await });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |s, cx| match result {
                Some(Ok(channel)) => {
                    let id = channel.id;
                    let change = s
                        .store
                        .apply(Update::ChannelCreate(Box::new(channel)), now_ms());
                    cx.emit(StateEvent::Store(change));
                    s.select_channel(id, cx);
                }
                Some(Err(e)) => s.toast(format!("Could not open the DM: {e}"), cx),
                None => {}
            })
            .ok();
        })
        .detach();
    }

    /// Accepts a friend request, removes a friend, ignores or cancels a request, or
    /// unblocks. The gateway's RELATIONSHIP_* event updates the list afterwards.
    pub fn set_friend(&mut self, user: Snowflake, add: bool, cx: &mut Context<Self>) {
        let Some(rest) = cx.global::<Net>().rest() else {
            return;
        };
        let task = cx.global::<Net>().run(async move {
            if add {
                rest.add_friend(user).await
            } else {
                rest.remove_relationship(user).await
            }
        });
        cx.spawn(async move |this, cx| {
            if let Some(Err(e)) = task.await {
                this.update(cx, |s, cx| {
                    s.toast(format!("Could not update the friend: {e}"), cx)
                })
                .ok();
            }
        })
        .detach();
    }

    /// Sends a friend request by username and reports the result to `done`.
    pub fn send_friend_request(
        &mut self,
        username: String,
        cx: &mut Context<Self>,
        done: impl FnOnce(Result<(), String>, &mut App) + 'static,
    ) {
        let Some(rest) = cx.global::<Net>().rest() else {
            done(Err("Not signed in.".into()), cx);
            return;
        };
        let task = cx
            .global::<Net>()
            .run(async move { rest.send_friend_request(&username).await });
        cx.spawn(async move |_, cx| {
            let result = match task.await {
                Some(Ok(())) => Ok(()),
                Some(Err(e)) => Err(e.to_string()),
                None => Err("Cancelled.".into()),
            };
            cx.update(|cx| done(result, cx)).ok();
        })
        .detach();
    }

    pub fn mark_guild_read(&mut self, guild: Snowflake, cx: &mut Context<Self>) {
        let ids: Vec<Snowflake> = self
            .store
            .guild(guild)
            .map(|g| g.channel_ids.clone())
            .unwrap_or_default();
        for c in ids {
            self.ack(c, cx);
        }
    }

    // ---- sending ------------------------------------------------------------------------

    pub fn send_message(&mut self, channel: Snowflake, content: String, cx: &mut Context<Self>) {
        let content = content.trim().to_owned();
        if content.is_empty() {
            return;
        }
        let Some(rest) = cx.global::<Net>().rest() else {
            return;
        };
        let nonce = Snowflake::from_timestamp_ms(now_ms())
            .map(|s| s.get() + u64::from(rand_suffix()))
            .unwrap_or_default()
            .to_string();
        let reply = self.reply.take().filter(|r| r.channel == channel);
        let guild = self.store.channel(channel).and_then(|c| c.guild_id);
        let body = CreateMessage {
            content: content.clone(),
            nonce: nonce.clone(),
            message_reference: reply.map(|r| Reference {
                message_id: r.message,
                channel_id: channel,
                guild_id: guild,
                fail_if_not_exists: false,
            }),
            allowed_mentions: reply
                .filter(|r| !r.ping)
                .map(|_| rcord_rest::routes::AllowedMentions::without_reply_ping()),
            ..CreateMessage::default()
        };
        self.store.add_pending(channel, nonce.clone(), content);
        self.last_typing_sent.remove(&channel);
        cx.emit(StateEvent::Store(Change::Messages(channel)));
        let send = cx
            .global::<Net>()
            .run(async move { rest.send_message(channel, &body).await });
        cx.spawn(async move |this, cx| {
            let result = send.await;
            this.update(cx, |state, cx| match result {
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    state.store.fail_pending(channel, &nonce);
                    state.toast(format!("Message not sent: {e}"), cx);
                    cx.emit(StateEvent::Store(Change::Messages(channel)));
                }
                None => {}
            })
            .ok();
        })
        .detach();
    }

    pub fn discard_pending(&mut self, channel: Snowflake, nonce: &str, cx: &mut Context<Self>) {
        self.store.remove_pending(channel, nonce);
        cx.emit(StateEvent::Store(Change::Messages(channel)));
    }

    pub fn typing(&mut self, channel: Snowflake, cx: &mut Context<Self>) {
        if !self.settings.privacy.send_typing {
            return;
        }
        let now = now_ms();
        if self
            .last_typing_sent
            .get(&channel)
            .is_some_and(|t| now - t < 8_000)
        {
            return;
        }
        self.last_typing_sent.insert(channel, now);
        if let Some(rest) = cx.global::<Net>().rest() {
            cx.global::<Net>()
                .spawn(async move { rest.trigger_typing(channel).await });
        }
    }

    pub fn toggle_reaction(
        &mut self,
        channel: Snowflake,
        message: Snowflake,
        emoji: Emoji,
        cx: &mut Context<Self>,
    ) {
        let Some(rest) = cx.global::<Net>().rest() else {
            return;
        };
        let mine = self
            .store
            .message(channel, message)
            .and_then(|m| {
                m.reactions.iter().find(|r| match (r.emoji.id, emoji.id) {
                    (Some(a), Some(b)) => a == b,
                    (None, None) => r.emoji.name == emoji.name,
                    _ => false,
                })
            })
            .is_some_and(|r| r.me);
        let task = cx.global::<Net>().run(async move {
            if mine {
                rest.remove_reaction(channel, message, &emoji).await
            } else {
                rest.add_reaction(channel, message, &emoji).await
            }
        });
        cx.spawn(async move |this, cx| {
            if let Some(Err(e)) = task.await {
                this.update(cx, |s, cx| s.toast(format!("Could not react: {e}"), cx))
                    .ok();
            }
        })
        .detach();
    }

    pub fn delete_message(
        &mut self,
        channel: Snowflake,
        message: Snowflake,
        cx: &mut Context<Self>,
    ) {
        if let Some(rest) = cx.global::<Net>().rest() {
            let task = cx
                .global::<Net>()
                .run(async move { rest.delete_message(channel, message).await });
            cx.spawn(async move |this, cx| {
                if let Some(Err(e)) = task.await {
                    this.update(cx, |s, cx| s.toast(format!("Could not delete: {e}"), cx))
                        .ok();
                }
            })
            .detach();
        }
    }

    pub fn set_status(&mut self, status: Status, cx: &mut Context<Self>) {
        self.status = status;
        if let Some(gw) = &self.gateway {
            gw.send(Command::UpdatePresence(PresenceUpdate {
                status,
                ..PresenceUpdate::default()
            }));
        }
        cx.emit(StateEvent::Store(Change::Me));
        cx.notify();
    }

    pub fn send_sticker(&mut self, channel: Snowflake, sticker: Snowflake, cx: &mut Context<Self>) {
        let Some(rest) = cx.global::<Net>().rest() else {
            return;
        };
        let body = CreateMessage {
            nonce: now_ms().to_string(),
            sticker_ids: vec![sticker],
            ..CreateMessage::default()
        };
        let task = cx
            .global::<Net>()
            .run(async move { rest.send_message(channel, &body).await });
        cx.spawn(async move |this, cx| {
            if let Some(Err(e)) = task.await {
                this.update(cx, |s, cx| s.toast(format!("Sticker not sent: {e}"), cx))
                    .ok();
            }
        })
        .detach();
    }

    pub fn load_sticker_packs(&mut self, cx: &mut Context<Self>) {
        if self.sticker_packs.is_some() {
            return;
        }
        self.sticker_packs = Some(Vec::new());
        let Some(rest) = cx.global::<Net>().rest() else {
            return;
        };
        let task = cx
            .global::<Net>()
            .run(async move { rest.sticker_packs().await });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |s, cx| {
                if let Some(Ok(packs)) = result {
                    s.sticker_packs = Some(packs);
                    cx.emit(StateEvent::Overlay);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Searches GIFs, or loads the trending set when `query` is empty.
    pub fn search_gifs(&mut self, query: String, cx: &mut Context<Self>) {
        let Some(rest) = cx.global::<Net>().rest() else {
            return;
        };
        let locale = self.settings.language.locale.clone();
        self.gifs.query = query.clone();
        self.gifs.loading = true;
        cx.notify();
        let task = cx.global::<Net>().run(async move {
            if query.trim().is_empty() {
                rest.trending_gifs(&locale)
                    .await
                    .map(|t| (t.gifs, t.categories))
            } else {
                rest.search_gifs(&query, &locale)
                    .await
                    .map(|g| (g, Vec::new()))
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |s, cx| {
                s.gifs.loading = false;
                match result {
                    Some(Ok((gifs, categories))) => {
                        s.gifs.results = gifs;
                        if !categories.is_empty() {
                            s.gifs.categories = categories;
                        }
                    }
                    Some(Err(e)) => s.toast(format!("GIF search failed: {e}"), cx),
                    None => {}
                }
                cx.emit(StateEvent::Overlay);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ---- profiles -----------------------------------------------------------------------

    pub fn open_profile(
        &mut self,
        user: Snowflake,
        guild: Option<Snowflake>,
        at: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.overlay = Some(Overlay::Profile { user, guild, at });
        let key = (user, guild);
        if !matches!(
            self.profiles.get(&key),
            Some(ProfileState::Loaded(_) | ProfileState::Loading)
        ) {
            self.profiles.insert(key, ProfileState::Loading);
            if let Some(rest) = cx.global::<Net>().rest() {
                let task = cx
                    .global::<Net>()
                    .run(async move { rest.user_profile(user, guild).await });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    this.update(cx, |s, cx| {
                        let state = match result {
                            Some(Ok(p)) => ProfileState::Loaded(Box::new(p)),
                            Some(Err(e)) => ProfileState::Failed(e.to_string()),
                            None => ProfileState::Failed("cancelled".into()),
                        };
                        s.profiles.insert(key, state);
                        cx.emit(StateEvent::Overlay);
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
        }
        cx.emit(StateEvent::Overlay);
        cx.notify();
    }

    // ---- overlays and toasts ------------------------------------------------------------

    pub fn set_overlay(&mut self, overlay: Option<Overlay>, cx: &mut Context<Self>) {
        self.overlay = overlay;
        cx.emit(StateEvent::Overlay);
        cx.notify();
    }

    pub fn toast(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.next_toast += 1;
        let id = self.next_toast;
        self.toasts.push(Toast {
            id,
            text: text.into(),
        });
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            this.update(cx, |s, cx| {
                s.toasts.retain(|t| t.id != id);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ---- settings -----------------------------------------------------------------------

    /// Changes settings, re-derives the theme, and saves shortly after the last change.
    pub fn update_settings(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Settings)) {
        f(&mut self.settings);
        self.settings.sanitize();
        self.voice_settings_changed();
        self.store
            .set_message_cap(self.settings.chat.messages_per_channel as usize);
        cx.set_global(Theme::from_settings(&self.settings.appearance));
        cx.emit(StateEvent::Settings);
        cx.notify();
        cx.refresh_windows();
        self.save_settings(cx);
    }

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.paths.as_ref().map(Paths::settings_file) else {
            return;
        };
        let settings = self.settings.clone();
        self.save_task = Some(cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            let result = cx
                .background_executor()
                .spawn(async move { settings.save(&path) })
                .await;
            if let Err(e) = result {
                tracing::error!("could not save settings: {e}");
            }
        }));
    }
}

fn rand_suffix() -> u16 {
    (now_ms() % 4096) as u16
}

pub fn init(settings: Settings, paths: Option<Paths>, cx: &mut App) -> Entity<AppState> {
    cx.set_global(Theme::from_settings(&settings.appearance));
    cx.new(|cx| {
        let mut state = AppState::new(settings, paths);
        if std::env::var_os("RCORD_DEMO").is_some() {
            state.start_demo(cx);
        } else {
            state.resume_session(cx);
        }
        state
    })
}
