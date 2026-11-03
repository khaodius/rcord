use serde::{Deserialize, Serialize};

use crate::{PermissionOverwrite, Snowflake, User};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "u8", into = "u8")]
pub enum ChannelKind {
    Text,
    Dm,
    Voice,
    GroupDm,
    Category,
    Announcement,
    AnnouncementThread,
    PublicThread,
    PrivateThread,
    Stage,
    Directory,
    Forum,
    Media,
    Unknown(u8),
}

impl ChannelKind {
    pub const fn is_thread(self) -> bool {
        matches!(
            self,
            Self::AnnouncementThread | Self::PublicThread | Self::PrivateThread
        )
    }

    pub const fn is_voice(self) -> bool {
        matches!(self, Self::Voice | Self::Stage)
    }

    pub const fn is_private(self) -> bool {
        matches!(self, Self::Dm | Self::GroupDm)
    }

    /// Whether messages can be read in the channel itself.
    pub const fn has_messages(self) -> bool {
        matches!(
            self,
            Self::Text
                | Self::Dm
                | Self::GroupDm
                | Self::Announcement
                | Self::AnnouncementThread
                | Self::PublicThread
                | Self::PrivateThread
                | Self::Voice
                | Self::Stage
        )
    }
}

impl From<u8> for ChannelKind {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::Text,
            1 => Self::Dm,
            2 => Self::Voice,
            3 => Self::GroupDm,
            4 => Self::Category,
            5 => Self::Announcement,
            10 => Self::AnnouncementThread,
            11 => Self::PublicThread,
            12 => Self::PrivateThread,
            13 => Self::Stage,
            14 => Self::Directory,
            15 => Self::Forum,
            16 => Self::Media,
            other => Self::Unknown(other),
        }
    }
}

impl From<ChannelKind> for u8 {
    fn from(v: ChannelKind) -> Self {
        match v {
            ChannelKind::Text => 0,
            ChannelKind::Dm => 1,
            ChannelKind::Voice => 2,
            ChannelKind::GroupDm => 3,
            ChannelKind::Category => 4,
            ChannelKind::Announcement => 5,
            ChannelKind::AnnouncementThread => 10,
            ChannelKind::PublicThread => 11,
            ChannelKind::PrivateThread => 12,
            ChannelKind::Stage => 13,
            ChannelKind::Directory => 14,
            ChannelKind::Forum => 15,
            ChannelKind::Media => 16,
            ChannelKind::Unknown(other) => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    pub id: Snowflake,
    #[serde(rename = "type")]
    pub kind: ChannelKind,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub position: i32,
    #[serde(default)]
    pub parent_id: Option<Snowflake>,
    #[serde(default)]
    pub nsfw: bool,
    #[serde(default)]
    pub last_message_id: Option<Snowflake>,
    #[serde(default)]
    pub rate_limit_per_user: Option<u32>,
    #[serde(default)]
    pub bitrate: Option<u32>,
    #[serde(default)]
    pub user_limit: Option<u32>,
    /// Full users, or empty when READY sends `recipient_ids` instead.
    #[serde(default)]
    pub recipients: Vec<User>,
    #[serde(default)]
    pub recipient_ids: Vec<Snowflake>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub owner_id: Option<Snowflake>,
    #[serde(default)]
    pub permission_overwrites: Vec<PermissionOverwrite>,
    #[serde(default)]
    pub flags: u64,
}

impl Channel {
    /// IDs of the other people in a DM or group DM, whichever form READY used.
    pub fn recipient_ids(&self) -> impl Iterator<Item = Snowflake> + '_ {
        self.recipient_ids
            .iter()
            .copied()
            .chain(self.recipients.iter().map(|u| u.id))
    }

    /// Sort key matching Discord's sidebar: position, then text before voice, then ID.
    pub fn sort_key(&self) -> (bool, i32, Snowflake) {
        (self.kind.is_voice(), self.position, self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dm_with_recipient_ids() {
        let c: Channel = serde_json::from_value(serde_json::json!({
            "id": "1", "type": 1, "recipient_ids": ["7"], "last_message_id": "20"
        }))
        .unwrap();
        assert_eq!(c.kind, ChannelKind::Dm);
        assert!(c.kind.is_private());
        assert_eq!(
            c.recipient_ids().collect::<Vec<_>>(),
            vec![Snowflake::new(7).unwrap()]
        );
    }

    #[test]
    fn unknown_kinds_round_trip() {
        let c: Channel =
            serde_json::from_value(serde_json::json!({"id": "1", "type": 99})).unwrap();
        assert_eq!(c.kind, ChannelKind::Unknown(99));
        assert_eq!(serde_json::to_value(c.kind).unwrap(), 99);
    }
}
