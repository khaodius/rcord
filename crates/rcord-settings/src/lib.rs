//! Every user setting, with defaults, range checks and an atomic TOML file. Missing or
//! unknown keys never fail a load: the file from an older or newer version still opens.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod theme_file;
pub use theme_file::{COLOR_KEYS, ColorKey, ThemeError, ThemeFile, parse_color};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Which of the one-time upgrades in [`Settings::from_toml`] this file has had. A
    /// file without it is from before revisions, so it reads as 0.
    #[serde(default)]
    pub revision: u32,
    pub appearance: Appearance,
    pub accessibility: Accessibility,
    pub chat: Chat,
    pub notifications: Notifications,
    pub privacy: Privacy,
    pub voice: Voice,
    pub stream: Stream,
    pub keybinds: Keybinds,
    pub layout: Layout,
    pub advanced: Advanced,
    pub language: Language,
    pub session: Session,
}

const REVISION: u32 = 1;

impl Default for Settings {
    fn default() -> Self {
        Self {
            revision: REVISION,
            appearance: Appearance::default(),
            accessibility: Accessibility::default(),
            chat: Chat::default(),
            notifications: Notifications::default(),
            privacy: Privacy::default(),
            voice: Voice::default(),
            stream: Stream::default(),
            keybinds: Keybinds::default(),
            layout: Layout::default(),
            advanced: Advanced::default(),
            language: Language::default(),
            session: Session::default(),
        }
    }
}

/// Which account to sign back in to. The token itself is in the system keychain.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub last_account: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeId {
    #[default]
    Dark,
    /// Pure black surfaces for OLED screens.
    Midnight,
    Ash,
    Light,
    /// The theme in `appearance.custom_theme`.
    Custom,
}

impl ThemeId {
    pub const ALL: [Self; 4] = [Self::Dark, Self::Midnight, Self::Ash, Self::Light];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Midnight => "Midnight",
            Self::Ash => "Ash",
            Self::Light => "Light",
            Self::Custom => "Custom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MessageDisplay {
    #[default]
    Cozy,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Animate {
    Always,
    #[default]
    OnHover,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RoleColors {
    #[default]
    InNames,
    BesideNames,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: ThemeId,
    /// Accent color as 0xRRGGBB.
    pub accent: u32,
    /// Interface zoom, 0.75 to 1.5.
    pub ui_scale: f32,
    /// Chat font size in points, 12 to 24.
    pub chat_font_size: f32,
    pub message_display: MessageDisplay,
    /// Space between message groups in pixels, 0 to 24.
    pub group_spacing: f32,
    pub show_avatars_in_compact: bool,
    /// Corner radius of panels, cards and inputs, 0 to 16.
    pub corner_radius: f32,
    /// Color saturation, 0 (grey) to 1.
    pub saturation: f32,
    pub animate_emoji: Animate,
    pub animate_avatars: Animate,
    pub role_colors: RoleColors,
    pub hour24: bool,
    pub ui_font: Option<String>,
    pub code_font: Option<String>,
    /// Shows server names under their icons in the server list.
    pub server_names: bool,
    /// Draws a one-pixel separator between panels instead of relying on color alone.
    pub panel_borders: bool,
    /// The custom theme, used when `theme` is `custom`.
    pub custom_theme: Option<ThemeFile>,
    /// Draws rcord's own title bar with minimize, maximize and close instead of the system's.
    pub custom_title_bar: bool,
    /// What the button at the top of the server list shows.
    pub home_icon: HomeIcon,
    /// The picture for [`HomeIcon::Image`].
    pub home_icon_path: Option<String>,
}

/// The picture on the Home button at the top of the server list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HomeIcon {
    #[default]
    Logo,
    Friends,
    Avatar,
    Image,
}

impl HomeIcon {
    pub const ALL: [Self; 4] = [Self::Logo, Self::Friends, Self::Avatar, Self::Image];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Logo => "rcord logo",
            Self::Friends => "Friends",
            Self::Avatar => "Your avatar",
            Self::Image => "A picture",
        }
    }
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: ThemeId::Dark,
            accent: 0x5865F2,
            ui_scale: 1.0,
            chat_font_size: 16.0,
            message_display: MessageDisplay::Cozy,
            group_spacing: 16.0,
            show_avatars_in_compact: false,
            corner_radius: 8.0,
            saturation: 1.0,
            animate_emoji: Animate::Always,
            animate_avatars: Animate::OnHover,
            role_colors: RoleColors::InNames,
            hour24: false,
            ui_font: None,
            code_font: None,
            server_names: false,
            panel_borders: false,
            custom_theme: None,
            // Wayland compositors such as GNOME draw no title bar for rcord at all, so Linux
            // gets rcord's own by default; Windows keeps the system one.
            custom_title_bar: cfg!(target_os = "linux"),
            home_icon: HomeIcon::Logo,
            home_icon_path: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Accessibility {
    pub reduced_motion: bool,
    pub underline_links: bool,
    /// Raises the contrast of role colors against the background.
    pub role_color_contrast: bool,
    pub always_show_focus: bool,
    pub tts_rate: f32,
}

impl Default for Accessibility {
    fn default() -> Self {
        Self {
            reduced_motion: false,
            underline_links: true,
            role_color_contrast: false,
            always_show_focus: false,
            tts_rate: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Spoilers {
    #[default]
    OnClick,
    InServersIModerate,
    Always,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chat {
    pub inline_attachments: bool,
    pub inline_links: bool,
    pub link_previews: bool,
    pub show_reactions: bool,
    pub convert_emoticons: bool,
    pub gif_autoplay: bool,
    pub sticker_autoplay: bool,
    pub jumbo_emoji: bool,
    pub spoilers: Spoilers,
    /// Tallest an image is drawn inline, in pixels.
    pub max_media_height: f32,
    pub send_on_enter: bool,
    pub reply_ping_default: bool,
    /// Messages kept in memory per channel. Older ones reload on scroll.
    pub messages_per_channel: u32,
}

impl Default for Chat {
    fn default() -> Self {
        Self {
            inline_attachments: true,
            inline_links: true,
            link_previews: true,
            show_reactions: true,
            convert_emoticons: true,
            gif_autoplay: true,
            sticker_autoplay: true,
            jumbo_emoji: true,
            spoilers: Spoilers::OnClick,
            max_media_height: 350.0,
            send_on_enter: true,
            reply_ping_default: true,
            messages_per_channel: 250,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub desktop: bool,
    pub sounds: bool,
    pub unread_badge: bool,
    pub flash_taskbar: bool,
    pub suppress_while_focused: bool,
    pub message_previews: bool,
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            desktop: true,
            sounds: true,
            unread_badge: true,
            flash_taskbar: true,
            suppress_while_focused: true,
            message_previews: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Privacy {
    pub send_typing: bool,
    pub show_activity: bool,
    pub streamer_mode: StreamerMode,
    /// Hides your own message embeds' link previews from your view.
    pub blur_explicit_media: bool,
}

impl Default for Privacy {
    fn default() -> Self {
        Self {
            send_typing: true,
            show_activity: true,
            streamer_mode: StreamerMode::default(),
            blur_explicit_media: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamerMode {
    pub enabled: bool,
    pub hide_personal_info: bool,
    pub hide_invite_links: bool,
    pub disable_sounds: bool,
    pub disable_notifications: bool,
}

impl Default for StreamerMode {
    fn default() -> Self {
        Self {
            enabled: false,
            hide_personal_info: true,
            hide_invite_links: true,
            disable_sounds: true,
            disable_notifications: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InputMode {
    #[default]
    VoiceActivity,
    PushToTalk,
}

/// A preset for the microphone, as Discord offers them. Only `Custom` uses the individual
/// processing settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InputProfile {
    /// Strong noise suppression, automatic gain and automatic sensitivity.
    #[default]
    VoiceIsolation,
    /// No processing: the microphone as it is, transmitted whenever it picks anything up.
    Studio,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoiseSuppression {
    Off,
    #[default]
    Standard,
    Strong,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Voice {
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    /// Gain applied to the microphone, 0 to 2.
    pub input_volume: f32,
    pub output_volume: f32,
    pub input_mode: InputMode,
    pub input_profile: InputProfile,
    /// Detect the speaking threshold automatically.
    pub auto_sensitivity: bool,
    /// Manual threshold in dBFS, -100 to 0.
    pub sensitivity_db: f32,
    pub push_to_talk_key: Option<String>,
    pub push_to_talk_release_ms: u32,
    pub noise_suppression: NoiseSuppression,
    pub echo_cancellation: bool,
    pub automatic_gain: bool,
    /// Lowers other apps while someone speaks, 0 to 1.
    pub attenuation: f32,
    /// Opus bitrate cap in kbit/s; the channel's bitrate applies when lower.
    pub max_bitrate_kbps: u32,
    /// Volume for each person, by user ID, 0 to 2; people not listed play at 1.
    pub user_volumes: BTreeMap<String, f32>,
    /// People muted for you alone, by user ID.
    pub muted_users: Vec<String>,
    /// Volume of each person's stream sound, by user ID, 0 to 2.
    pub stream_volumes: BTreeMap<String, f32>,
}

impl Voice {
    /// How loud `user` plays for you, counting a local mute.
    pub fn user_volume(&self, user: &str) -> f32 {
        if self.muted_users.iter().any(|u| u == user) {
            return 0.0;
        }
        self.user_volumes.get(user).copied().unwrap_or(1.0)
    }
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            input_device: None,
            output_device: None,
            input_volume: 1.0,
            output_volume: 1.0,
            input_mode: InputMode::VoiceActivity,
            input_profile: InputProfile::VoiceIsolation,
            auto_sensitivity: true,
            sensitivity_db: -50.0,
            push_to_talk_key: None,
            push_to_talk_release_ms: 20,
            noise_suppression: NoiseSuppression::Standard,
            echo_cancellation: true,
            automatic_gain: true,
            attenuation: 0.0,
            max_bitrate_kbps: 128,
            user_volumes: BTreeMap::new(),
            muted_users: Vec::new(),
            stream_volumes: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Encoder {
    #[default]
    Auto,
    Hardware,
    Software,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stream {
    /// Vertical resolution; 0 streams at the source's resolution.
    pub resolution: u32,
    pub fps: u32,
    pub include_audio: bool,
    pub encoder: Encoder,
    /// Codec preference, best first, from `av1`, `h265`, `h264`, `vp9`, `vp8`.
    pub codecs: Vec<String>,
    /// Shows the cursor in screen shares.
    pub capture_cursor: bool,
}

impl Default for Stream {
    fn default() -> Self {
        Self {
            resolution: 1080,
            fps: 60,
            include_audio: true,
            encoder: Encoder::Auto,
            codecs: ["av1", "h264", "vp8"].map(String::from).to_vec(),
            capture_cursor: true,
        }
    }
}

/// Action name to key chord, in GPUI's notation (`ctrl-k`, `alt-shift-up`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Keybinds(pub BTreeMap<String, String>);

impl Default for Keybinds {
    fn default() -> Self {
        let pairs = [
            ("quick-switcher", "ctrl-k"),
            ("settings", "ctrl-,"),
            ("previous-channel", "alt-up"),
            ("next-channel", "alt-down"),
            ("previous-unread", "alt-shift-up"),
            ("next-unread", "alt-shift-down"),
            ("previous-server", "ctrl-alt-up"),
            ("next-server", "ctrl-alt-down"),
            ("toggle-mute", "ctrl-shift-m"),
            ("toggle-deafen", "ctrl-shift-d"),
            ("toggle-member-list", "ctrl-u"),
            ("mark-read", "escape"),
            ("search", "ctrl-f"),
            ("zoom-in", "ctrl-="),
            ("zoom-out", "ctrl--"),
            ("zoom-reset", "ctrl-0"),
            ("reset-layout", "ctrl-alt-shift-l"),
        ];
        Self(
            pairs
                .into_iter()
                .map(|(a, k)| (a.to_owned(), k.to_owned()))
                .collect(),
        )
    }
}

/// The panels of the main window, which can be reordered with a right-click hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PanelId {
    Servers,
    Channels,
    Chat,
    Members,
}

impl PanelId {
    pub const ALL: [Self; 4] = [Self::Servers, Self::Channels, Self::Chat, Self::Members];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Servers => "Server list",
            Self::Channels => "Channel list",
            Self::Chat => "Chat",
            Self::Members => "Member list",
        }
    }

    /// Default and allowed widths in pixels. Chat takes the remaining space.
    pub const fn width_range(self) -> Option<(f32, f32, f32)> {
        match self {
            Self::Servers => None,
            Self::Channels => Some((240.0, 180.0, 420.0)),
            Self::Chat => None,
            Self::Members => Some((248.0, 200.0, 420.0)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    /// Left to right.
    pub order: Vec<PanelId>,
    pub hidden: Vec<PanelId>,
    pub widths: BTreeMap<PanelId, f32>,
    /// Server IDs in the order dragged into the server list. Servers not listed follow in
    /// Discord's order.
    pub guild_order: Vec<u64>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            order: PanelId::ALL.to_vec(),
            hidden: Vec::new(),
            widths: BTreeMap::new(),
            guild_order: Vec::new(),
        }
    }
}

impl Layout {
    pub fn width(&self, panel: PanelId) -> Option<f32> {
        let (default, min, max) = panel.width_range()?;
        Some(
            self.widths
                .get(&panel)
                .copied()
                .unwrap_or(default)
                .clamp(min, max),
        )
    }

    pub fn is_visible(&self, panel: PanelId) -> bool {
        !self.hidden.contains(&panel)
    }

    /// Moves `panel` to `index` in the order.
    pub fn move_panel(&mut self, panel: PanelId, index: usize) {
        if let Some(from) = self.order.iter().position(|p| *p == panel) {
            self.order.remove(from);
            self.order.insert(index.min(self.order.len()), panel);
        }
    }

    /// Repairs an order with missing or repeated panels, keeping the user's arrangement.
    fn sanitize(&mut self) {
        let mut seen = Vec::with_capacity(PanelId::ALL.len());
        self.order.retain(|p| {
            let fresh = !seen.contains(p);
            seen.push(*p);
            fresh
        });
        for p in PanelId::ALL {
            if !self.order.contains(&p) {
                self.order.push(p);
            }
        }
        self.hidden.retain(|p| *p != PanelId::Chat);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Advanced {
    pub developer_mode: bool,
    /// Megabytes of decoded images kept in memory.
    pub image_cache_mb: u32,
    /// Megabytes of downloaded images kept on disk.
    pub disk_cache_mb: u32,
    /// Starts minimized to the tray.
    pub start_minimized: bool,
    pub close_to_tray: bool,
    /// Discord's client build number, sent when signing in.
    pub client_build_number: u64,
    /// Looks for a newer rcord at each start and offers to install it.
    pub check_for_updates: bool,
}

impl Default for Advanced {
    fn default() -> Self {
        Self {
            developer_mode: false,
            image_cache_mb: 96,
            disk_cache_mb: 1024,
            start_minimized: false,
            close_to_tray: false,
            client_build_number: 0,
            check_for_updates: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Language {
    pub locale: String,
}

impl Default for Language {
    fn default() -> Self {
        Self {
            locale: "en-US".into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not read or write {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("settings file is not valid TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("could not encode settings: {0}")]
    Encode(#[from] toml::ser::Error),
}

impl Settings {
    /// Clamps every number into its range and repairs the layout.
    pub fn sanitize(&mut self) {
        let a = &mut self.appearance;
        a.ui_scale = finite_or(a.ui_scale, 1.0).clamp(0.75, 1.5);
        a.chat_font_size = finite_or(a.chat_font_size, 16.0).clamp(12.0, 24.0);
        a.group_spacing = finite_or(a.group_spacing, 16.0).clamp(0.0, 24.0);
        a.corner_radius = finite_or(a.corner_radius, 8.0).clamp(0.0, 16.0);
        a.saturation = finite_or(a.saturation, 1.0).clamp(0.0, 1.0);
        a.accent &= 0xFF_FFFF;
        if let Some(theme) = &mut a.custom_theme {
            theme.sanitize();
        }
        if a.theme == ThemeId::Custom && a.custom_theme.is_none() {
            a.theme = ThemeId::Dark;
        }
        self.accessibility.tts_rate = finite_or(self.accessibility.tts_rate, 1.0).clamp(0.5, 2.0);
        self.chat.max_media_height =
            finite_or(self.chat.max_media_height, 350.0).clamp(100.0, 1000.0);
        self.chat.messages_per_channel = self.chat.messages_per_channel.clamp(50, 5_000);
        let v = &mut self.voice;
        v.input_volume = finite_or(v.input_volume, 1.0).clamp(0.0, 2.0);
        v.output_volume = finite_or(v.output_volume, 1.0).clamp(0.0, 2.0);
        v.sensitivity_db = finite_or(v.sensitivity_db, -50.0).clamp(-100.0, 0.0);
        v.attenuation = finite_or(v.attenuation, 0.0).clamp(0.0, 1.0);
        v.max_bitrate_kbps = v.max_bitrate_kbps.clamp(8, 512);
        v.push_to_talk_release_ms = v.push_to_talk_release_ms.min(2_000);
        for vol in v.user_volumes.values_mut() {
            *vol = finite_or(*vol, 1.0).clamp(0.0, 2.0);
        }
        // Full volume is the default, so it needs no entry.
        v.user_volumes.retain(|_, vol| (*vol - 1.0).abs() > 0.005);
        for vol in v.stream_volumes.values_mut() {
            *vol = finite_or(*vol, 1.0).clamp(0.0, 2.0);
        }
        v.stream_volumes.retain(|_, vol| (*vol - 1.0).abs() > 0.005);
        v.muted_users.sort();
        v.muted_users.dedup();
        self.stream.fps = match self.stream.fps {
            0..=15 => 15,
            16..=30 => 30,
            _ => 60,
        };
        self.advanced.image_cache_mb = self.advanced.image_cache_mb.clamp(32, 4_096);
        self.advanced.disk_cache_mb = self.advanced.disk_cache_mb.clamp(0, 32_768);
        self.layout.sanitize();
    }

    pub fn from_toml(text: &str) -> Result<Self, Error> {
        let mut s: Self = toml::from_str(text)?;
        if s.revision < 1 && s.advanced.image_cache_mb == 256 {
            // 0.1.3 lowered the image memory default; files still on the old default move.
            s.advanced.image_cache_mb = Advanced::default().image_cache_mb;
        }
        s.revision = s.revision.max(REVISION);
        s.sanitize();
        Ok(s)
    }

    pub fn to_toml(&self) -> Result<String, Error> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Loads `path`, or the defaults when it does not exist yet.
    pub fn load(path: &Path) -> Result<Self, Error> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_toml(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(Error::Io {
                path: path.to_owned(),
                source,
            }),
        }
    }

    /// Writes to a temporary file beside `path` and renames it into place, so a crash
    /// mid-write never leaves a truncated settings file.
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let io = |source| Error::Io {
            path: path.to_owned(),
            source,
        };
        let text = self.to_toml()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let tmp = path.with_extension("toml.tmp");
        let mut file = std::fs::File::create(&tmp).map_err(io)?;
        file.write_all(text.as_bytes()).map_err(io)?;
        file.sync_all().map_err(io)?;
        drop(file);
        std::fs::rename(&tmp, path).map_err(io)
    }
}

fn finite_or(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v } else { fallback }
}

/// Where rcord keeps its files: settings under the config directory, images under cache.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl Paths {
    pub fn detect() -> Option<Self> {
        let dirs = directories::ProjectDirs::from("dev", "khaodius", "rcord")?;
        Some(Self {
            config_dir: dirs.config_dir().to_owned(),
            cache_dir: dirs.cache_dir().to_owned(),
        })
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }

    /// Where theme files are kept, so they show up in Settings > Appearance.
    pub fn themes(&self) -> PathBuf {
        self.config_dir.join("themes")
    }

    pub fn image_cache(&self) -> PathBuf {
        self.cache_dir.join("images")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_volumes_count_local_mutes_and_drop_defaults() {
        let mut s = Settings::default();
        s.voice.user_volumes.insert("1".into(), 1.5);
        s.voice.user_volumes.insert("2".into(), 1.0);
        s.voice.user_volumes.insert("3".into(), 9.0);
        s.voice.muted_users = vec!["1".into(), "1".into()];
        s.sanitize();
        assert_eq!(s.voice.user_volume("1"), 0.0);
        assert!(!s.voice.user_volumes.contains_key("2"));
        assert_eq!(s.voice.user_volume("3"), 2.0);
        assert_eq!(s.voice.user_volume("4"), 1.0);
        assert_eq!(s.voice.muted_users, ["1"]);
        let back = Settings::from_toml(&s.to_toml().unwrap()).unwrap();
        assert_eq!(back.voice.user_volumes, s.voice.user_volumes);
    }

    #[test]
    fn old_image_default_moves_down() {
        let s = Settings::from_toml("[advanced]\nimage_cache_mb = 256\n").unwrap();
        assert_eq!(s.advanced.image_cache_mb, 96);
        let s = Settings::from_toml("revision = 1\n[advanced]\nimage_cache_mb = 256\n").unwrap();
        assert_eq!(s.advanced.image_cache_mb, 256);
    }

    #[test]
    fn round_trips() {
        let s = Settings::default();
        let text = s.to_toml().unwrap();
        assert_eq!(Settings::from_toml(&text).unwrap(), s);
    }

    #[test]
    fn partial_and_unknown_keys_load() {
        let s = Settings::from_toml(
            "[appearance]\ntheme = \"midnight\"\nui_scale = 9.0\nfuture_key = 1\n[brand_new]\nx = 1\n",
        )
        .unwrap();
        assert_eq!(s.appearance.theme, ThemeId::Midnight);
        assert_eq!(s.appearance.ui_scale, 1.5);
        assert_eq!(s.chat, Chat::default());
    }

    #[test]
    fn layout_repairs() {
        let s = Settings::from_toml(
            "[layout]\norder = [\"chat\", \"chat\", \"servers\"]\nhidden = [\"chat\", \"members\"]\n",
        )
        .unwrap();
        assert_eq!(
            s.layout.order,
            vec![
                PanelId::Chat,
                PanelId::Servers,
                PanelId::Channels,
                PanelId::Members
            ]
        );
        assert_eq!(s.layout.hidden, vec![PanelId::Members]);
    }

    #[test]
    fn move_panel() {
        let mut l = Layout::default();
        l.move_panel(PanelId::Members, 0);
        assert_eq!(
            l.order,
            vec![
                PanelId::Members,
                PanelId::Servers,
                PanelId::Channels,
                PanelId::Chat
            ]
        );
        assert_eq!(l.width(PanelId::Members), Some(248.0));
        l.widths.insert(PanelId::Members, 10_000.0);
        assert_eq!(l.width(PanelId::Members), Some(420.0));
    }

    #[test]
    fn saves_atomically() {
        let dir = std::env::temp_dir().join(format!("rcord-settings-{}", std::process::id()));
        let path = dir.join("settings.toml");
        let mut s = Settings::default();
        s.appearance.hour24 = true;
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), s);
        assert!(!path.with_extension("toml.tmp").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
