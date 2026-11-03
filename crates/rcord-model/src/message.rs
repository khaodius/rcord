use serde::{Deserialize, Serialize};

use crate::{Channel, Member, Reaction, Snowflake, StickerItem, User};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "u8", into = "u8")]
pub enum MessageKind {
    Default,
    RecipientAdd,
    RecipientRemove,
    Call,
    ChannelNameChange,
    ChannelIconChange,
    ChannelPinnedMessage,
    UserJoin,
    GuildBoost,
    GuildBoostTier1,
    GuildBoostTier2,
    GuildBoostTier3,
    ChannelFollowAdd,
    ThreadCreated,
    Reply,
    ChatInputCommand,
    ThreadStarterMessage,
    ContextMenuCommand,
    AutoModerationAction,
    PollResult,
    Unknown(u8),
}

impl From<u8> for MessageKind {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::Default,
            1 => Self::RecipientAdd,
            2 => Self::RecipientRemove,
            3 => Self::Call,
            4 => Self::ChannelNameChange,
            5 => Self::ChannelIconChange,
            6 => Self::ChannelPinnedMessage,
            7 => Self::UserJoin,
            8 => Self::GuildBoost,
            9 => Self::GuildBoostTier1,
            10 => Self::GuildBoostTier2,
            11 => Self::GuildBoostTier3,
            12 => Self::ChannelFollowAdd,
            18 => Self::ThreadCreated,
            19 => Self::Reply,
            20 => Self::ChatInputCommand,
            21 => Self::ThreadStarterMessage,
            23 => Self::ContextMenuCommand,
            24 => Self::AutoModerationAction,
            46 => Self::PollResult,
            other => Self::Unknown(other),
        }
    }
}

impl From<MessageKind> for u8 {
    fn from(v: MessageKind) -> Self {
        match v {
            MessageKind::Default => 0,
            MessageKind::RecipientAdd => 1,
            MessageKind::RecipientRemove => 2,
            MessageKind::Call => 3,
            MessageKind::ChannelNameChange => 4,
            MessageKind::ChannelIconChange => 5,
            MessageKind::ChannelPinnedMessage => 6,
            MessageKind::UserJoin => 7,
            MessageKind::GuildBoost => 8,
            MessageKind::GuildBoostTier1 => 9,
            MessageKind::GuildBoostTier2 => 10,
            MessageKind::GuildBoostTier3 => 11,
            MessageKind::ChannelFollowAdd => 12,
            MessageKind::ThreadCreated => 18,
            MessageKind::Reply => 19,
            MessageKind::ChatInputCommand => 20,
            MessageKind::ThreadStarterMessage => 21,
            MessageKind::ContextMenuCommand => 23,
            MessageKind::AutoModerationAction => 24,
            MessageKind::PollResult => 46,
            MessageKind::Unknown(other) => other,
        }
    }
}

impl MessageKind {
    /// System messages are drawn as a single centered line rather than a chat bubble.
    pub const fn is_system(self) -> bool {
        !matches!(
            self,
            Self::Default | Self::Reply | Self::ChatInputCommand | Self::ContextMenuCommand
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: Snowflake,
    pub channel_id: Snowflake,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    pub author: User,
    #[serde(default)]
    pub member: Option<Member>,
    #[serde(default)]
    pub content: String,
    pub timestamp: String,
    #[serde(default)]
    pub edited_timestamp: Option<String>,
    #[serde(default)]
    pub tts: bool,
    #[serde(default)]
    pub mention_everyone: bool,
    #[serde(default)]
    pub mentions: Vec<User>,
    #[serde(default)]
    pub mention_roles: Vec<Snowflake>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub embeds: Vec<Embed>,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
    #[serde(default)]
    pub sticker_items: Vec<StickerItem>,
    #[serde(default)]
    pub nonce: Option<Nonce>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub webhook_id: Option<Snowflake>,
    #[serde(rename = "type", default = "default_kind")]
    pub kind: MessageKind,
    #[serde(default)]
    pub flags: u64,
    #[serde(default)]
    pub message_reference: Option<MessageReference>,
    #[serde(default)]
    pub referenced_message: Option<Box<Message>>,
    #[serde(default)]
    pub thread: Option<Box<Channel>>,
}

fn default_kind() -> MessageKind {
    MessageKind::Default
}

impl Message {
    /// Unix milliseconds, from the ID, which avoids parsing `timestamp`.
    pub fn created_at_ms(&self) -> u64 {
        self.id.timestamp_ms()
    }

    pub fn mentions_user(&self, user_id: Snowflake) -> bool {
        self.mention_everyone || self.mentions.iter().any(|u| u.id == user_id)
    }
}

/// Clients send a string nonce; some bots send a number.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Nonce {
    Str(String),
    Num(i64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageReference {
    #[serde(default)]
    pub message_id: Option<Snowflake>,
    #[serde(default)]
    pub channel_id: Option<Snowflake>,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Snowflake,
    pub filename: String,
    #[serde(default)]
    pub size: u64,
    pub url: String,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Voice messages carry their length and a base64 waveform.
    #[serde(default)]
    pub duration_secs: Option<f64>,
    #[serde(default)]
    pub waveform: Option<String>,
    #[serde(default)]
    pub flags: u64,
}

impl Attachment {
    pub fn is_image(&self) -> bool {
        self.content_type
            .as_deref()
            .is_some_and(|t| t.starts_with("image/"))
    }

    pub fn is_video(&self) -> bool {
        self.content_type
            .as_deref()
            .is_some_and(|t| t.starts_with("video/"))
    }

    /// Spoilered attachments are uploaded with a `SPOILER_` prefix.
    pub fn is_spoiler(&self) -> bool {
        self.filename.starts_with("SPOILER_")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Embed {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub color: Option<u32>,
    #[serde(default)]
    pub footer: Option<EmbedFooter>,
    #[serde(default)]
    pub image: Option<EmbedMedia>,
    #[serde(default)]
    pub thumbnail: Option<EmbedMedia>,
    #[serde(default)]
    pub video: Option<EmbedMedia>,
    #[serde(default)]
    pub provider: Option<EmbedProvider>,
    #[serde(default)]
    pub author: Option<EmbedAuthor>,
    #[serde(default)]
    pub fields: Vec<EmbedField>,
}

impl Embed {
    /// Tenor and Giphy links arrive as `gifv` embeds and are drawn as the media alone.
    pub fn is_gifv(&self) -> bool {
        self.kind.as_deref() == Some("gifv")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedFooter {
    pub text: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub proxy_icon_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedMedia {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedProvider {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedAuthor {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub proxy_icon_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub inline: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_reply_with_sticker_and_gifv() {
        let m: Message = serde_json::from_value(serde_json::json!({
            "id": "1150000000000000000",
            "channel_id": "2",
            "author": {"id": "3", "username": "a"},
            "content": "look",
            "timestamp": "2023-09-01T00:00:00.000000+00:00",
            "type": 19,
            "nonce": 12345,
            "message_reference": {"message_id": "9", "channel_id": "2"},
            "sticker_items": [{"id": "4", "name": "wave", "format_type": 3}],
            "embeds": [{"type": "gifv", "url": "https://tenor.com/x",
                        "video": {"url": "https://media.tenor.com/x.mp4", "width": 220, "height": 200}}],
            "reactions": [{"count": 2, "me": true, "emoji": {"id": null, "name": "👍"}}]
        }))
        .unwrap();
        assert_eq!(m.kind, MessageKind::Reply);
        assert!(!m.kind.is_system());
        assert_eq!(m.nonce, Some(Nonce::Num(12345)));
        assert!(m.embeds[0].is_gifv());
        assert_eq!(m.reactions[0].count, 2);
        assert!(m.created_at_ms() > crate::snowflake::DISCORD_EPOCH_MS);
    }
}
