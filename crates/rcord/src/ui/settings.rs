//! The settings screen. Every control writes through [`AppState::update_settings`], which
//! applies the change at once and saves it shortly after.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, ClipboardItem, Context, ElementId, Entity, FocusHandle, Focusable,
    FontWeight, Hsla, IntoElement, MouseButton, Pixels, Render, ScrollHandle, SharedString,
    Subscription, Window, canvas, div, prelude::*, px,
};
use rcord_settings::{
    Animate, Encoder, InputMode, Keybinds, MessageDisplay, NoiseSuppression, PanelId, RoleColors,
    Settings, Spoilers, ThemeId,
};

use crate::state::{AppState, Overlay, SettingsPage, StateEvent};
use crate::theme::{Theme, theme};
use crate::ui::icons::{Icon, icon};
use crate::ui::profile::profile_card;
use crate::ui::text_input::{InputEvent, TextInput};
use crate::ui::{avatar, section_label, user_avatar_url};

type Field<T> = fn(&mut Settings) -> &mut T;

const ACCENTS: [u32; 10] = [
    0x5865F2, 0x3BA55C, 0xEB459E, 0xED4245, 0xFAA61A, 0x00A8FC, 0x9B59B6, 0x1ABC9C, 0xE67E22,
    0x99AAB5,
];

const LOCALES: [(&str, &str); 16] = [
    ("en-US", "English, US"),
    ("en-GB", "English, UK"),
    ("de", "Deutsch"),
    ("es-ES", "Español"),
    ("fr", "Français"),
    ("it", "Italiano"),
    ("nl", "Nederlands"),
    ("pl", "Polski"),
    ("pt-BR", "Português do Brasil"),
    ("sv-SE", "Svenska"),
    ("tr", "Türkçe"),
    ("ru", "Русский"),
    ("uk", "Українська"),
    ("ja", "日本語"),
    ("ko", "한국어"),
    ("zh-CN", "中文"),
];

const KEYBIND_LABELS: [(&str, &str); 17] = [
    ("quick-switcher", "Open the quick switcher"),
    ("settings", "Open settings"),
    ("search", "Search"),
    ("previous-channel", "Previous channel"),
    ("next-channel", "Next channel"),
    ("previous-unread", "Previous unread channel"),
    ("next-unread", "Next unread channel"),
    ("previous-server", "Previous server"),
    ("next-server", "Next server"),
    ("mark-read", "Mark channel read / close"),
    ("toggle-member-list", "Toggle member list"),
    ("toggle-mute", "Toggle mute"),
    ("toggle-deafen", "Toggle deafen"),
    ("zoom-in", "Zoom in"),
    ("zoom-out", "Zoom out"),
    ("zoom-reset", "Reset zoom"),
    ("reset-layout", "Reset panel layout"),
];

pub struct SettingsView {
    state: Entity<AppState>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    accent_input: Entity<TextInput>,
    ui_font_input: Entity<TextInput>,
    code_font_input: Entity<TextInput>,
    build_input: Entity<TextInput>,
    recording: Option<String>,
    revealed_email: bool,
    _subs: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let s = state.read(cx).settings.clone();
        let accent_input = cx.new(|cx| {
            let mut i = TextInput::new("#5865F2", cx);
            i.set_text(format!("#{:06X}", s.appearance.accent), cx);
            i
        });
        let ui_font_input = cx.new(|cx| {
            let mut i = TextInput::new("Inter", cx);
            i.set_text(s.appearance.ui_font.clone().unwrap_or_default(), cx);
            i
        });
        let code_font_input = cx.new(|cx| {
            let mut i = TextInput::new("JetBrains Mono", cx);
            i.set_text(s.appearance.code_font.clone().unwrap_or_default(), cx);
            i
        });
        let build_input = cx.new(|cx| {
            let mut i = TextInput::new("Automatic", cx);
            if s.advanced.client_build_number != 0 {
                i.set_text(s.advanced.client_build_number.to_string(), cx);
            }
            i
        });
        let mut subs = vec![cx.subscribe(&state, |_, _, e, cx| {
            if matches!(
                e,
                StateEvent::Settings | StateEvent::Overlay | StateEvent::Store(_)
            ) {
                cx.notify();
            }
        })];
        subs.push(
            cx.subscribe_in(&accent_input, window, |this, input, e, _, cx| {
                if matches!(e, InputEvent::Submit | InputEvent::Changed) {
                    let text = input
                        .read(cx)
                        .text()
                        .trim()
                        .trim_start_matches('#')
                        .to_owned();
                    if text.len() == 6
                        && let Ok(v) = u32::from_str_radix(&text, 16)
                    {
                        this.update(cx, |s| s.appearance.accent = v);
                    }
                }
            }),
        );
        subs.push(
            cx.subscribe_in(&ui_font_input, window, |this, input, e, _, cx| {
                if matches!(e, InputEvent::Submit) {
                    let v = input.read(cx).text().trim().to_owned();
                    this.update(cx, |s| s.appearance.ui_font = (!v.is_empty()).then_some(v));
                }
            }),
        );
        subs.push(
            cx.subscribe_in(&code_font_input, window, |this, input, e, _, cx| {
                if matches!(e, InputEvent::Submit) {
                    let v = input.read(cx).text().trim().to_owned();
                    this.update(cx, |s| {
                        s.appearance.code_font = (!v.is_empty()).then_some(v)
                    });
                }
            }),
        );
        subs.push(
            cx.subscribe_in(&build_input, window, |this, input, e, _, cx| {
                if matches!(e, InputEvent::Submit) {
                    let v = input.read(cx).text().trim().parse::<u64>().unwrap_or(0);
                    this.update(cx, |s| s.advanced.client_build_number = v);
                    this.toast("Takes effect the next time rcord starts", cx);
                }
            }),
        );
        Self {
            state,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            accent_input,
            ui_font_input,
            code_font_input,
            build_input,
            recording: None,
            revealed_email: false,
            _subs: subs,
        }
    }

    fn update(&self, cx: &mut App, f: impl FnOnce(&mut Settings)) {
        self.state.update(cx, |s, cx| s.update_settings(cx, f));
    }

    fn toast(&self, text: &str, cx: &mut App) {
        let text = text.to_owned();
        self.state.update(cx, |s, cx| s.toast(text, cx));
    }

    fn page(&self, cx: &App) -> SettingsPage {
        match self.state.read(cx).overlay {
            Some(Overlay::Settings(p)) => p,
            _ => SettingsPage::Account,
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.recording = None;
        self.state.update(cx, |s, cx| s.set_overlay(None, cx));
    }

    fn on_key(&mut self, e: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let Some(action) = self.recording.clone() else {
            if e.keystroke.key == "escape" {
                self.close(cx);
            }
            return;
        };
        let k = &e.keystroke;
        if matches!(
            k.key.as_str(),
            "shift" | "control" | "alt" | "platform" | "function"
        ) {
            return;
        }
        cx.stop_propagation();
        self.recording = None;
        if k.key == "escape" && !k.modifiers.modified() {
            cx.notify();
            return;
        }
        let chord = k.unparse();
        self.update(cx, |s| {
            s.keybinds.0.insert(action, chord);
        });
        let keys = self.state.read(cx).settings.keybinds.clone();
        crate::rebind_keys(&keys, cx);
        cx.notify();
    }

    // ---- pages ---------------------------------------------------------------------------

    fn account(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let s = self.state.read(cx);
        let streamer = s.settings.privacy.streamer_mode.enabled
            && s.settings.privacy.streamer_mode.hide_personal_info;
        let Some(me) = s.store.me.clone() else {
            return vec![note("Not signed in.", t)];
        };
        let email = me.email.clone().unwrap_or_else(|| "Not set".into());
        let shown_email = if self.revealed_email && !streamer {
            email
        } else {
            mask_email(&email)
        };
        let reveal = cx.listener(|this, _, _, cx| {
            this.revealed_email = !this.revealed_email;
            cx.notify();
        });
        let id = me.id;
        let state = self.state.clone();
        vec![
            div()
                .rounded(t.radius)
                .bg(t.sidebar)
                .overflow_hidden()
                .child(
                    div()
                        .h(px(100.))
                        .bg(me.accent_color.map_or(t.accent, |c| gpui::rgb(c).into())),
                )
                .child(
                    div()
                        .p(px(16.))
                        .flex()
                        .items_end()
                        .gap(px(16.))
                        .mt(px(-48.))
                        .child(div().p(px(6.)).rounded_full().bg(t.sidebar).child(avatar(
                            user_avatar_url(&me, 80., true),
                            px(80.),
                            None,
                        )))
                        .child(
                            div()
                                .pb(px(8.))
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_xl()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(t.header)
                                        .child(me.display_name().to_owned()),
                                )
                                .child(div().text_sm().text_color(t.text_muted).child(me.tag())),
                        ),
                )
                .child(
                    div()
                        .m(px(16.))
                        .mt(px(0.))
                        .p(px(16.))
                        .rounded(t.radius_small)
                        .bg(t.base)
                        .flex()
                        .flex_col()
                        .gap(px(16.))
                        .child(info_row("Username", me.username.clone(), t))
                        .child(
                            div()
                                .flex()
                                .items_end()
                                .justify_between()
                                .child(info_row("Email", shown_email, t))
                                .when(!streamer, |el| {
                                    el.child(
                                        link_button(
                                            "reveal-email",
                                            if self.revealed_email {
                                                "Hide"
                                            } else {
                                                "Reveal"
                                            },
                                            t,
                                        )
                                        .on_click(reveal),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_end()
                                .justify_between()
                                .child(info_row("User ID", id.to_string(), t))
                                .child(link_button("copy-id", "Copy", t).on_click(
                                    move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            id.to_string(),
                                        ))
                                    },
                                )),
                        ),
                )
                .into_any_element(),
            heading("Session", t),
            note(
                "Your sign-in token is kept in the system keychain, never in the settings file.",
                t,
            ),
            div()
                .flex()
                .child(
                    button("log-out", "Log Out", t.danger, gpui::white(), t)
                        .on_click(move |_, _, cx| state.update(cx, |s, cx| s.sign_out(cx))),
                )
                .into_any_element(),
        ]
    }

    fn profile(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let me = self.state.read(cx).store.me_id();
        let mut out = vec![note(
            "This is how others see your profile. Badges, decorations and server tags come from Discord.",
            t,
        )];
        if let Some(me) = me {
            out.push(
                div()
                    .child(profile_card(&self.state, me, None, cx))
                    .into_any_element(),
            );
        }
        out
    }

    fn privacy(&mut self, t: &Theme, s: &mut Settings, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![
            self.toggle(
                "Send typing indicators",
                "Others see when you are typing.",
                s,
                |s| &mut s.privacy.send_typing,
                t,
                cx,
            ),
            self.toggle(
                "Share activity status",
                "Show what you are playing or listening to.",
                s,
                |s| &mut s.privacy.show_activity,
                t,
                cx,
            ),
            self.toggle(
                "Blur explicit media",
                "Blurs images Discord flags as sensitive until you click them.",
                s,
                |s| &mut s.privacy.blur_explicit_media,
                t,
                cx,
            ),
        ]
    }

    fn appearance(
        &mut self,
        t: &Theme,
        s: &mut Settings,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let current = s.appearance.theme;
        let mut themes = div().flex().gap(px(12.));
        for id in ThemeId::ALL {
            let preview = Theme::from_settings(&rcord_settings::Appearance {
                theme: id,
                ..s.appearance.clone()
            });
            let selected = id == current;
            themes = themes.child(
                div()
                    .id(ElementId::Name(format!("theme-{}", id.label()).into()))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update(cx, |s| s.appearance.theme = id)
                    }))
                    .child(
                        div()
                            .size(px(64.))
                            .rounded_full()
                            .border_2()
                            .border_color(if selected { t.accent } else { t.border })
                            .overflow_hidden()
                            .flex()
                            .child(div().w(px(20.)).h_full().bg(preview.sidebar))
                            .child(
                                div()
                                    .flex_1()
                                    .h_full()
                                    .bg(preview.base)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(selected, |el| {
                                        el.child(
                                            icon(Icon::Check)
                                                .size(px(18.))
                                                .text_color(preview.accent),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if selected { t.header } else { t.text_muted })
                            .child(id.label()),
                    ),
            );
        }
        let accent = s.appearance.accent;
        let mut swatches = div().flex().items_center().gap(px(8.));
        for c in ACCENTS {
            let selected = c == accent;
            let input = self.accent_input.clone();
            swatches = swatches.child(
                div()
                    .id(ElementId::NamedInteger("accent".into(), c as u64))
                    .size(px(32.))
                    .rounded(t.radius_small)
                    .bg(gpui::rgb(c))
                    .cursor_pointer()
                    .when(selected, |el| el.border_2().border_color(t.header))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        input.update(cx, |i, cx| i.set_text(format!("#{c:06X}"), cx));
                        this.update(cx, |s| s.appearance.accent = c);
                    })),
            );
        }
        swatches = swatches.child(div().w(px(110.)).child(self.accent_input.clone()));

        vec![
            heading("Theme", t),
            themes.into_any_element(),
            heading("Accent color", t),
            swatches.into_any_element(),
            heading("Scaling", t),
            self.slider(
                "Interface zoom",
                s,
                |s| &mut s.appearance.ui_scale,
                0.75,
                1.5,
                0.05,
                |v| format!("{:.0}%", v * 100.),
                t,
                cx,
            ),
            self.slider(
                "Chat font size",
                s,
                |s| &mut s.appearance.chat_font_size,
                12.,
                24.,
                1.,
                |v| format!("{v:.0}px"),
                t,
                cx,
            ),
            self.slider(
                "Space between message groups",
                s,
                |s| &mut s.appearance.group_spacing,
                0.,
                24.,
                2.,
                |v| format!("{v:.0}px"),
                t,
                cx,
            ),
            self.slider(
                "Corner roundness",
                s,
                |s| &mut s.appearance.corner_radius,
                0.,
                16.,
                1.,
                |v| format!("{v:.0}px"),
                t,
                cx,
            ),
            self.slider(
                "Saturation",
                s,
                |s| &mut s.appearance.saturation,
                0.,
                1.,
                0.05,
                |v| format!("{:.0}%", v * 100.),
                t,
                cx,
            ),
            heading("Messages", t),
            self.choice(
                "Message display",
                s,
                |s| &mut s.appearance.message_display,
                &[
                    ("Cozy", MessageDisplay::Cozy),
                    ("Compact", MessageDisplay::Compact),
                ],
                t,
                cx,
            ),
            self.toggle(
                "Show avatars in compact mode",
                "",
                s,
                |s| &mut s.appearance.show_avatars_in_compact,
                t,
                cx,
            ),
            self.choice(
                "Role colors",
                s,
                |s| &mut s.appearance.role_colors,
                &[
                    ("In names", RoleColors::InNames),
                    ("Beside names", RoleColors::BesideNames),
                    ("Hidden", RoleColors::Hidden),
                ],
                t,
                cx,
            ),
            self.toggle("24-hour time", "", s, |s| &mut s.appearance.hour24, t, cx),
            heading("Motion", t),
            self.choice(
                "Animate emoji",
                s,
                |s| &mut s.appearance.animate_emoji,
                ANIMATE,
                t,
                cx,
            ),
            self.choice(
                "Animate avatars",
                s,
                |s| &mut s.appearance.animate_avatars,
                ANIMATE,
                t,
                cx,
            ),
            heading("Panels", t),
            self.toggle(
                "Show server names",
                "Shows names under icons in the server list.",
                s,
                |s| &mut s.appearance.server_names,
                t,
                cx,
            ),
            self.toggle(
                "Panel separators",
                "Draws a thin line between panels.",
                s,
                |s| &mut s.appearance.panel_borders,
                t,
                cx,
            ),
            heading("Fonts", t),
            labeled(
                "Interface font",
                "Any installed font. Press Enter to apply; leave empty for Inter.",
                self.ui_font_input.clone().into_any_element(),
                t,
            ),
            labeled(
                "Code font",
                "Used in code blocks. Leave empty for JetBrains Mono.",
                self.code_font_input.clone().into_any_element(),
                t,
            ),
        ]
    }

    fn accessibility(
        &mut self,
        t: &Theme,
        s: &mut Settings,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        vec![
            self.toggle(
                "Reduced motion",
                "Turns off animations and hover effects that move.",
                s,
                |s| &mut s.accessibility.reduced_motion,
                t,
                cx,
            ),
            self.toggle(
                "Underline links",
                "",
                s,
                |s| &mut s.accessibility.underline_links,
                t,
                cx,
            ),
            self.toggle(
                "Role color contrast",
                "Lightens or darkens role colors that are hard to read on your theme.",
                s,
                |s| &mut s.accessibility.role_color_contrast,
                t,
                cx,
            ),
            self.toggle(
                "Always show focus rings",
                "",
                s,
                |s| &mut s.accessibility.always_show_focus,
                t,
                cx,
            ),
            self.slider(
                "Text-to-speech rate",
                s,
                |s| &mut s.accessibility.tts_rate,
                0.5,
                2.,
                0.1,
                |v| format!("{v:.1}×"),
                t,
                cx,
            ),
        ]
    }

    fn voice(&mut self, t: &Theme, s: &mut Settings, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let ptt = s.voice.input_mode == InputMode::PushToTalk;
        let auto = s.voice.auto_sensitivity;
        let mut out = vec![
            banner(
                "Voice and screen share are under construction. These settings are saved now and apply once calls ship.",
                t,
            ),
            heading("Devices", t),
            labeled(
                "Input device",
                "",
                value_chip(
                    s.voice.input_device.as_deref().unwrap_or("System default"),
                    t,
                ),
                t,
            ),
            labeled(
                "Output device",
                "",
                value_chip(
                    s.voice.output_device.as_deref().unwrap_or("System default"),
                    t,
                ),
                t,
            ),
            self.slider(
                "Input volume",
                s,
                |s| &mut s.voice.input_volume,
                0.,
                2.,
                0.05,
                |v| format!("{:.0}%", v * 100.),
                t,
                cx,
            ),
            self.slider(
                "Output volume",
                s,
                |s| &mut s.voice.output_volume,
                0.,
                2.,
                0.05,
                |v| format!("{:.0}%", v * 100.),
                t,
                cx,
            ),
            heading("Input mode", t),
            self.choice(
                "Mode",
                s,
                |s| &mut s.voice.input_mode,
                &[
                    ("Voice activity", InputMode::VoiceActivity),
                    ("Push to talk", InputMode::PushToTalk),
                ],
                t,
                cx,
            ),
        ];
        if ptt {
            out.push(self.slider_u32(
                "Release delay",
                s,
                |s| &mut s.voice.push_to_talk_release_ms,
                0,
                2000,
                10,
                |v| format!("{v} ms"),
                t,
                cx,
            ));
        } else {
            out.push(self.toggle(
                "Automatic sensitivity",
                "",
                s,
                |s| &mut s.voice.auto_sensitivity,
                t,
                cx,
            ));
            if !auto {
                out.push(self.slider(
                    "Sensitivity",
                    s,
                    |s| &mut s.voice.sensitivity_db,
                    -100.,
                    0.,
                    1.,
                    |v| format!("{v:.0} dB"),
                    t,
                    cx,
                ));
            }
        }
        out.extend([
            heading("Processing", t),
            self.choice(
                "Noise suppression",
                s,
                |s| &mut s.voice.noise_suppression,
                &[
                    ("Off", NoiseSuppression::Off),
                    ("Standard", NoiseSuppression::Standard),
                    ("Strong", NoiseSuppression::Strong),
                ],
                t,
                cx,
            ),
            self.toggle(
                "Echo cancellation",
                "",
                s,
                |s| &mut s.voice.echo_cancellation,
                t,
                cx,
            ),
            self.toggle(
                "Automatic gain control",
                "",
                s,
                |s| &mut s.voice.automatic_gain,
                t,
                cx,
            ),
            self.slider(
                "Attenuation",
                s,
                |s| &mut s.voice.attenuation,
                0.,
                1.,
                0.05,
                |v| format!("{:.0}%", v * 100.),
                t,
                cx,
            ),
            self.slider_u32(
                "Maximum bitrate",
                s,
                |s| &mut s.voice.max_bitrate_kbps,
                8,
                512,
                8,
                |v| format!("{v} kbps"),
                t,
                cx,
            ),
            heading("Screen share", t),
            self.choice(
                "Resolution",
                s,
                |s| &mut s.stream.resolution,
                &[
                    ("720p", 720),
                    ("1080p", 1080),
                    ("1440p", 1440),
                    ("Source", 0),
                ],
                t,
                cx,
            ),
            self.choice(
                "Frame rate",
                s,
                |s| &mut s.stream.fps,
                &[("15", 15), ("30", 30), ("60", 60)],
                t,
                cx,
            ),
            self.choice(
                "Encoder",
                s,
                |s| &mut s.stream.encoder,
                &[
                    ("Automatic", Encoder::Auto),
                    ("Hardware", Encoder::Hardware),
                    ("Software", Encoder::Software),
                ],
                t,
                cx,
            ),
            self.toggle("Share audio", "", s, |s| &mut s.stream.include_audio, t, cx),
            self.toggle(
                "Show cursor",
                "",
                s,
                |s| &mut s.stream.capture_cursor,
                t,
                cx,
            ),
        ]);
        out
    }

    fn chat(&mut self, t: &Theme, s: &mut Settings, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![
            heading("Media", t),
            self.toggle(
                "Show images and videos inline",
                "",
                s,
                |s| &mut s.chat.inline_attachments,
                t,
                cx,
            ),
            self.toggle(
                "Show media from links",
                "",
                s,
                |s| &mut s.chat.inline_links,
                t,
                cx,
            ),
            self.toggle(
                "Show link previews",
                "",
                s,
                |s| &mut s.chat.link_previews,
                t,
                cx,
            ),
            self.toggle("Autoplay GIFs", "", s, |s| &mut s.chat.gif_autoplay, t, cx),
            self.toggle(
                "Animate stickers",
                "",
                s,
                |s| &mut s.chat.sticker_autoplay,
                t,
                cx,
            ),
            self.slider(
                "Tallest inline media",
                s,
                |s| &mut s.chat.max_media_height,
                100.,
                1000.,
                25.,
                |v| format!("{v:.0}px"),
                t,
                cx,
            ),
            heading("Messages", t),
            self.toggle(
                "Show reactions",
                "",
                s,
                |s| &mut s.chat.show_reactions,
                t,
                cx,
            ),
            self.toggle(
                "Large emoji when alone",
                "",
                s,
                |s| &mut s.chat.jumbo_emoji,
                t,
                cx,
            ),
            self.toggle(
                "Convert emoticons",
                "Turns :) into 🙂 as you type.",
                s,
                |s| &mut s.chat.convert_emoticons,
                t,
                cx,
            ),
            self.choice(
                "Show spoilers",
                s,
                |s| &mut s.chat.spoilers,
                &[
                    ("On click", Spoilers::OnClick),
                    ("Servers I moderate", Spoilers::InServersIModerate),
                    ("Always", Spoilers::Always),
                ],
                t,
                cx,
            ),
            heading("Composer", t),
            self.toggle(
                "Enter sends",
                "When off, Ctrl+Enter sends.",
                s,
                |s| &mut s.chat.send_on_enter,
                t,
                cx,
            ),
            self.toggle(
                "Mention people you reply to",
                "",
                s,
                |s| &mut s.chat.reply_ping_default,
                t,
                cx,
            ),
            heading("Memory", t),
            self.slider_u32(
                "Messages kept per channel",
                s,
                |s| &mut s.chat.messages_per_channel,
                50,
                5000,
                50,
                |v| v.to_string(),
                t,
                cx,
            ),
        ]
    }

    fn notifications(
        &mut self,
        t: &Theme,
        s: &mut Settings,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        vec![
            self.toggle(
                "Desktop notifications",
                "",
                s,
                |s| &mut s.notifications.desktop,
                t,
                cx,
            ),
            self.toggle(
                "Notification sounds",
                "",
                s,
                |s| &mut s.notifications.sounds,
                t,
                cx,
            ),
            self.toggle(
                "Unread badge",
                "Shows a count on the taskbar icon.",
                s,
                |s| &mut s.notifications.unread_badge,
                t,
                cx,
            ),
            self.toggle(
                "Flash the taskbar",
                "",
                s,
                |s| &mut s.notifications.flash_taskbar,
                t,
                cx,
            ),
            self.toggle(
                "Quiet while focused",
                "No popups for the channel you are looking at.",
                s,
                |s| &mut s.notifications.suppress_while_focused,
                t,
                cx,
            ),
            self.toggle(
                "Message previews",
                "Shows message text in notifications.",
                s,
                |s| &mut s.notifications.message_previews,
                t,
                cx,
            ),
        ]
    }

    fn keybinds(&mut self, t: &Theme, s: &mut Settings, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![note(
            "Click a shortcut, then press the new keys. Escape cancels.",
            t,
        )];
        for (action, label) in KEYBIND_LABELS {
            let current = s.keybinds.0.get(action).cloned().unwrap_or_default();
            let recording = self.recording.as_deref() == Some(action);
            out.push(
                row(label, "", t)
                    .child(
                        div()
                            .id(ElementId::Name(format!("kb-{action}").into()))
                            .min_w(px(140.))
                            .h(px(32.))
                            .px(px(10.))
                            .rounded(t.radius_small)
                            .bg(t.input)
                            .border_1()
                            .border_color(if recording { t.accent } else { t.border })
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .font_family("JetBrains Mono")
                            .text_sm()
                            .text_color(if recording { t.accent } else { t.header })
                            .child(if recording {
                                "Press keys…".to_owned()
                            } else {
                                pretty_chord(&current)
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.recording = Some(action.to_owned());
                                window.focus(&this.focus);
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            );
        }
        out.push(
            div()
                .flex()
                .child(
                    button("kb-reset", "Restore defaults", t.hover, t.header, t).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.update(cx, |s| s.keybinds = Keybinds::default());
                            let keys = this.state.read(cx).settings.keybinds.clone();
                            crate::rebind_keys(&keys, cx);
                        }),
                    ),
                )
                .into_any_element(),
        );
        out
    }

    fn language(&mut self, t: &Theme, s: &mut Settings, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let current = s.language.locale.clone();
        let mut out = vec![note(
            "Sent to Discord with every request and used for dates. Applies on next start.",
            t,
        )];
        for (code, name) in LOCALES {
            let selected = current == code;
            out.push(
                div()
                    .id(ElementId::Name(format!("locale-{code}").into()))
                    .h(px(44.))
                    .px(px(12.))
                    .rounded(t.radius_small)
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .cursor_pointer()
                    .bg(if selected { t.selected } else { t.sidebar })
                    .hover(|st| st.bg(t.hover))
                    .child(radio(selected, t))
                    .child(div().flex_1().text_color(t.header).child(name))
                    .child(div().text_sm().text_color(t.text_faint).child(code))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update(cx, |s| s.language.locale = code.to_owned())
                    }))
                    .into_any_element(),
            );
        }
        out
    }

    fn streamer(&mut self, t: &Theme, s: &mut Settings, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![
            self.toggle(
                "Streamer mode",
                "Hides personal details while you stream.",
                s,
                |s| &mut s.privacy.streamer_mode.enabled,
                t,
                cx,
            ),
            self.toggle(
                "Hide personal information",
                "Email, connections and notes.",
                s,
                |s| &mut s.privacy.streamer_mode.hide_personal_info,
                t,
                cx,
            ),
            self.toggle(
                "Hide invite links",
                "",
                s,
                |s| &mut s.privacy.streamer_mode.hide_invite_links,
                t,
                cx,
            ),
            self.toggle(
                "Disable sounds",
                "",
                s,
                |s| &mut s.privacy.streamer_mode.disable_sounds,
                t,
                cx,
            ),
            self.toggle(
                "Disable notifications",
                "",
                s,
                |s| &mut s.privacy.streamer_mode.disable_notifications,
                t,
                cx,
            ),
        ]
    }

    fn layout(&mut self, t: &Theme, s: &mut Settings, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![note(
            "Hold the right mouse button on a panel, move it, and release to place it. Drag the edge of a side panel to resize it; double-click the edge to restore its width.",
            t,
        )];
        let order = s.layout.order.clone();
        let last = order.len().saturating_sub(1);
        for (ix, panel) in order.into_iter().enumerate() {
            let visible = s.layout.is_visible(panel);
            let width = s.layout.width(panel);
            let mut r = row(panel.label(), "", t).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when_some(width, |el, w| {
                        el.child(
                            div()
                                .text_sm()
                                .text_color(t.text_faint)
                                .mr(px(8.))
                                .child(format!("{w:.0}px")),
                        )
                    })
                    .child(
                        small_icon_button(
                            ElementId::NamedInteger("up".into(), ix as u64),
                            Icon::ChevronUp,
                            ix > 0,
                            t,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.update(cx, |s| s.layout.move_panel(panel, ix.saturating_sub(1)))
                        })),
                    )
                    .child(
                        small_icon_button(
                            ElementId::NamedInteger("down".into(), ix as u64),
                            Icon::ChevronDown,
                            ix < last,
                            t,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.update(cx, |s| s.layout.move_panel(panel, ix + 1))
                        })),
                    ),
            );
            if panel != PanelId::Chat {
                r = r.child(
                    switch(ElementId::NamedInteger("vis".into(), ix as u64), visible, t).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.update(cx, |s| {
                                if visible {
                                    s.layout.hidden.push(panel);
                                } else {
                                    s.layout.hidden.retain(|p| *p != panel);
                                }
                            })
                        }),
                    ),
                );
            }
            out.push(r.into_any_element());
        }
        out.push(
            div()
                .flex()
                .child(
                    button("layout-reset", "Reset layout", t.hover, t.header, t).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.update(cx, |s| s.layout = Default::default())
                        }),
                    ),
                )
                .into_any_element(),
        );
        out
    }

    fn advanced(&mut self, t: &Theme, s: &mut Settings, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let paths = self.state.read(cx).paths.clone();
        let mut out = vec![
            self.toggle(
                "Developer mode",
                "Adds Copy ID to context menus.",
                s,
                |s| &mut s.advanced.developer_mode,
                t,
                cx,
            ),
            self.toggle(
                "Start minimized",
                "",
                s,
                |s| &mut s.advanced.start_minimized,
                t,
                cx,
            ),
            self.toggle(
                "Close to tray",
                "",
                s,
                |s| &mut s.advanced.close_to_tray,
                t,
                cx,
            ),
            heading("Updates", t),
            self.toggle(
                "Check for updates when rcord starts",
                concat!(
                    "Asks before installing. Updates download, install without any installer screens and restart rcord. This is version ",
                    env!("CARGO_PKG_VERSION"),
                    "."
                ),
                s,
                |s| &mut s.advanced.check_for_updates,
                t,
                cx,
            ),
            heading("Caches", t),
            self.slider_u32(
                "Image memory",
                s,
                |s| &mut s.advanced.image_cache_mb,
                32,
                4096,
                32,
                |v| format!("{v} MB"),
                t,
                cx,
            ),
            self.slider_u32(
                "Image disk cache",
                s,
                |s| &mut s.advanced.disk_cache_mb,
                0,
                32768,
                256,
                |v| {
                    if v == 0 {
                        "Off".into()
                    } else {
                        format!("{v} MB")
                    }
                },
                t,
                cx,
            ),
            heading("Client", t),
            labeled(
                "Client build number",
                "Reported to Discord. Leave empty to use the number Discord's web app currently ships, read at each start.",
                self.build_input.clone().into_any_element(),
                t,
            ),
        ];
        if let Some(p) = paths {
            let dir = p.config_dir.clone();
            out.push(
                div()
                    .flex()
                    .child(
                        button("open-config", "Open settings folder", t.hover, t.header, t)
                            .on_click(move |_, _, cx| cx.open_with_system(&dir)),
                    )
                    .into_any_element(),
            );
        }
        out.push(note(
            &format!(
                "rcord {} · {} · {}",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
            t,
        ));
        out
    }

    // ---- controls --------------------------------------------------------------------------

    fn toggle(
        &self,
        title: &'static str,
        desc: &'static str,
        s: &mut Settings,
        field: Field<bool>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on = *field(s);
        row(title, desc, t)
            .child(
                switch(ElementId::Name(title.into()), on, t).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.update(cx, |s| *field(s) = !on);
                    },
                )),
            )
            .into_any_element()
    }

    fn choice<T: Copy + PartialEq + 'static>(
        &self,
        title: &'static str,
        s: &mut Settings,
        field: Field<T>,
        options: &[(&'static str, T)],
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = *field(s);
        let mut seg = div()
            .flex()
            .p(px(3.))
            .gap(px(2.))
            .rounded(t.radius_small)
            .bg(t.base);
        for (ix, (label, value)) in options.iter().copied().enumerate() {
            let selected = value == current;
            seg = seg.child(
                div()
                    .id(ElementId::NamedInteger(
                        SharedString::from(title),
                        ix as u64,
                    ))
                    .px(px(12.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .rounded(t.radius_small)
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .when(selected, |el| el.bg(t.selected).text_color(t.header))
                    .when(!selected, |el| {
                        el.text_color(t.text_muted)
                            .hover(|st| st.text_color(t.text))
                    })
                    .child(label)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.update(cx, |s| *field(s) = value)),
                    ),
            );
        }
        row(title, "", t).child(seg).into_any_element()
    }

    fn slider(
        &self,
        title: &'static str,
        s: &mut Settings,
        field: Field<f32>,
        min: f32,
        max: f32,
        step: f32,
        fmt: fn(f32) -> String,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let value = *field(s);
        let set: Rc<dyn Fn(&mut Settings, f32)> = Rc::new(move |s, v| *field(s) = v);
        self.slider_inner(title, value, min, max, step, fmt(value), set, t, cx)
    }

    fn slider_u32(
        &self,
        title: &'static str,
        s: &mut Settings,
        field: Field<u32>,
        min: u32,
        max: u32,
        step: u32,
        fmt: fn(u32) -> String,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let value = *field(s);
        let set: Rc<dyn Fn(&mut Settings, f32)> = Rc::new(move |s, v| *field(s) = v.round() as u32);
        self.slider_inner(
            title,
            value as f32,
            min as f32,
            max as f32,
            step as f32,
            fmt(value),
            set,
            t,
            cx,
        )
    }

    fn slider_inner(
        &self,
        title: &'static str,
        value: f32,
        min: f32,
        max: f32,
        step: f32,
        label: String,
        set: Rc<dyn Fn(&mut Settings, f32)>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fraction = ((value - min) / (max - min)).clamp(0., 1.);
        let bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::default();
        let pick = {
            let bounds = bounds.clone();
            move |x: Pixels| -> Option<f32> {
                let b = bounds.get()?;
                let f = (f32::from(x - b.origin.x) / f32::from(b.size.width)).clamp(0., 1.);
                let raw = min + f * (max - min);
                Some(((raw - min) / step).round() * step + min)
            }
        };
        let (down_set, move_set, dec_set, inc_set) = (set.clone(), set.clone(), set.clone(), set);
        let pick_down = pick.clone();
        let track = div()
            .id(ElementId::Name(format!("slider-{title}").into()))
            .w(px(240.))
            .h(px(24.))
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
                    .ml(px(-8.))
                    .size(px(16.))
                    .rounded_full()
                    .bg(gpui::white())
                    .shadow_sm(),
            )
            .child(
                canvas(
                    move |b, _, _| {
                        bounds.set(Some(b));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                    if let Some(v) = pick_down(e.position.x) {
                        let set = down_set.clone();
                        this.update(cx, |s| set(s, v));
                    }
                }),
            )
            .on_mouse_move(cx.listener(move |this, e: &gpui::MouseMoveEvent, _, cx| {
                if e.pressed_button == Some(MouseButton::Left)
                    && let Some(v) = pick(e.position.x)
                    && (v - value).abs() >= step * 0.5
                {
                    let set = move_set.clone();
                    this.update(cx, |s| set(s, v));
                }
            }));
        row(title, "", t)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        step_button(ElementId::Name(format!("dec-{title}").into()), "−", t)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let set = dec_set.clone();
                                this.update(cx, |s| set(s, (value - step).max(min)));
                            })),
                    )
                    .child(track)
                    .child(
                        step_button(ElementId::Name(format!("inc-{title}").into()), "+", t)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let set = inc_set.clone();
                                this.update(cx, |s| set(s, (value + step).min(max)));
                            })),
                    )
                    .child(
                        div()
                            .w(px(64.))
                            .text_right()
                            .text_sm()
                            .text_color(t.text)
                            .child(label),
                    ),
            )
            .into_any_element()
    }
}

const ANIMATE: &[(&str, Animate)] = &[
    ("Always", Animate::Always),
    ("On hover", Animate::OnHover),
    ("Never", Animate::Never),
];

fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((_, domain)) => format!("**********@{domain}"),
        None => "**********".into(),
    }
}

/// `ctrl-shift-k` as `Ctrl + Shift + K`.
fn pretty_chord(chord: &str) -> String {
    if chord.is_empty() {
        return "Not set".into();
    }
    chord
        .split(' ')
        .map(|stroke| {
            let mut parts: Vec<String> = Vec::new();
            let mut rest = stroke;
            for (prefix, name) in [
                ("ctrl-", "Ctrl"),
                ("alt-", "Alt"),
                ("shift-", "Shift"),
                ("cmd-", "Super"),
                ("super-", "Super"),
                ("fn-", "Fn"),
            ] {
                while let Some(r) = rest.strip_prefix(prefix) {
                    parts.push(name.into());
                    rest = r;
                }
            }
            let key = match rest {
                "" => "-".to_owned(),
                k if k.chars().count() == 1 => k.to_uppercase(),
                k => {
                    let mut c = k.chars();
                    c.next()
                        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                        .unwrap_or_default()
                }
            };
            parts.push(key);
            parts.join(" + ")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn heading(text: &str, t: &Theme) -> AnyElement {
    section_label(text.to_owned(), t)
        .mt(px(16.))
        .into_any_element()
}

fn note(text: &str, t: &Theme) -> AnyElement {
    div()
        .text_sm()
        .text_color(t.text_muted)
        .child(text.to_owned())
        .into_any_element()
}

fn banner(text: &str, t: &Theme) -> AnyElement {
    div()
        .p(px(12.))
        .rounded(t.radius_small)
        .border_1()
        .border_color(t.accent)
        .bg(Hsla { a: 0.1, ..t.accent })
        .text_sm()
        .text_color(t.text)
        .child(text.to_owned())
        .into_any_element()
}

fn row(title: &str, desc: &str, t: &Theme) -> gpui::Div {
    div()
        .min_h(px(48.))
        .py(px(8.))
        .flex()
        .items_center()
        .gap(px(16.))
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.header)
                        .child(title.to_owned()),
                )
                .when(!desc.is_empty(), |el| {
                    el.child(
                        div()
                            .text_sm()
                            .text_color(t.text_muted)
                            .child(desc.to_owned()),
                    )
                }),
        )
}

fn labeled(title: &str, desc: &str, control: AnyElement, t: &Theme) -> AnyElement {
    row(title, desc, t)
        .child(div().w(px(240.)).child(control))
        .into_any_element()
}

fn value_chip(text: &str, t: &Theme) -> AnyElement {
    div()
        .h(px(36.))
        .px(px(12.))
        .rounded(t.radius_small)
        .bg(t.input)
        .flex()
        .items_center()
        .text_sm()
        .text_color(t.text)
        .child(text.to_owned())
        .into_any_element()
}

fn info_row(label: &str, value: String, t: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(section_label(label.to_owned(), t))
        .child(div().text_color(t.header).child(value))
}

fn switch(id: ElementId, on: bool, t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .w(px(40.))
        .h(px(24.))
        .p(px(3.))
        .rounded_full()
        .cursor_pointer()
        .bg(if on { t.accent } else { t.text_faint })
        .flex()
        .when(on, |el| el.justify_end())
        .child(
            div()
                .size(px(18.))
                .rounded_full()
                .bg(gpui::white())
                .shadow_sm(),
        )
}

fn radio(on: bool, t: &Theme) -> gpui::Div {
    div()
        .size(px(18.))
        .rounded_full()
        .border_2()
        .border_color(if on { t.accent } else { t.text_faint })
        .flex()
        .items_center()
        .justify_center()
        .when(on, |el| {
            el.child(div().size(px(8.)).rounded_full().bg(t.accent))
        })
}

fn button(
    id: &'static str,
    label: &'static str,
    bg: Hsla,
    fg: Hsla,
    t: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(36.))
        .px(px(16.))
        .rounded(t.radius_small)
        .bg(bg)
        .flex()
        .items_center()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .cursor_pointer()
        .hover(|s| s.opacity(0.9))
        .child(label)
}

fn link_button(id: &'static str, label: &'static str, t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(32.))
        .px(px(14.))
        .rounded(t.radius_small)
        .bg(t.hover)
        .flex()
        .items_center()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.header)
        .cursor_pointer()
        .child(label)
}

fn step_button(id: ElementId, label: &'static str, t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(24.))
        .rounded(t.radius_small)
        .flex()
        .items_center()
        .justify_center()
        .text_color(t.text_muted)
        .cursor_pointer()
        .hover(|s| s.bg(t.hover).text_color(t.header))
        .child(label)
}

fn small_icon_button(
    id: ElementId,
    which: Icon,
    enabled: bool,
    t: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let glyph = icon(which).size(px(16.));
    div()
        .id(id)
        .size(px(28.))
        .rounded(t.radius_small)
        .flex()
        .items_center()
        .justify_center()
        .text_color(if enabled { t.text_muted } else { t.text_faint })
        .when(!enabled, |el| el.opacity(0.4))
        .when(enabled, |el| {
            el.cursor_pointer()
                .hover(|s| s.bg(t.hover).text_color(t.header))
        })
        .child(glyph)
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

const NAV: &[(Option<&str>, SettingsPage, &str)] = &[
    (Some("User Settings"), SettingsPage::Account, "My Account"),
    (None, SettingsPage::Profile, "Profiles"),
    (None, SettingsPage::Privacy, "Privacy & Safety"),
    (Some("App Settings"), SettingsPage::Appearance, "Appearance"),
    (None, SettingsPage::Accessibility, "Accessibility"),
    (None, SettingsPage::Voice, "Voice & Video"),
    (None, SettingsPage::Chat, "Chat"),
    (None, SettingsPage::Notifications, "Notifications"),
    (None, SettingsPage::Keybinds, "Keybinds"),
    (None, SettingsPage::Language, "Language"),
    (None, SettingsPage::Streamer, "Streamer Mode"),
    (Some("rcord"), SettingsPage::Layout, "Layout"),
    (None, SettingsPage::Advanced, "Advanced"),
];

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let page = self.page(cx);
        let mut snapshot = self.state.read(cx).settings.clone();
        let title = NAV.iter().find(|(_, p, _)| *p == page).map_or("", |n| n.2);
        let body = match page {
            SettingsPage::Account => self.account(&t, cx),
            SettingsPage::Profile => self.profile(&t, cx),
            SettingsPage::Privacy => self.privacy(&t, &mut snapshot, cx),
            SettingsPage::Appearance => self.appearance(&t, &mut snapshot, cx),
            SettingsPage::Accessibility => self.accessibility(&t, &mut snapshot, cx),
            SettingsPage::Voice => self.voice(&t, &mut snapshot, cx),
            SettingsPage::Chat => self.chat(&t, &mut snapshot, cx),
            SettingsPage::Notifications => self.notifications(&t, &mut snapshot, cx),
            SettingsPage::Keybinds => self.keybinds(&t, &mut snapshot, cx),
            SettingsPage::Language => self.language(&t, &mut snapshot, cx),
            SettingsPage::Streamer => self.streamer(&t, &mut snapshot, cx),
            SettingsPage::Layout => self.layout(&t, &mut snapshot, cx),
            SettingsPage::Advanced => self.advanced(&t, &mut snapshot, cx),
        };

        let mut nav = div()
            .w(px(218.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .py(px(60.))
            .px(px(6.));
        for (ix, (section, p, label)) in NAV.iter().copied().enumerate() {
            if let Some(section) = section {
                nav = nav.child(
                    section_label(section.to_owned(), &t)
                        .px(px(10.))
                        .pt(px(if ix == 0 { 0. } else { 16. }))
                        .pb(px(6.)),
                );
            }
            let selected = p == page;
            nav = nav.child(
                div()
                    .id(ElementId::NamedInteger("nav".into(), ix as u64))
                    .h(px(32.))
                    .px(px(10.))
                    .rounded(t.radius_small)
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .font_weight(FontWeight::MEDIUM)
                    .when(selected, |el| el.bg(t.selected).text_color(t.header))
                    .when(!selected, |el| {
                        el.text_color(t.text_muted)
                            .hover(|s| s.bg(t.hover).text_color(t.text))
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.recording = None;
                        this.state
                            .update(cx, |s, cx| s.set_overlay(Some(Overlay::Settings(p)), cx));
                    })),
            );
        }
        let state = self.state.clone();
        nav = nav
            .child(div().h(px(1.)).mx(px(10.)).my(px(8.)).bg(t.border))
            .child(
                div()
                    .id("nav-logout")
                    .h(px(32.))
                    .px(px(10.))
                    .rounded(t.radius_small)
                    .flex()
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.danger)
                    .hover(|s| s.bg(t.hover))
                    .child("Log Out")
                    .child(icon(Icon::Logout).size(px(16.)))
                    .on_click(move |_, _, cx| state.update(cx, |s, cx| s.sign_out(cx))),
            );

        div()
            .id("settings")
            .key_context("Settings")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, _, cx| this.on_key(e, cx)))
            .occlude()
            .size_full()
            .flex()
            .bg(t.base)
            .text_color(t.text)
            .font_family("Inter")
            .child(
                div()
                    .id("settings-nav")
                    .flex_1()
                    .min_w(px(230.))
                    .h_full()
                    .bg(t.sidebar)
                    .flex()
                    .justify_end()
                    .overflow_y_scroll()
                    .child(nav),
            )
            .child(
                div()
                    .flex_basis(px(800.))
                    .flex_shrink()
                    .flex_grow()
                    .h_full()
                    .flex()
                    .child(
                        div()
                            .id("settings-body")
                            .flex_1()
                            .h_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .child(
                                div()
                                    .max_w(px(740.))
                                    .px(px(40.))
                                    .pt(px(60.))
                                    .pb(px(80.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .text_xl()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(t.header)
                                            .mb(px(12.))
                                            .child(title),
                                    )
                                    .children(body.into_iter().map(|e| div().flex_none().child(e))),
                            ),
                    )
                    .child(
                        div()
                            .pt(px(60.))
                            .pr(px(24.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .id("settings-close")
                                    .size(px(36.))
                                    .rounded_full()
                                    .border_2()
                                    .border_color(t.text_muted)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .text_color(t.text_muted)
                                    .hover(|s| s.border_color(t.header).text_color(t.header))
                                    .child(icon(Icon::Close).size(px(18.)))
                                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(t.text_faint)
                                    .child("ESC"),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_read_naturally() {
        assert_eq!(pretty_chord("ctrl-k"), "Ctrl + K");
        assert_eq!(pretty_chord("ctrl-alt-shift-l"), "Ctrl + Alt + Shift + L");
        assert_eq!(pretty_chord("alt-up"), "Alt + Up");
        assert_eq!(pretty_chord("ctrl--"), "Ctrl + -");
        assert_eq!(pretty_chord(""), "Not set");
    }

    #[test]
    fn email_is_masked() {
        assert_eq!(mask_email("a@b.com"), "**********@b.com");
    }
}
