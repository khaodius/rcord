//! Settings > Voice, laid out the way Discord's is: devices and volumes first, a mic test
//! with a live level meter, then an input profile. Voice Isolation and Studio are presets;
//! Custom opens every control. Rarely changed settings sit behind "Show Advanced Voice
//! Settings".

use gpui::{AnyElement, Context, ElementId, FontWeight, Hsla, Task, div, prelude::*, px};
use rcord_settings::{InputMode, InputProfile, NoiseSuppression, Settings};
use rcord_voice::{AudioEvent, AudioSettings, DeviceList, MicTest};

use super::{SettingsView, button, heading, note, radio, row};
use crate::theme::Theme;
use crate::ui::icons::{Icon, icon};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Picker {
    Input,
    Output,
}

/// State of the voice page that is not a setting.
#[derive(Default)]
pub struct VoicePage {
    devices: Option<DeviceList>,
    picker: Option<Picker>,
    test: Option<MicTest>,
    test_events: Option<Task<()>>,
    /// The settings last given to the running mic test.
    test_settings: Option<AudioSettings>,
    level_db: f32,
    transmitting: bool,
    advanced: bool,
}

/// Number of bars in the level meter.
const METER_BARS: usize = 40;

impl SettingsView {
    pub(super) fn voice_page(
        &mut self,
        t: &Theme,
        s: &mut Settings,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        // Keep a running mic test on the current settings.
        let audio = crate::voice::audio_settings(s, None);
        if let Some(test) = &self.voice_page.test
            && self.voice_page.test_settings.as_ref() != Some(&audio)
        {
            test.update_settings(audio.clone());
            self.voice_page.test_settings = Some(audio);
        }

        let profile = s.voice.input_profile;
        let mut out = vec![
            heading("Voice", t),
            div()
                .w_full()
                .flex()
                .gap(px(16.))
                .child(self.device_picker(Picker::Input, s, t, cx))
                .child(self.device_picker(Picker::Output, s, t, cx))
                .into_any_element(),
            self.slider(
                "Input Volume",
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
                "Output Volume",
                s,
                |s| &mut s.voice.output_volume,
                0.,
                2.,
                0.05,
                |v| format!("{:.0}%", v * 100.),
                t,
                cx,
            ),
            self.mic_test(t, cx),
            heading("Input Profile", t),
            self.profile_option(
                InputProfile::VoiceIsolation,
                "Voice Isolation",
                "Just your voice: noise suppression, automatic gain and sensitivity handled for you.",
                profile,
                t,
                cx,
            ),
            self.profile_option(
                InputProfile::Studio,
                "Studio",
                "Pure audio: open mic with no processing.",
                profile,
                t,
                cx,
            ),
            self.profile_option(
                InputProfile::Custom,
                "Custom",
                "Advanced mode: every button and dial.",
                profile,
                t,
                cx,
            ),
        ];

        if profile == InputProfile::Custom {
            out.extend(self.custom_controls(s, t, cx));
        }

        out.push(self.voice_advanced_toggle(t, cx));
        if self.voice_page.advanced {
            out.extend(self.voice_advanced(s, t, cx));
        }
        out
    }

    fn device_picker(
        &mut self,
        which: Picker,
        s: &Settings,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (title, current) = match which {
            Picker::Input => ("Input Device", s.voice.input_device.clone()),
            Picker::Output => ("Output Device", s.voice.output_device.clone()),
        };
        let open = self.voice_page.picker == Some(which);
        if open && self.voice_page.devices.is_none() {
            self.voice_page.devices = Some(rcord_voice::devices());
        }
        let names: Vec<String> = match (&self.voice_page.devices, which) {
            (Some(d), Picker::Input) => d.inputs.clone(),
            (Some(d), Picker::Output) => d.outputs.clone(),
            (None, _) => Vec::new(),
        };
        let label = current.clone().unwrap_or_else(|| "Default".into());
        let id = match which {
            Picker::Input => "voice-input-device",
            Picker::Output => "voice-output-device",
        };
        let field = div()
            .id(id)
            .h(px(40.))
            .px(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .rounded(t.radius_small)
            .bg(t.input)
            .border_1()
            .border_color(if open { t.accent } else { t.border })
            .cursor_pointer()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(t.text)
                    .child(label),
            )
            .child(
                icon(Icon::ChevronDown)
                    .size(px(16.))
                    .text_color(t.text_muted),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                let page = &mut this.voice_page;
                if page.picker == Some(which) {
                    page.picker = None;
                } else {
                    page.picker = Some(which);
                    page.devices = None; // Re-read, in case something was plugged in.
                }
                cx.notify();
            }));

        let list = open.then(|| {
            let options = std::iter::once(None).chain(names.into_iter().map(Some));
            div()
                .mt(px(4.))
                .p(px(4.))
                .max_h(px(240.))
                .flex()
                .flex_col()
                .rounded(t.radius_small)
                .bg(t.floating)
                .border_1()
                .border_color(t.border)
                .children(options.enumerate().map(|(ix, name)| {
                    let selected = name == current;
                    let text = name.clone().unwrap_or_else(|| "Default".into());
                    div()
                        .id(ElementId::NamedInteger(id.into(), ix as u64))
                        .px(px(8.))
                        .py(px(6.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .rounded(t.radius_small)
                        .cursor_pointer()
                        .hover(|st| st.bg(t.hover))
                        .text_sm()
                        .text_color(if selected { t.header } else { t.text })
                        .child(div().min_w_0().truncate().child(text))
                        .when(selected, |el| {
                            el.child(icon(Icon::Check).size(px(14.)).text_color(t.accent))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.voice_page.picker = None;
                            let name = name.clone();
                            this.update(cx, move |s| match which {
                                Picker::Input => s.voice.input_device = name,
                                Picker::Output => s.voice.output_device = name,
                            });
                        }))
                }))
        });

        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(t.text_muted)
                    .child(title.to_uppercase()),
            )
            .child(field)
            .children(list)
            .into_any_element()
    }

    fn mic_test(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let testing = self.voice_page.test.is_some();
        let level = self.voice_page.level_db;
        let lit = if testing {
            (((level + 60.0) / 60.0).clamp(0.0, 1.0) * METER_BARS as f32).round() as usize
        } else {
            0
        };
        let on_color = if self.voice_page.transmitting {
            t.online
        } else {
            t.idle
        };
        let meter = div()
            .flex_1()
            .h(px(20.))
            .flex()
            .items_center()
            .gap(px(3.))
            .children((0..METER_BARS).map(|i| {
                div().flex_1().h(px(16.)).rounded(px(2.)).bg(if i < lit {
                    on_color
                } else {
                    t.base
                })
            }));
        let label = if testing {
            "Stop Testing"
        } else {
            "Let's Check"
        };
        div()
            .w_full()
            .py(px(8.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.header)
                    .child("Mic Test"),
            )
            .child(note(
                "Having mic issues? Start a test and say something. You hear yourself exactly as others in a call would.",
                t,
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(
                        button(
                            "voice-mic-test",
                            label,
                            if testing { t.danger } else { t.accent },
                            gpui::white(),
                            t,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_mic_test(cx))),
                    )
                    .child(meter),
            )
            .into_any_element()
    }

    fn toggle_mic_test(&mut self, cx: &mut Context<Self>) {
        let page = &mut self.voice_page;
        if page.test.take().is_some() {
            page.test_events = None;
            page.test_settings = None;
            page.level_db = -100.0;
            page.transmitting = false;
            cx.notify();
            return;
        }
        let settings = crate::voice::audio_settings(&self.state.read(cx).settings, None);
        match MicTest::start(settings.clone()) {
            Ok((test, events)) => {
                page.test = Some(test);
                page.test_settings = Some(settings);
                page.test_events = Some(cx.spawn(async move |this, cx| {
                    while let Ok(e) = events.recv().await {
                        let ok = this.update(cx, |this, cx| {
                            match e {
                                AudioEvent::InputLevel(l) => this.voice_page.level_db = l,
                                AudioEvent::Speaking {
                                    user_id: 0,
                                    speaking,
                                } => this.voice_page.transmitting = speaking,
                                AudioEvent::DeviceError(err) => this.toast(&err, cx),
                                AudioEvent::Speaking { .. } => return,
                            }
                            cx.notify();
                        });
                        if ok.is_err() {
                            break;
                        }
                    }
                }));
            }
            Err(e) => self.toast(&format!("Mic test could not start: {e}"), cx),
        }
        cx.notify();
    }

    fn profile_option(
        &self,
        value: InputProfile,
        title: &'static str,
        desc: &'static str,
        current: InputProfile,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on = value == current;
        div()
            .id(ElementId::Name(format!("voice-profile-{title}").into()))
            .w_full()
            .py(px(8.))
            .flex()
            .items_start()
            .gap(px(12.))
            .cursor_pointer()
            .child(div().pt(px(2.)).child(radio(on, t)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.header)
                            .child(title),
                    )
                    .child(div().text_sm().text_color(t.text_muted).child(desc)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.update(cx, |s| s.voice.input_profile = value);
            }))
            .into_any_element()
    }

    fn custom_controls(
        &mut self,
        s: &mut Settings,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let ptt = s.voice.input_mode == InputMode::PushToTalk;
        let mut out = vec![
            heading("Input Mode", t),
            self.mode_option(
                InputMode::VoiceActivity,
                "Voice Activity",
                "Transmit when you speak.",
                s.voice.input_mode,
                t,
                cx,
            ),
            self.mode_option(
                InputMode::PushToTalk,
                "Push to Talk",
                "Transmit only while the push-to-talk key is held.",
                s.voice.input_mode,
                t,
                cx,
            ),
        ];
        if ptt {
            out.push(self.slider_u32(
                "Push to Talk Release Delay",
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
                "Automatically Adjust Input Sensitivity",
                "Controls how much sound rcord transmits from your mic.",
                s,
                |s| &mut s.voice.auto_sensitivity,
                t,
                cx,
            ));
            if !s.voice.auto_sensitivity {
                out.push(self.slider(
                    "Input Sensitivity",
                    s,
                    |s| &mut s.voice.sensitivity_db,
                    -100.,
                    0.,
                    1.,
                    |v| format!("{v:.0} dB"),
                    t,
                    cx,
                ));
                out.push(sensitivity_bar(
                    s.voice.sensitivity_db,
                    self.voice_page
                        .test
                        .is_some()
                        .then_some(self.voice_page.level_db),
                    t,
                ));
            }
        }
        out.push(self.denoise_choice(s, t, cx));
        out.push(
            row(
                "Echo Cancellation",
                "Stops others hearing themselves through your speakers. Coming soon; use headphones meanwhile.",
                t,
            )
            .child(soon_chip(t))
            .into_any_element(),
        );
        out.push(self.toggle(
            "Automatic Gain Control",
            "Keeps your voice at a steady volume, however close you are to the mic.",
            s,
            |s| &mut s.voice.automatic_gain,
            t,
            cx,
        ));
        out
    }

    fn mode_option(
        &self,
        value: InputMode,
        title: &'static str,
        desc: &'static str,
        current: InputMode,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on = value == current;
        div()
            .id(ElementId::Name(format!("voice-mode-{title}").into()))
            .w_full()
            .py(px(6.))
            .flex()
            .items_start()
            .gap(px(12.))
            .cursor_pointer()
            .child(div().pt(px(2.)).child(radio(on, t)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.header)
                            .child(title),
                    )
                    .child(div().text_sm().text_color(t.text_muted).child(desc)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.update(cx, |s| s.voice.input_mode = value);
            }))
            .into_any_element()
    }

    fn denoise_choice(&self, s: &mut Settings, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let current = s.voice.noise_suppression;
        let options = [
            ("None", NoiseSuppression::Off),
            ("Standard", NoiseSuppression::Standard),
            ("Strong", NoiseSuppression::Strong),
        ];
        let mut seg = div()
            .flex_none()
            .flex()
            .p(px(3.))
            .gap(px(2.))
            .rounded(t.radius_small)
            .bg(t.base);
        for (ix, (label, value)) in options.into_iter().enumerate() {
            let selected = value == current;
            seg = seg.child(
                div()
                    .id(ElementId::NamedInteger("voice-denoise".into(), ix as u64))
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
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update(cx, |s| s.voice.noise_suppression = value);
                    })),
            );
        }
        row(
            "Noise Suppression",
            "Removes background noise with RNNoise, a neural network that runs on your computer. Strong also cuts keyboard clicks and noises between words.",
            t,
        )
        .child(seg)
        .into_any_element()
    }

    fn voice_advanced_toggle(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let open = self.voice_page.advanced;
        row(
            if open {
                "Hide Advanced Voice Settings"
            } else {
                "Show Advanced Voice Settings"
            },
            "Bitrate, push to talk release delay and more",
            t,
        )
        .child(
            div()
                .id("voice-advanced")
                .size(px(36.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(t.radius_small)
                .bg(t.input)
                .cursor_pointer()
                .hover(|st| st.bg(t.hover))
                .child(
                    icon(if open {
                        Icon::ChevronUp
                    } else {
                        Icon::ChevronDown
                    })
                    .size(px(18.))
                    .text_color(t.text),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.voice_page.advanced = !this.voice_page.advanced;
                    cx.notify();
                })),
        )
        .into_any_element()
    }

    fn voice_advanced(
        &mut self,
        s: &mut Settings,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        vec![
            self.slider_u32(
                "Maximum Bitrate",
                s,
                |s| &mut s.voice.max_bitrate_kbps,
                8,
                512,
                8,
                |v| format!("{v} kbps"),
                t,
                cx,
            ),
            note(
                "Audio is sent at the voice channel's bitrate, up to this cap.",
                t,
            ),
            row(
                "Global Attenuation",
                "Lower the volume of other apps while someone speaks. Coming soon.",
                t,
            )
            .child(soon_chip(t))
            .into_any_element(),
        ]
    }
}

fn soon_chip(t: &Theme) -> AnyElement {
    div()
        .flex_none()
        .px(px(8.))
        .py(px(2.))
        .rounded_full()
        .bg(t.base)
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.text_muted)
        .child("Coming soon")
        .into_any_element()
}

/// Discord's sensitivity bar: yellow below the threshold, green above, with the live mic
/// level drawn over it while a mic test runs.
fn sensitivity_bar(threshold_db: f32, level_db: Option<f32>, t: &Theme) -> AnyElement {
    let at = ((threshold_db + 100.0) / 100.0).clamp(0.0, 1.0);
    let yellow = Hsla { a: 0.9, ..t.idle };
    let green = Hsla { a: 0.9, ..t.online };
    div()
        .w_full()
        .pb(px(8.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .relative()
                .w_full()
                .h(px(8.))
                .flex()
                .rounded_full()
                .overflow_hidden()
                .child(div().h_full().w(gpui::relative(at)).bg(yellow))
                .child(div().h_full().flex_1().bg(green))
                .children(level_db.map(|l| {
                    let f = ((l + 100.0) / 100.0).clamp(0.0, 1.0);
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .h_full()
                        .w(gpui::relative(f))
                        .bg(Hsla {
                            a: 0.5,
                            ..gpui::white()
                        })
                })),
        )
        .child(note(
            "Speak during a mic test: your voice is sent while the white bar reaches the green part.",
            t,
        ))
        .into_any_element()
}
