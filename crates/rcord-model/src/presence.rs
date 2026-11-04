use serde::{Deserialize, Serialize};

use crate::{Emoji, Member, Snowflake};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Online,
    Idle,
    Dnd,
    Invisible,
    #[default]
    #[serde(other)]
    Offline,
}

impl Status {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::Idle => "Idle",
            Self::Dnd => "Do Not Disturb",
            Self::Invisible => "Invisible",
            Self::Offline => "Offline",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "u8", into = "u8")]
pub enum ActivityKind {
    Playing,
    Streaming,
    Listening,
    Watching,
    Custom,
    Competing,
    Unknown(u8),
}

impl From<u8> for ActivityKind {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::Playing,
            1 => Self::Streaming,
            2 => Self::Listening,
            3 => Self::Watching,
            4 => Self::Custom,
            5 => Self::Competing,
            other => Self::Unknown(other),
        }
    }
}

impl From<ActivityKind> for u8 {
    fn from(v: ActivityKind) -> Self {
        match v {
            ActivityKind::Playing => 0,
            ActivityKind::Streaming => 1,
            ActivityKind::Listening => 2,
            ActivityKind::Watching => 3,
            ActivityKind::Custom => 4,
            ActivityKind::Competing => 5,
            ActivityKind::Unknown(other) => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ActivityKind,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub details: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub emoji: Option<Emoji>,
    #[serde(default)]
    pub application_id: Option<Snowflake>,
    #[serde(default)]
    pub timestamps: Option<ActivityTimestamps>,
    #[serde(default)]
    pub assets: Option<ActivityAssets>,
}

impl Activity {
    /// The one line shown under a name in the member list.
    pub fn summary(&self) -> String {
        match self.kind {
            ActivityKind::Custom => self.state.clone().unwrap_or_default(),
            ActivityKind::Playing => format!("Playing {}", self.name),
            ActivityKind::Streaming => format!(
                "Streaming {}",
                self.details.as_deref().unwrap_or(&self.name)
            ),
            ActivityKind::Listening => format!("Listening to {}", self.name),
            ActivityKind::Watching => format!("Watching {}", self.name),
            ActivityKind::Competing => format!("Competing in {}", self.name),
            ActivityKind::Unknown(_) => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityTimestamps {
    #[serde(default)]
    pub start: Option<u64>,
    #[serde(default)]
    pub end: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityAssets {
    #[serde(default)]
    pub large_image: Option<String>,
    #[serde(default)]
    pub large_text: Option<String>,
    #[serde(default)]
    pub small_image: Option<String>,
    #[serde(default)]
    pub small_text: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientStatus {
    #[serde(default)]
    pub desktop: Option<Status>,
    #[serde(default)]
    pub mobile: Option<Status>,
    #[serde(default)]
    pub web: Option<Status>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceUser {
    pub id: Snowflake,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    /// READY_SUPPLEMENTAL sends `user_id` instead.
    #[serde(default)]
    pub user: Option<PresenceUser>,
    #[serde(default)]
    pub user_id: Option<Snowflake>,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    #[serde(default)]
    pub status: Status,
    #[serde(default)]
    pub activities: Vec<Activity>,
    #[serde(default)]
    pub client_status: ClientStatus,
}

impl Presence {
    pub fn user_id(&self) -> Option<Snowflake> {
        self.user.as_ref().map(|u| u.id).or(self.user_id)
    }

    pub fn custom_status(&self) -> Option<&Activity> {
        self.activities
            .iter()
            .find(|a| a.kind == ActivityKind::Custom)
    }

    /// The first activity that is not a custom status.
    pub fn primary_activity(&self) -> Option<&Activity> {
        self.activities
            .iter()
            .find(|a| a.kind != ActivityKind::Custom)
    }
}

fn lenient_member<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Member>, D::Error> {
    let raw: Option<serde_json::Value> = Deserialize::deserialize(d)?;
    Ok(raw.and_then(|v| serde_json::from_value(v).ok()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceState {
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    #[serde(default)]
    pub channel_id: Option<Snowflake>,
    pub user_id: Snowflake,
    #[serde(default)]
    pub session_id: String,
    /// Dropped rather than failing the whole voice state when it arrives in a shape rcord
    /// does not read, so a participant never goes missing over a profile field.
    #[serde(default, deserialize_with = "lenient_member")]
    pub member: Option<Member>,
    #[serde(default)]
    pub deaf: bool,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub self_deaf: bool,
    #[serde(default)]
    pub self_mute: bool,
    #[serde(default)]
    pub self_stream: bool,
    #[serde(default)]
    pub self_video: bool,
    #[serde(default)]
    pub suppress: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_status_is_offline() {
        let s: Status = serde_json::from_str("\"something\"").unwrap();
        assert_eq!(s, Status::Offline);
    }

    #[test]
    fn activity_summary() {
        let p: Presence = serde_json::from_value(serde_json::json!({
            "user": {"id": "1"},
            "status": "dnd",
            "activities": [
                {"name": "Custom Status", "type": 4, "state": "shipping"},
                {"name": "Spotify", "type": 2}
            ]
        }))
        .unwrap();
        assert_eq!(p.status, Status::Dnd);
        assert_eq!(p.custom_status().unwrap().summary(), "shipping");
        assert_eq!(
            p.primary_activity().unwrap().summary(),
            "Listening to Spotify"
        );
    }
}
