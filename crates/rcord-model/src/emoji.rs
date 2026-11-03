use serde::{Deserialize, Serialize};

use crate::Snowflake;

/// A custom emoji (with an ID) or a Unicode emoji (name only).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Emoji {
    #[serde(default)]
    pub id: Option<Snowflake>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub animated: bool,
    #[serde(default)]
    pub roles: Vec<Snowflake>,
    #[serde(default = "yes")]
    pub available: bool,
    #[serde(default)]
    pub managed: bool,
}

fn yes() -> bool {
    true
}

impl Emoji {
    /// The form a message uses to embed it: `<:name:id>`, `<a:name:id>`, or the character.
    pub fn to_message_form(&self) -> String {
        match (self.id, self.name.as_deref()) {
            (Some(id), name) => format!(
                "<{}:{}:{id}>",
                if self.animated { "a" } else { "" },
                name.unwrap_or("_")
            ),
            (None, Some(name)) => name.to_owned(),
            (None, None) => String::new(),
        }
    }

    /// The form the reactions endpoint expects in its path, before URL encoding.
    pub fn to_reaction_form(&self) -> String {
        match (self.id, self.name.as_deref()) {
            (Some(id), name) => format!("{}:{id}", name.unwrap_or("_")),
            (None, Some(name)) => name.to_owned(),
            (None, None) => String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "u8", into = "u8")]
pub enum StickerFormat {
    Png,
    Apng,
    Lottie,
    Gif,
    Unknown(u8),
}

impl From<u8> for StickerFormat {
    fn from(v: u8) -> Self {
        match v {
            1 => Self::Png,
            2 => Self::Apng,
            3 => Self::Lottie,
            4 => Self::Gif,
            other => Self::Unknown(other),
        }
    }
}

impl From<StickerFormat> for u8 {
    fn from(v: StickerFormat) -> Self {
        match v {
            StickerFormat::Png => 1,
            StickerFormat::Apng => 2,
            StickerFormat::Lottie => 3,
            StickerFormat::Gif => 4,
            StickerFormat::Unknown(other) => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sticker {
    pub id: Snowflake,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: String,
    pub format_type: StickerFormat,
    #[serde(default)]
    pub pack_id: Option<Snowflake>,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    #[serde(default = "yes")]
    pub available: bool,
}

/// The short form of a sticker attached to a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StickerItem {
    pub id: Snowflake,
    pub name: String,
    pub format_type: StickerFormat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StickerPack {
    pub id: Snowflake,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub stickers: Vec<Sticker>,
    #[serde(default)]
    pub banner_asset_id: Option<Snowflake>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionCounts {
    #[serde(default)]
    pub burst: u32,
    #[serde(default)]
    pub normal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub count: u32,
    #[serde(default)]
    pub count_details: ReactionCounts,
    #[serde(default)]
    pub me: bool,
    #[serde(default)]
    pub me_burst: bool,
    pub emoji: Emoji,
    /// Hex colors of a super reaction's burst animation.
    #[serde(default)]
    pub burst_colors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_and_reaction_forms() {
        let e: Emoji =
            serde_json::from_str(r#"{"id":"41771983429993937","name":"LUL","animated":true}"#)
                .unwrap();
        assert_eq!(e.to_message_form(), "<a:LUL:41771983429993937>");
        assert_eq!(e.to_reaction_form(), "LUL:41771983429993937");
        let u: Emoji = serde_json::from_str(r#"{"id":null,"name":"🔥"}"#).unwrap();
        assert_eq!(u.to_message_form(), "🔥");
        assert!(u.available);
    }
}
