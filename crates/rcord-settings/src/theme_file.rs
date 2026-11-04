//! Custom themes: a built-in theme as the starting point, with any of its colors and shapes
//! replaced. The same TOML is kept in settings for the theme in use and written to `.toml`
//! files to share; docs/THEMES.md describes every key.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ThemeId;

/// One themeable color: its key in `[colors]` and what it paints.
#[derive(Debug, Clone, Copy)]
pub struct ColorKey {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
}

/// Every key `[colors]` accepts, in the order the editor shows them.
pub const COLOR_KEYS: &[ColorKey] = &[
    ColorKey {
        key: "base",
        label: "Window background",
        help: "Behind everything, and the server list.",
    },
    ColorKey {
        key: "sidebar",
        label: "Sidebars",
        help: "The channel list and the member list.",
    },
    ColorKey {
        key: "surface",
        label: "Chat",
        help: "The message area.",
    },
    ColorKey {
        key: "floating",
        label: "Popups",
        help: "Menus, tooltips, profiles and pickers.",
    },
    ColorKey {
        key: "input",
        label: "Inputs",
        help: "The message box and text fields.",
    },
    ColorKey {
        key: "hover",
        label: "Hover",
        help: "Rows under the pointer. Usually translucent.",
    },
    ColorKey {
        key: "selected",
        label: "Selected",
        help: "The open channel and selected items. Usually translucent.",
    },
    ColorKey {
        key: "border",
        label: "Borders",
        help: "Lines between panels and around cards.",
    },
    ColorKey {
        key: "text",
        label: "Text",
        help: "Messages and most labels.",
    },
    ColorKey {
        key: "text_muted",
        label: "Muted text",
        help: "Channel names, descriptions and secondary labels.",
    },
    ColorKey {
        key: "text_faint",
        label: "Faint text",
        help: "Timestamps, placeholders and hints.",
    },
    ColorKey {
        key: "header",
        label: "Headings",
        help: "Names, titles and the open channel.",
    },
    ColorKey {
        key: "accent",
        label: "Accent",
        help: "Buttons, switches and highlights. Replaces the accent color setting.",
    },
    ColorKey {
        key: "accent_text",
        label: "Text on accent",
        help: "Text on accent buttons. Picked for contrast when not set.",
    },
    ColorKey {
        key: "link",
        label: "Links",
        help: "Links in messages.",
    },
    ColorKey {
        key: "danger",
        label: "Danger",
        help: "Errors, delete buttons and mention counts.",
    },
    ColorKey {
        key: "online",
        label: "Online",
        help: "The online status dot.",
    },
    ColorKey {
        key: "idle",
        label: "Idle",
        help: "The idle status dot.",
    },
    ColorKey {
        key: "dnd",
        label: "Do Not Disturb",
        help: "The do-not-disturb status dot.",
    },
    ColorKey {
        key: "offline",
        label: "Offline",
        help: "The offline status dot.",
    },
    ColorKey {
        key: "mention_bg",
        label: "Mention background",
        help: "Behind @mentions in messages.",
    },
    ColorKey {
        key: "mention_text",
        label: "Mention text",
        help: "The text of @mentions.",
    },
    ColorKey {
        key: "mentioned_row",
        label: "Mentioned message",
        help: "Behind messages that mention you. Usually translucent.",
    },
    ColorKey {
        key: "code_bg",
        label: "Code background",
        help: "Inline code and code blocks.",
    },
    ColorKey {
        key: "scrim",
        label: "Dimming",
        help: "Over the app behind dialogs. Usually translucent black.",
    },
];

/// A custom theme, as kept in settings and in `.toml` theme files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeFile {
    pub name: String,
    pub author: String,
    /// The built-in theme whose colors are used for every key not set in `colors`.
    pub base: ThemeId,
    /// Colors as `#rrggbb` or `#rrggbbaa`, by key in [`COLOR_KEYS`].
    pub colors: BTreeMap<String, String>,
    /// Corner radius of panels, cards and inputs, 0 to 24; replaces the setting when set.
    pub radius: Option<f32>,
    /// Corner radius of small items such as buttons and inputs, 0 to 16.
    pub radius_small: Option<f32>,
    /// Interface font family; replaces the font setting when set.
    pub ui_font: Option<String>,
    /// Code font family.
    pub code_font: Option<String>,
}

impl Default for ThemeFile {
    fn default() -> Self {
        Self {
            name: "My theme".into(),
            author: String::new(),
            base: ThemeId::Dark,
            colors: BTreeMap::new(),
            radius: None,
            radius_small: None,
            ui_font: None,
            code_font: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("could not read the theme: {0}")]
    Io(#[from] std::io::Error),
    #[error("the theme file is not valid TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("could not write the theme: {0}")]
    Write(#[from] toml::ser::Error),
}

/// Parses `#rgb`, `#rrggbb` or `#rrggbbaa` into 0xRRGGBBAA.
pub fn parse_color(text: &str) -> Option<u32> {
    let hex = text.trim().trim_start_matches('#');
    let value = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        3 => {
            let (r, g, b) = ((value >> 8) & 0xf, (value >> 4) & 0xf, value & 0xf);
            Some((r * 17) << 24 | (g * 17) << 16 | (b * 17) << 8 | 0xff)
        }
        6 => Some(value << 8 | 0xff),
        8 => Some(value),
        _ => None,
    }
}

impl ThemeFile {
    pub fn from_toml(text: &str) -> Result<Self, ThemeError> {
        let mut theme: Self = toml::from_str(text)?;
        theme.sanitize();
        Ok(theme)
    }

    pub fn to_toml(&self) -> Result<String, ThemeError> {
        Ok(format!(
            "# rcord theme. Every key is described in docs/THEMES.md.\n{}",
            toml::to_string_pretty(self)?
        ))
    }

    pub fn load(path: &std::path::Path) -> Result<Self, ThemeError> {
        Self::from_toml(&std::fs::read_to_string(path)?)
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), ThemeError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_toml()?)?;
        Ok(())
    }

    /// The color set for `key`, as 0xRRGGBBAA.
    pub fn color(&self, key: &str) -> Option<u32> {
        self.colors.get(key).and_then(|c| parse_color(c))
    }

    /// Drops unknown keys and unreadable colors, and clamps the shapes.
    pub fn sanitize(&mut self) {
        self.colors
            .retain(|k, v| COLOR_KEYS.iter().any(|c| c.key == k) && parse_color(v).is_some());
        if self.base == ThemeId::Custom {
            self.base = ThemeId::Dark;
        }
        self.radius = self
            .radius
            .filter(|r| r.is_finite())
            .map(|r| r.clamp(0.0, 24.0));
        self.radius_small = self
            .radius_small
            .filter(|r| r.is_finite())
            .map(|r| r.clamp(0.0, 16.0));
        for font in [&mut self.ui_font, &mut self.code_font] {
            if font.as_deref().is_some_and(|f| f.trim().is_empty()) {
                *font = None;
            }
        }
        if self.name.trim().is_empty() {
            self.name = "Untitled theme".into();
        }
    }

    /// A file name for the theme: its name in lower case with dashes.
    pub fn file_name(&self) -> String {
        let slug: String = self
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let slug = slug.trim_matches('-');
        format!("{}.toml", if slug.is_empty() { "theme" } else { slug })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colors() {
        assert_eq!(parse_color("#fff"), Some(0xffffffff));
        assert_eq!(parse_color("5865F2"), Some(0x5865f2ff));
        assert_eq!(parse_color("#00000080"), Some(0x00000080));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("red"), None);
    }

    #[test]
    fn round_trips_and_sanitizes() {
        let text = r##"
            name = "Ocean"
            base = "midnight"
            radius = 99
            [colors]
            accent = "#00a8fc"
            surface = "#0b1622"
            nonsense = "#fff"
            text = "blue"
        "##;
        let theme = ThemeFile::from_toml(text).unwrap();
        assert_eq!(theme.base, ThemeId::Midnight);
        assert_eq!(theme.radius, Some(24.0));
        assert_eq!(theme.colors.len(), 2);
        assert_eq!(theme.color("accent"), Some(0x00a8fcff));
        let again = ThemeFile::from_toml(&theme.to_toml().unwrap()).unwrap();
        assert_eq!(again, theme);
        assert_eq!(theme.file_name(), "ocean.toml");
    }
}
