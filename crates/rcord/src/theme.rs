//! Colors and metrics derived from the appearance settings. Rebuilt only when a setting
//! changes, then read by every view as a global.

use gpui::{App, Global, Hsla, Pixels, Rgba, px, rgb, rgba};
use rcord_model::Status;
use rcord_settings::{Appearance, ThemeId};

#[derive(Debug, Clone)]
pub struct Theme {
    /// Window background and the server list.
    pub base: Hsla,
    /// Channel list and member list.
    pub sidebar: Hsla,
    /// Chat area.
    pub surface: Hsla,
    /// Popovers, menus, tooltips.
    pub floating: Hsla,
    pub input: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,
    pub header: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub accent_text: Hsla,
    pub link: Hsla,
    pub danger: Hsla,
    pub online: Hsla,
    pub idle: Hsla,
    pub dnd: Hsla,
    pub offline: Hsla,
    pub mention_bg: Hsla,
    pub mention_text: Hsla,
    pub mentioned_row: Hsla,
    pub code_bg: Hsla,
    pub scrim: Hsla,
    pub radius: Pixels,
    pub radius_small: Pixels,
}

impl Global for Theme {}

struct Palette {
    base: u32,
    sidebar: u32,
    surface: u32,
    floating: u32,
    input: u32,
    border: u32,
    text: u32,
    text_muted: u32,
    text_faint: u32,
    header: u32,
    /// Hover and selection are white or black overlays at these alphas.
    overlay_light: bool,
}

const DARK: Palette = Palette {
    base: 0x0f1012,
    sidebar: 0x16171a,
    surface: 0x1b1c20,
    floating: 0x202126,
    input: 0x24252b,
    border: 0x26272d,
    text: 0xdcdee3,
    text_muted: 0x9b9ea6,
    text_faint: 0x6d7079,
    header: 0xf3f4f6,
    overlay_light: true,
};

const MIDNIGHT: Palette = Palette {
    base: 0x000000,
    sidebar: 0x08080a,
    surface: 0x0c0c0e,
    floating: 0x141417,
    input: 0x17171b,
    border: 0x1c1c21,
    text: 0xd8dade,
    text_muted: 0x92959d,
    text_faint: 0x62656d,
    header: 0xf3f4f6,
    overlay_light: true,
};

const ASH: Palette = Palette {
    base: 0x1e1f22,
    sidebar: 0x2b2d31,
    surface: 0x313338,
    floating: 0x111214,
    input: 0x383a40,
    border: 0x3f4147,
    text: 0xdbdee1,
    text_muted: 0xb5bac1,
    text_faint: 0x80848e,
    header: 0xf2f3f5,
    overlay_light: true,
};

const LIGHT: Palette = Palette {
    base: 0xe3e5e8,
    sidebar: 0xf2f3f5,
    surface: 0xffffff,
    floating: 0xffffff,
    input: 0xebedef,
    border: 0xe1e2e4,
    text: 0x2e3338,
    text_muted: 0x4f5660,
    text_faint: 0x80848e,
    header: 0x060607,
    overlay_light: false,
};

fn hsla(color: u32) -> Hsla {
    rgb(color).into()
}

fn with_alpha(color: Hsla, a: f32) -> Hsla {
    Hsla { a, ..color }
}

fn saturate(color: Hsla, amount: f32) -> Hsla {
    Hsla {
        s: color.s * amount,
        ..color
    }
}

impl Theme {
    pub fn from_settings(a: &Appearance) -> Self {
        let p = match a.theme {
            ThemeId::Dark => &DARK,
            ThemeId::Midnight => &MIDNIGHT,
            ThemeId::Ash => &ASH,
            ThemeId::Light => &LIGHT,
        };
        let sat = a.saturation;
        let c = |v: u32| saturate(hsla(v), sat);
        let overlay = |alpha: f32| {
            if p.overlay_light {
                rgba(0xffffff00 | (alpha * 255.0) as u32).into()
            } else {
                Hsla::from(Rgba {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: alpha,
                })
            }
        };
        let accent = c(a.accent);
        let accent_hover = Hsla {
            l: (accent.l - 0.06).max(0.0),
            ..accent
        };
        let accent_text = if relative_luminance(a.accent) > 0.45 {
            hsla(0x000000)
        } else {
            hsla(0xffffff)
        };
        let link = if p.overlay_light {
            c(0x00a8fc)
        } else {
            c(0x006ce7)
        };
        Self {
            base: c(p.base),
            sidebar: c(p.sidebar),
            surface: c(p.surface),
            floating: c(p.floating),
            input: c(p.input),
            hover: overlay(0.045),
            selected: overlay(0.085),
            border: c(p.border),
            text: c(p.text),
            text_muted: c(p.text_muted),
            text_faint: c(p.text_faint),
            header: c(p.header),
            accent,
            accent_hover,
            accent_text,
            link,
            danger: c(0xf23f43),
            online: c(0x23a55a),
            idle: c(0xf0b232),
            dnd: c(0xf23f43),
            offline: c(0x80848e),
            mention_bg: with_alpha(accent, 0.24),
            mention_text: if p.overlay_light {
                Hsla {
                    l: (accent.l + 0.22).min(0.92),
                    ..accent
                }
            } else {
                accent
            },
            mentioned_row: with_alpha(c(0xf0b232), 0.08),
            code_bg: if p.overlay_light {
                overlay(0.06)
            } else {
                overlay(0.05)
            },
            scrim: Hsla::from(Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.7,
            }),
            radius: px(a.corner_radius),
            radius_small: px((a.corner_radius * 0.5).max(2.0)),
        }
    }

    pub fn status(&self, status: Status) -> Hsla {
        match status {
            Status::Online => self.online,
            Status::Idle => self.idle,
            Status::Dnd => self.dnd,
            Status::Invisible | Status::Offline => self.offline,
        }
    }

    /// A role color made readable against the surface when contrast boosting is on.
    pub fn role_color(&self, color: u32, boost: bool) -> Hsla {
        let c = hsla(color);
        if !boost {
            return c;
        }
        let dark_ui = self.surface.l < 0.5;
        if dark_ui && c.l < 0.55 {
            Hsla { l: 0.62, ..c }
        } else if !dark_ui && c.l > 0.45 {
            Hsla { l: 0.38, ..c }
        } else {
            c
        }
    }
}

/// WCAG relative luminance of a 0xRRGGBB color.
fn relative_luminance(rgb: u32) -> f32 {
    let channel = |shift: u32| {
        let c = ((rgb >> shift) & 0xff) as f32 / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

pub fn theme(cx: &App) -> &Theme {
    cx.global::<Theme>()
}
