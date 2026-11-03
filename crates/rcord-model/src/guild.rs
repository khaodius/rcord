use serde::{Deserialize, Serialize};

use crate::{Channel, Emoji, Permissions, Snowflake, Sticker, User};

/// A server. READY sends some fields under `properties` when the client asks for the
/// newer client state; [`Guild::from_gateway`] accepts both shapes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Guild {
    pub id: Snowflake,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub splash: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub owner_id: Option<Snowflake>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub premium_tier: u8,
    #[serde(default)]
    pub premium_subscription_count: Option<u32>,
    #[serde(default)]
    pub member_count: Option<u32>,
    #[serde(default)]
    pub roles: Vec<Role>,
    #[serde(default)]
    pub emojis: Vec<Emoji>,
    #[serde(default)]
    pub stickers: Vec<Sticker>,
    #[serde(default)]
    pub channels: Vec<Channel>,
    #[serde(default)]
    pub threads: Vec<Channel>,
    #[serde(default)]
    pub unavailable: bool,
}

impl Guild {
    /// Parses a guild from READY or GUILD_CREATE, lifting fields out of `properties`.
    pub fn from_gateway(mut value: serde_json::Value) -> serde_json::Result<Self> {
        if let Some(obj) = value.as_object_mut()
            && let Some(serde_json::Value::Object(props)) = obj.remove("properties")
        {
            for (k, v) in props {
                obj.entry(k).or_insert(v);
            }
        }
        serde_json::from_value(value)
    }

    /// Initials shown in place of a missing icon, like "TC" for "The Coven".
    pub fn acronym(&self) -> String {
        self.name
            .split_whitespace()
            .filter_map(|w| w.chars().next())
            .take(4)
            .collect()
    }

    pub fn everyone_role(&self) -> Option<&Role> {
        self.roles.iter().find(|r| r.id == self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    pub id: Snowflake,
    pub name: String,
    #[serde(default)]
    pub color: u32,
    #[serde(default)]
    pub colors: Option<RoleColors>,
    #[serde(default)]
    pub hoist: bool,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub unicode_emoji: Option<String>,
    #[serde(default)]
    pub position: i32,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub managed: bool,
    #[serde(default)]
    pub mentionable: bool,
}

/// Gradient role colors. `primary_color` matches `color`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleColors {
    pub primary_color: u32,
    #[serde(default)]
    pub secondary_color: Option<u32>,
    #[serde(default)]
    pub tertiary_color: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// Missing in MESSAGE_CREATE, where the author is sent separately.
    #[serde(default)]
    pub user: Option<User>,
    /// Set instead of `user` in READY's deduplicated `merged_members`.
    #[serde(default)]
    pub user_id: Option<Snowflake>,
    #[serde(default)]
    pub nick: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub roles: Vec<Snowflake>,
    #[serde(default)]
    pub joined_at: Option<String>,
    #[serde(default)]
    pub premium_since: Option<String>,
    #[serde(default)]
    pub communication_disabled_until: Option<String>,
    #[serde(default)]
    pub pending: bool,
}

impl Member {
    pub fn id(&self) -> Option<Snowflake> {
        self.user.as_ref().map(|u| u.id).or(self.user_id)
    }

    /// The highest role with a non-zero color, which is the color the name is drawn in.
    pub fn color(&self, guild: &Guild) -> Option<u32> {
        guild
            .roles
            .iter()
            .filter(|r| r.color != 0 && self.roles.contains(&r.id))
            .max_by_key(|r| (r.position, std::cmp::Reverse(r.id)))
            .map(|r| r.color)
    }

    /// The hoisted role this member is grouped under in the member list.
    pub fn hoisted_role<'g>(&self, guild: &'g Guild) -> Option<&'g Role> {
        guild
            .roles
            .iter()
            .filter(|r| r.hoist && self.roles.contains(&r.id))
            .max_by_key(|r| (r.position, std::cmp::Reverse(r.id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifts_properties() {
        let g = Guild::from_gateway(serde_json::json!({
            "id": "10",
            "properties": {"name": "Rust", "icon": "abc", "owner_id": "5"},
            "roles": [{"id": "10", "name": "@everyone", "permissions": "1024", "position": 0}],
            "channels": []
        }))
        .unwrap();
        assert_eq!(g.name, "Rust");
        assert_eq!(g.icon.as_deref(), Some("abc"));
        assert_eq!(
            g.everyone_role().unwrap().permissions,
            Permissions::VIEW_CHANNEL
        );
        assert_eq!(g.acronym(), "R");
    }

    #[test]
    fn member_color_uses_highest_colored_role() {
        let g = Guild::from_gateway(serde_json::json!({
            "id": "1",
            "name": "g",
            "roles": [
                {"id": "2", "name": "low", "color": 255, "position": 1},
                {"id": "3", "name": "high", "color": 0, "position": 5, "hoist": true},
                {"id": "4", "name": "mid", "color": 65280, "position": 3}
            ]
        }))
        .unwrap();
        let m: Member =
            serde_json::from_value(serde_json::json!({"roles": ["2", "3", "4"]})).unwrap();
        assert_eq!(m.color(&g), Some(65280));
        assert_eq!(m.hoisted_role(&g).unwrap().name, "high");
    }
}
