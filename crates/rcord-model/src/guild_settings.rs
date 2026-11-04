//! Your own notification settings for each server and its channels: mutes, how many
//! notifications to get, and whether muted channels are hidden.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::Snowflake;

/// How many notifications a server or channel gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(from = "u8", into = "u8")]
pub enum NotifyLevel {
    All,
    Mentions,
    Nothing,
    /// A channel follows its server; a server follows its own default.
    #[default]
    Inherit,
}

impl From<u8> for NotifyLevel {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::All,
            1 => Self::Mentions,
            2 => Self::Nothing,
            _ => Self::Inherit,
        }
    }
}

impl From<NotifyLevel> for u8 {
    fn from(v: NotifyLevel) -> Self {
        match v {
            NotifyLevel::All => 0,
            NotifyLevel::Mentions => 1,
            NotifyLevel::Nothing => 2,
            NotifyLevel::Inherit => 3,
        }
    }
}

impl NotifyLevel {
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All Messages",
            Self::Mentions => "Only @mentions",
            Self::Nothing => "Nothing",
            Self::Inherit => "Use Server Default",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MuteConfig {
    /// When the mute ends, or never.
    #[serde(default)]
    pub end_time: Option<String>,
    /// The length picked, in seconds; -1 is until turned off.
    #[serde(default)]
    pub selected_time_window: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChannelOverride {
    pub channel_id: Snowflake,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub mute_config: Option<MuteConfig>,
    #[serde(default)]
    pub message_notifications: NotifyLevel,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct UserGuildSettings {
    /// Missing or null for the settings that cover direct messages.
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub mute_config: Option<MuteConfig>,
    #[serde(default)]
    pub message_notifications: NotifyLevel,
    #[serde(default)]
    pub suppress_everyone: bool,
    #[serde(default)]
    pub hide_muted_channels: bool,
    #[serde(default)]
    pub channel_overrides: Vec<ChannelOverride>,
}

/// Whether a mute is in force at `now`: muted, and with no end or an end still ahead.
pub fn mute_active(muted: bool, config: Option<&MuteConfig>, now: OffsetDateTime) -> bool {
    if !muted {
        return false;
    }
    match config.and_then(|c| c.end_time.as_deref()) {
        None => true,
        Some(end) => OffsetDateTime::parse(end, &Rfc3339).map_or(true, |end| end > now),
    }
}

impl UserGuildSettings {
    pub fn channel(&self, id: Snowflake) -> Option<&ChannelOverride> {
        self.channel_overrides.iter().find(|o| o.channel_id == id)
    }

    pub fn is_muted(&self, now: OffsetDateTime) -> bool {
        mute_active(self.muted, self.mute_config.as_ref(), now)
    }

    pub fn is_channel_muted(&self, id: Snowflake, now: OffsetDateTime) -> bool {
        self.channel(id)
            .is_some_and(|o| mute_active(o.muted, o.mute_config.as_ref(), now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_expires_mutes() {
        let s: UserGuildSettings = serde_json::from_value(serde_json::json!({
            "guild_id": "5",
            "muted": true,
            "mute_config": { "end_time": "2026-01-01T00:00:00+00:00", "selected_time_window": 3600 },
            "message_notifications": 1,
            "channel_overrides": [
                { "channel_id": "7", "muted": true, "mute_config": null, "message_notifications": 3 }
            ]
        }))
        .unwrap();
        let before = OffsetDateTime::parse("2025-12-31T23:00:00Z", &Rfc3339).unwrap();
        let after = OffsetDateTime::parse("2026-01-01T01:00:00Z", &Rfc3339).unwrap();
        assert!(s.is_muted(before));
        assert!(!s.is_muted(after));
        assert!(s.is_channel_muted(Snowflake::new(7).unwrap(), after));
        assert_eq!(s.message_notifications, NotifyLevel::Mentions);
        assert_eq!(
            s.channel(Snowflake::new(7).unwrap())
                .unwrap()
                .message_notifications,
            NotifyLevel::Inherit
        );
    }
}
