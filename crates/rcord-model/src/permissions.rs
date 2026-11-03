use serde::{Deserialize, Serialize};

use crate::{Channel, Guild, Snowflake};

bitflags::bitflags! {
    /// Permission bits, sent as a decimal string.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Permissions: u64 {
        const CREATE_INSTANT_INVITE = 1 << 0;
        const KICK_MEMBERS = 1 << 1;
        const BAN_MEMBERS = 1 << 2;
        const ADMINISTRATOR = 1 << 3;
        const MANAGE_CHANNELS = 1 << 4;
        const MANAGE_GUILD = 1 << 5;
        const ADD_REACTIONS = 1 << 6;
        const VIEW_AUDIT_LOG = 1 << 7;
        const PRIORITY_SPEAKER = 1 << 8;
        const STREAM = 1 << 9;
        const VIEW_CHANNEL = 1 << 10;
        const SEND_MESSAGES = 1 << 11;
        const SEND_TTS_MESSAGES = 1 << 12;
        const MANAGE_MESSAGES = 1 << 13;
        const EMBED_LINKS = 1 << 14;
        const ATTACH_FILES = 1 << 15;
        const READ_MESSAGE_HISTORY = 1 << 16;
        const MENTION_EVERYONE = 1 << 17;
        const USE_EXTERNAL_EMOJIS = 1 << 18;
        const CONNECT = 1 << 20;
        const SPEAK = 1 << 21;
        const MUTE_MEMBERS = 1 << 22;
        const DEAFEN_MEMBERS = 1 << 23;
        const MOVE_MEMBERS = 1 << 24;
        const USE_VAD = 1 << 25;
        const CHANGE_NICKNAME = 1 << 26;
        const MANAGE_NICKNAMES = 1 << 27;
        const MANAGE_ROLES = 1 << 28;
        const MANAGE_WEBHOOKS = 1 << 29;
        const MANAGE_GUILD_EXPRESSIONS = 1 << 30;
        const USE_APPLICATION_COMMANDS = 1 << 31;
        const REQUEST_TO_SPEAK = 1 << 32;
        const MANAGE_EVENTS = 1 << 33;
        const MANAGE_THREADS = 1 << 34;
        const CREATE_PUBLIC_THREADS = 1 << 35;
        const CREATE_PRIVATE_THREADS = 1 << 36;
        const USE_EXTERNAL_STICKERS = 1 << 37;
        const SEND_MESSAGES_IN_THREADS = 1 << 38;
        const USE_EMBEDDED_ACTIVITIES = 1 << 39;
        const MODERATE_MEMBERS = 1 << 40;
        const USE_SOUNDBOARD = 1 << 42;
        const USE_EXTERNAL_SOUNDS = 1 << 45;
        const SEND_VOICE_MESSAGES = 1 << 46;
        const SEND_POLLS = 1 << 49;
        const _ = !0;
    }
}

impl Serialize for Permissions {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&self.bits())
    }
}

impl<'de> Deserialize<'de> for Permissions {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Str(String),
            Num(u64),
        }
        let bits = match Raw::deserialize(d)? {
            Raw::Str(s) => s.parse().map_err(serde::de::Error::custom)?,
            Raw::Num(n) => n,
        };
        Ok(Self::from_bits_retain(bits))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "u8", into = "u8")]
pub enum OverwriteKind {
    Role,
    Member,
}

impl From<u8> for OverwriteKind {
    fn from(v: u8) -> Self {
        if v == 1 { Self::Member } else { Self::Role }
    }
}

impl From<OverwriteKind> for u8 {
    fn from(v: OverwriteKind) -> Self {
        match v {
            OverwriteKind::Role => 0,
            OverwriteKind::Member => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOverwrite {
    pub id: Snowflake,
    #[serde(rename = "type")]
    pub kind: OverwriteKind,
    #[serde(default)]
    pub allow: Permissions,
    #[serde(default)]
    pub deny: Permissions,
}

/// Guild-wide permissions of a member with `roles`, before channel overwrites.
pub fn base_permissions(guild: &Guild, user_id: Snowflake, roles: &[Snowflake]) -> Permissions {
    if guild.owner_id == Some(user_id) {
        return Permissions::all();
    }
    let mut perms = guild
        .everyone_role()
        .map(|r| r.permissions)
        .unwrap_or_default();
    for role in guild.roles.iter().filter(|r| roles.contains(&r.id)) {
        perms |= role.permissions;
    }
    if perms.contains(Permissions::ADMINISTRATOR) {
        Permissions::all()
    } else {
        perms
    }
}

/// Applies a channel's overwrites in Discord's order: @everyone, roles together, then the
/// member's own overwrite.
pub fn channel_permissions(
    base: Permissions,
    guild_id: Snowflake,
    channel: &Channel,
    user_id: Snowflake,
    roles: &[Snowflake],
) -> Permissions {
    if base.contains(Permissions::ADMINISTRATOR) {
        return Permissions::all();
    }
    let mut perms = base;
    let overwrites = &channel.permission_overwrites;
    if let Some(o) = overwrites.iter().find(|o| o.id == guild_id) {
        perms &= !o.deny;
        perms |= o.allow;
    }
    let (mut allow, mut deny) = (Permissions::empty(), Permissions::empty());
    for o in overwrites
        .iter()
        .filter(|o| o.kind == OverwriteKind::Role && roles.contains(&o.id))
    {
        allow |= o.allow;
        deny |= o.deny;
    }
    perms &= !deny;
    perms |= allow;
    if let Some(o) = overwrites
        .iter()
        .find(|o| o.kind == OverwriteKind::Member && o.id == user_id)
    {
        perms &= !o.deny;
        perms |= o.allow;
    }
    if !perms.contains(Permissions::VIEW_CHANNEL) {
        return Permissions::empty();
    }
    perms
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guild() -> Guild {
        Guild::from_gateway(serde_json::json!({
            "id": "1",
            "name": "g",
            "owner_id": "99",
            "roles": [
                {"id": "1", "name": "@everyone", "permissions": "3072"},
                {"id": "2", "name": "mod", "permissions": "8192"}
            ]
        }))
        .unwrap()
    }

    fn channel(overwrites: serde_json::Value) -> Channel {
        serde_json::from_value(serde_json::json!({
            "id": "50", "type": 0, "permission_overwrites": overwrites
        }))
        .unwrap()
    }

    fn id(n: u64) -> Snowflake {
        Snowflake::new(n).unwrap()
    }

    #[test]
    fn owner_has_everything() {
        assert_eq!(base_permissions(&guild(), id(99), &[]), Permissions::all());
    }

    #[test]
    fn roles_add_up() {
        let p = base_permissions(&guild(), id(5), &[id(2)]);
        assert!(p.contains(Permissions::VIEW_CHANNEL | Permissions::MANAGE_MESSAGES));
    }

    #[test]
    fn overwrites_apply_in_order() {
        let g = guild();
        let base = base_permissions(&g, id(5), &[id(2)]);
        // @everyone loses view, the mod role gets it back, the member loses send.
        let c = channel(serde_json::json!([
            {"id": "1", "type": 0, "allow": "0", "deny": "1024"},
            {"id": "2", "type": 0, "allow": "1024", "deny": "0"},
            {"id": "5", "type": 1, "allow": "0", "deny": "2048"}
        ]));
        let p = channel_permissions(base, g.id, &c, id(5), &[id(2)]);
        assert!(p.contains(Permissions::VIEW_CHANNEL));
        assert!(!p.contains(Permissions::SEND_MESSAGES));
        let hidden = channel_permissions(base, g.id, &c, id(6), &[]);
        assert_eq!(hidden, Permissions::empty());
    }
}
