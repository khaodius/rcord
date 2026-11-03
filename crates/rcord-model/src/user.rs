use serde::{Deserialize, Serialize};

use crate::Snowflake;

/// A Discord account as it appears in messages, member lists and READY.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: Snowflake,
    pub username: String,
    /// "0" for accounts migrated to unique usernames.
    #[serde(default)]
    pub discriminator: Option<String>,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub avatar_decoration_data: Option<AvatarDecoration>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub accent_color: Option<u32>,
    #[serde(default)]
    pub public_flags: UserFlags,
    #[serde(default)]
    pub premium_type: Option<PremiumType>,
    #[serde(default)]
    pub bot: bool,
    #[serde(default)]
    pub system: bool,
    /// The server tag shown next to the name.
    #[serde(default, alias = "clan")]
    pub primary_guild: Option<PrimaryGuild>,
    /// Only present on the signed-in user.
    #[serde(default)]
    pub email: Option<String>,
}

impl User {
    /// The name shown in the UI: global display name, then username.
    pub fn display_name(&self) -> &str {
        self.global_name
            .as_deref()
            .filter(|n| !n.is_empty())
            .unwrap_or(&self.username)
    }

    /// `username` for migrated accounts, `username#1234` for legacy ones and bots.
    pub fn tag(&self) -> String {
        match self.discriminator.as_deref() {
            Some(d) if d != "0" && !d.is_empty() => format!("{}#{d}", self.username),
            _ => self.username.clone(),
        }
    }

    /// Index of the default avatar Discord assigns when none is set.
    pub fn default_avatar_index(&self) -> u64 {
        match self
            .discriminator
            .as_deref()
            .and_then(|d| d.parse::<u64>().ok())
        {
            Some(d) if d != 0 => d % 5,
            _ => (self.id.get() >> 22) % 6,
        }
    }

    /// Badges that can be derived from public flags alone. The profile endpoint returns
    /// the full list, including Nitro and boost badges.
    pub fn flag_badges(&self) -> Vec<FlagBadge> {
        FlagBadge::ALL
            .iter()
            .copied()
            .filter(|b| self.public_flags.contains(b.flag()))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarDecoration {
    pub asset: String,
    #[serde(default)]
    pub sku_id: Option<Snowflake>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrimaryGuild {
    #[serde(default)]
    pub identity_guild_id: Option<Snowflake>,
    #[serde(default)]
    pub identity_enabled: Option<bool>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub badge: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "u8", into = "u8")]
pub enum PremiumType {
    None,
    NitroClassic,
    Nitro,
    NitroBasic,
    Unknown(u8),
}

impl From<u8> for PremiumType {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::None,
            1 => Self::NitroClassic,
            2 => Self::Nitro,
            3 => Self::NitroBasic,
            other => Self::Unknown(other),
        }
    }
}

impl From<PremiumType> for u8 {
    fn from(v: PremiumType) -> Self {
        match v {
            PremiumType::None => 0,
            PremiumType::NitroClassic => 1,
            PremiumType::Nitro => 2,
            PremiumType::NitroBasic => 3,
            PremiumType::Unknown(other) => other,
        }
    }
}

bitflags::bitflags! {
    /// Public user flags, as documented for the `public_flags` field.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct UserFlags: u64 {
        const STAFF = 1 << 0;
        const PARTNER = 1 << 1;
        const HYPESQUAD = 1 << 2;
        const BUG_HUNTER_LEVEL_1 = 1 << 3;
        const HYPESQUAD_BRAVERY = 1 << 6;
        const HYPESQUAD_BRILLIANCE = 1 << 7;
        const HYPESQUAD_BALANCE = 1 << 8;
        const PREMIUM_EARLY_SUPPORTER = 1 << 9;
        const TEAM_PSEUDO_USER = 1 << 10;
        const BUG_HUNTER_LEVEL_2 = 1 << 14;
        const VERIFIED_BOT = 1 << 16;
        const VERIFIED_DEVELOPER = 1 << 17;
        const CERTIFIED_MODERATOR = 1 << 18;
        const BOT_HTTP_INTERACTIONS = 1 << 19;
        const ACTIVE_DEVELOPER = 1 << 22;
        const _ = !0;
    }
}

impl Serialize for UserFlags {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(self.bits())
    }
}

impl<'de> Deserialize<'de> for UserFlags {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::from_bits_retain(u64::deserialize(d)?))
    }
}

/// A badge derived from public flags, with the CDN icon hash Discord's own client uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlagBadge {
    Staff,
    Partner,
    CertifiedModerator,
    Hypesquad,
    HypesquadBravery,
    HypesquadBrilliance,
    HypesquadBalance,
    BugHunter,
    BugHunterGold,
    ActiveDeveloper,
    VerifiedDeveloper,
    EarlySupporter,
}

impl FlagBadge {
    pub const ALL: [Self; 12] = [
        Self::Staff,
        Self::Partner,
        Self::CertifiedModerator,
        Self::Hypesquad,
        Self::HypesquadBravery,
        Self::HypesquadBrilliance,
        Self::HypesquadBalance,
        Self::BugHunter,
        Self::BugHunterGold,
        Self::ActiveDeveloper,
        Self::VerifiedDeveloper,
        Self::EarlySupporter,
    ];

    pub const fn flag(self) -> UserFlags {
        match self {
            Self::Staff => UserFlags::STAFF,
            Self::Partner => UserFlags::PARTNER,
            Self::CertifiedModerator => UserFlags::CERTIFIED_MODERATOR,
            Self::Hypesquad => UserFlags::HYPESQUAD,
            Self::HypesquadBravery => UserFlags::HYPESQUAD_BRAVERY,
            Self::HypesquadBrilliance => UserFlags::HYPESQUAD_BRILLIANCE,
            Self::HypesquadBalance => UserFlags::HYPESQUAD_BALANCE,
            Self::BugHunter => UserFlags::BUG_HUNTER_LEVEL_1,
            Self::BugHunterGold => UserFlags::BUG_HUNTER_LEVEL_2,
            Self::ActiveDeveloper => UserFlags::ACTIVE_DEVELOPER,
            Self::VerifiedDeveloper => UserFlags::VERIFIED_DEVELOPER,
            Self::EarlySupporter => UserFlags::PREMIUM_EARLY_SUPPORTER,
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Staff => "Discord Staff",
            Self::Partner => "Partnered Server Owner",
            Self::CertifiedModerator => "Moderator Programs Alumni",
            Self::Hypesquad => "HypeSquad Events",
            Self::HypesquadBravery => "HypeSquad Bravery",
            Self::HypesquadBrilliance => "HypeSquad Brilliance",
            Self::HypesquadBalance => "HypeSquad Balance",
            Self::BugHunter => "Discord Bug Hunter",
            Self::BugHunterGold => "Discord Bug Hunter",
            Self::ActiveDeveloper => "Active Developer",
            Self::VerifiedDeveloper => "Early Verified Bot Developer",
            Self::EarlySupporter => "Early Supporter",
        }
    }

    /// Icon hash under `https://cdn.discordapp.com/badge-icons/`.
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Staff => "5e74e9b61934fc1f67c65515d1f7e60d",
            Self::Partner => "3f9748e53446a137a052f3454e2de41e",
            Self::CertifiedModerator => "fee1624003e2fee35cb398e125dc479b",
            Self::Hypesquad => "bf01d1073931f921909045f3a39fd264",
            Self::HypesquadBravery => "8a88d63823d8a71cd5e390baa45efa02",
            Self::HypesquadBrilliance => "011940fd013da3f7fb926e4a1cd2e618",
            Self::HypesquadBalance => "3aa41de486fa12454c3761e8e223442e",
            Self::BugHunter => "2717692c7dca7289b35297368a940dd0",
            Self::BugHunterGold => "848f79194d4be5ff5f81505cbd0ce1e6",
            Self::ActiveDeveloper => "6bdc42827a38498929a4920da12695d9",
            Self::VerifiedDeveloper => "6df5892e0f35b051f8b61eace34f4967",
            Self::EarlySupporter => "7060786766c9c840eb3019e725d2b358",
        }
    }
}

/// A badge as returned by `GET /users/{id}/profile`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileBadge {
    pub id: String,
    pub description: String,
    pub icon: String,
    #[serde(default)]
    pub link: Option<String>,
}

/// `GET /users/{id}/profile`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserProfile {
    pub user: ProfileUser,
    #[serde(default)]
    pub user_profile: Option<ProfileMetadata>,
    #[serde(default)]
    pub guild_member_profile: Option<ProfileMetadata>,
    #[serde(default)]
    pub badges: Vec<ProfileBadge>,
    #[serde(default)]
    pub guild_badges: Vec<ProfileBadge>,
    #[serde(default)]
    pub mutual_guilds: Vec<MutualGuild>,
    #[serde(default)]
    pub mutual_friends_count: Option<u32>,
    #[serde(default)]
    pub connected_accounts: Vec<ConnectedAccount>,
    #[serde(default)]
    pub premium_since: Option<String>,
    #[serde(default)]
    pub premium_guild_since: Option<String>,
}

/// The user object inside a profile, which adds the bio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileUser {
    #[serde(flatten)]
    pub user: User,
    #[serde(default)]
    pub bio: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMetadata {
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub pronouns: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub accent_color: Option<u32>,
    /// Primary and accent colors of a Nitro profile theme.
    #[serde(default)]
    pub theme_colors: Option<Vec<u32>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutualGuild {
    pub id: Snowflake,
    #[serde(default)]
    pub nick: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectedAccount {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "u8", into = "u8")]
pub enum RelationshipKind {
    None,
    Friend,
    Blocked,
    IncomingRequest,
    OutgoingRequest,
    Implicit,
    Unknown(u8),
}

impl From<u8> for RelationshipKind {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::None,
            1 => Self::Friend,
            2 => Self::Blocked,
            3 => Self::IncomingRequest,
            4 => Self::OutgoingRequest,
            5 => Self::Implicit,
            other => Self::Unknown(other),
        }
    }
}

impl From<RelationshipKind> for u8 {
    fn from(v: RelationshipKind) -> Self {
        match v {
            RelationshipKind::None => 0,
            RelationshipKind::Friend => 1,
            RelationshipKind::Blocked => 2,
            RelationshipKind::IncomingRequest => 3,
            RelationshipKind::OutgoingRequest => 4,
            RelationshipKind::Implicit => 5,
            RelationshipKind::Unknown(other) => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relationship {
    pub id: Snowflake,
    #[serde(rename = "type")]
    pub kind: RelationshipKind,
    #[serde(default)]
    pub nickname: Option<String>,
    /// Present unless READY deduplicates users into its `users` list.
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default)]
    pub user_id: Option<Snowflake>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(json: &str) -> User {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn display_name_prefers_global_name() {
        let u = user(r#"{"id":"1","username":"neo","global_name":"Neo","discriminator":"0"}"#);
        assert_eq!(u.display_name(), "Neo");
        assert_eq!(u.tag(), "neo");
        let legacy = user(r#"{"id":"1","username":"bot","discriminator":"0420","bot":true}"#);
        assert_eq!(legacy.display_name(), "bot");
        assert_eq!(legacy.tag(), "bot#0420");
        assert_eq!(legacy.default_avatar_index(), 420 % 5);
    }

    #[test]
    fn badges_from_flags() {
        let u = user(r#"{"id":"1","username":"a","public_flags":4194368}"#);
        assert_eq!(
            u.flag_badges(),
            vec![FlagBadge::HypesquadBravery, FlagBadge::ActiveDeveloper]
        );
    }

    #[test]
    fn unknown_flags_survive_round_trip() {
        let u = user(r#"{"id":"1","username":"a","public_flags":9007199254740992}"#);
        assert_eq!(u.public_flags.bits(), 1 << 53);
    }

    #[test]
    fn parses_profile() {
        let p: UserProfile = serde_json::from_str(
            r#"{"user":{"id":"2","username":"b","bio":"hi"},
                "user_profile":{"bio":"hi","pronouns":"they/them","theme_colors":[1,2]},
                "badges":[{"id":"premium","description":"Subscriber since Jan 1, 2020","icon":"2ba85e8026a8614b640c2837bcdfe21b","link":"https://discord.com/settings/premium"}],
                "mutual_guilds":[{"id":"3","nick":null}]}"#,
        )
        .unwrap();
        assert_eq!(p.user.bio.as_deref(), Some("hi"));
        assert_eq!(p.badges[0].id, "premium");
        assert_eq!(p.mutual_guilds.len(), 1);
    }
}
