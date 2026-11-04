//! Colors and metrics derived from the appearance settings. Rebuilt only when a setting
//! changes, then read by every view as a global.

use gpui::{App, Global, Hsla, Pixels, Rgba, px, rgb, rgba};
use rcord_model::Status;
use rcord_settings::{Appearance, COLOR_KEYS, ThemeFile, ThemeId};

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
    /// Interface font family.
    pub ui_font: gpui::SharedString,
    /// Code block font family.
    pub code_font: gpui::SharedString,
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
        let custom = a
            .custom_theme
            .as_ref()
            .filter(|_| a.theme == ThemeId::Custom);
        let p = match custom.map_or(a.theme, |c| c.base) {
            ThemeId::Dark | ThemeId::Custom => &DARK,
            ThemeId::Midnight => &MIDNIGHT,
            ThemeId::Ash => &ASH,
            ThemeId::Light => &LIGHT,
        };
        let accent_rgb = custom
            .and_then(|c| c.color("accent"))
            .map_or(a.accent, |c| c >> 8);
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
        let accent = c(accent_rgb);
        let accent_hover = Hsla {
            l: (accent.l - 0.06).max(0.0),
            ..accent
        };
        let accent_text = if relative_luminance(accent_rgb) > 0.45 {
            hsla(0x000000)
        } else {
            hsla(0xffffff)
        };
        let link = if p.overlay_light {
            c(0x00a8fc)
        } else {
            c(0x006ce7)
        };
        let mut theme = Self {
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
            ui_font: pick_font(
                custom.and_then(|c| c.ui_font.as_ref()),
                a.ui_font.as_ref(),
                "Inter",
            ),
            code_font: pick_font(
                custom.and_then(|c| c.code_font.as_ref()),
                a.code_font.as_ref(),
                "JetBrains Mono",
            ),
        };
        if let Some(custom) = custom {
            theme.apply(custom, sat);
        }
        theme
    }

    /// Replaces the colors and shapes a custom theme sets.
    fn apply(&mut self, custom: &ThemeFile, sat: f32) {
        for key in COLOR_KEYS {
            let Some(rgba_value) = custom.color(key.key) else {
                continue;
            };
            let color = saturate(rgba(rgba_value).into(), sat);
            let slot = match key.key {
                "base" => &mut self.base,
                "sidebar" => &mut self.sidebar,
                "surface" => &mut self.surface,
                "floating" => &mut self.floating,
                "input" => &mut self.input,
                "hover" => &mut self.hover,
                "selected" => &mut self.selected,
                "border" => &mut self.border,
                "text" => &mut self.text,
                "text_muted" => &mut self.text_muted,
                "text_faint" => &mut self.text_faint,
                "header" => &mut self.header,
                "accent" => {
                    self.accent_hover = Hsla {
                        l: (color.l - 0.06).max(0.0),
                        ..color
                    };
                    &mut self.accent
                }
                "accent_text" => &mut self.accent_text,
                "link" => &mut self.link,
                "danger" => &mut self.danger,
                "online" => &mut self.online,
                "idle" => &mut self.idle,
                "dnd" => &mut self.dnd,
                "offline" => &mut self.offline,
                "mention_bg" => &mut self.mention_bg,
                "mention_text" => &mut self.mention_text,
                "mentioned_row" => &mut self.mentioned_row,
                "code_bg" => &mut self.code_bg,
                "scrim" => &mut self.scrim,
                _ => continue,
            };
            *slot = color;
        }
        if let Some(r) = custom.radius {
            self.radius = px(r);
            if custom.radius_small.is_none() {
                self.radius_small = px((r * 0.5).max(2.0));
            }
        }
        if let Some(r) = custom.radius_small {
            self.radius_small = px(r);
        }
    }

    /// The theme's colors as theme-file text, for starting a custom theme from this one.
    pub fn color_of(&self, key: &str) -> Option<Hsla> {
        Some(match key {
            "base" => self.base,
            "sidebar" => self.sidebar,
            "surface" => self.surface,
            "floating" => self.floating,
            "input" => self.input,
            "hover" => self.hover,
            "selected" => self.selected,
            "border" => self.border,
            "text" => self.text,
            "text_muted" => self.text_muted,
            "text_faint" => self.text_faint,
            "header" => self.header,
            "accent" => self.accent,
            "accent_text" => self.accent_text,
            "link" => self.link,
            "danger" => self.danger,
            "online" => self.online,
            "idle" => self.idle,
            "dnd" => self.dnd,
            "offline" => self.offline,
            "mention_bg" => self.mention_bg,
            "mention_text" => self.mention_text,
            "mentioned_row" => self.mentioned_row,
            "code_bg" => self.code_bg,
            "scrim" => self.scrim,
            _ => return None,
        })
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

fn pick_font(
    theme: Option<&String>,
    setting: Option<&String>,
    default: &'static str,
) -> gpui::SharedString {
    theme
        .or(setting)
        .map(|f| f.trim())
        .filter(|f| !f.is_empty())
        .map_or_else(|| default.into(), |f| f.to_owned().into())
}

/// `#rrggbb`, or `#rrggbbaa` when the color is translucent.
pub fn hex(color: Hsla) -> String {
    let c = Rgba::from(color);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    let rgb = byte(c.r) << 16 | byte(c.g) << 8 | byte(c.b);
    if c.a >= 0.999 {
        format!("#{rgb:06x}")
    } else {
        format!("#{rgb:06x}{:02x}", byte(c.a))
    }
}

#[cfg(test)]
mod custom_tests {
    use super::*;

    #[test]
    fn custom_theme_overrides_colors() {
        let theme = ThemeFile {
            base: ThemeId::Ash,
            colors: [("surface", "#102030"), ("accent", "#ffffff")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            radius: Some(12.0),
            ..ThemeFile::default()
        };
        let a = Appearance {
            theme: ThemeId::Custom,
            custom_theme: Some(theme),
            ..Appearance::default()
        };
        let t = Theme::from_settings(&a);
        assert_eq!(hex(t.surface), "#102030");
        assert_eq!(hex(t.sidebar), "#2b2d31");
        assert_eq!(hex(t.accent_text), "#000000");
        assert_eq!(t.radius, px(12.0));
    }
}
