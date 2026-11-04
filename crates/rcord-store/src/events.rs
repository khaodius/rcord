//! Dispatch events parsed into types. Parsing runs on the network thread, so the UI thread
//! only applies already-built values.

use rcord_model::{
    Channel, Emoji, Guild, Member, Message, Presence, Relationship, Role, Snowflake, Sticker, User,
    UserGuildSettings, VoiceState,
};
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Debug)]
pub enum Update {
    Ready(Box<Ready>),
    ReadySupplemental(Box<ReadySupplemental>),
    Resumed,
    MessageCreate(Box<Message>),
    /// Partial message; only fields present change.
    MessageUpdate(Box<serde_json::Value>),
    MessageDelete {
        id: Snowflake,
        channel_id: Snowflake,
    },
    MessageDeleteBulk {
        ids: Vec<Snowflake>,
        channel_id: Snowflake,
    },
    ReactionAdd(ReactionEvent),
    ReactionRemove(ReactionEvent),
    ReactionRemoveAll {
        channel_id: Snowflake,
        message_id: Snowflake,
    },
    MessageAck {
        channel_id: Snowflake,
        message_id: Snowflake,
        mention_count: u32,
    },
    GuildCreate(Box<Guild>),
    GuildUpdate(Box<serde_json::Value>),
    GuildDelete {
        id: Snowflake,
        unavailable: bool,
    },
    GuildRoleUpsert {
        guild_id: Snowflake,
        role: Role,
    },
    GuildRoleDelete {
        guild_id: Snowflake,
        role_id: Snowflake,
    },
    GuildEmojisUpdate {
        guild_id: Snowflake,
        emojis: Vec<Emoji>,
    },
    GuildStickersUpdate {
        guild_id: Snowflake,
        stickers: Vec<Sticker>,
    },
    GuildMemberUpdate {
        guild_id: Snowflake,
        member: Member,
    },
    GuildMemberListUpdate(Box<MemberListUpdate>),
    ChannelCreate(Box<Channel>),
    ChannelUpdate(Box<Channel>),
    ChannelDelete {
        id: Snowflake,
        guild_id: Option<Snowflake>,
    },
    ThreadListSync {
        guild_id: Snowflake,
        threads: Vec<Channel>,
    },
    PresenceUpdate(Box<Presence>),
    GuildSettings(Box<UserGuildSettings>),
    TypingStart(TypingStart),
    VoiceStateUpdate(Box<VoiceState>),
    /// Members asked for by user ID, such as voice participants not cached yet.
    GuildMembersChunk(Box<MembersChunk>),
    /// Where to connect for voice. Used by the call, not cached.
    VoiceServerUpdate(Box<VoiceServerUpdate>),
    /// A Go Live stream this client started or asked to watch now exists.
    StreamCreate(Box<StreamInfo>),
    /// A stream's viewers or paused state changed.
    StreamUpdate(Box<StreamInfo>),
    /// Where to connect for a stream's media.
    StreamServerUpdate(Box<StreamServerUpdate>),
    /// A stream ended, or this client stopped watching it.
    StreamDelete(Box<StreamDelete>),
    UserUpdate(Box<User>),
    RelationshipAdd(Box<Relationship>),
    RelationshipRemove {
        id: Snowflake,
    },
    /// An event rcord does not use yet.
    Ignored,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VoiceServerUpdate {
    pub token: String,
    /// Set for guild calls.
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    /// Set instead of the guild for direct calls.
    #[serde(default)]
    pub channel_id: Option<Snowflake>,
    /// `None` while Discord moves the call to another server.
    #[serde(default)]
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamInfo {
    pub stream_key: String,
    /// The stream media server's ID; set on STREAM_CREATE.
    #[serde(default, deserialize_with = "lenient_id")]
    pub rtc_server_id: Option<u64>,
    #[serde(default)]
    pub viewer_ids: Vec<Snowflake>,
    #[serde(default)]
    pub paused: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamServerUpdate {
    pub stream_key: String,
    pub token: String,
    #[serde(default)]
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamDelete {
    pub stream_key: String,
    /// Why it ended, such as `"user_requested"` or `"stream_full"`.
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub unavailable: bool,
}

fn lenient_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match v {
        Some(serde_json::Value::String(s)) => s.parse().ok(),
        Some(serde_json::Value::Number(n)) => n.as_u64(),
        _ => None,
    })
}

#[derive(Debug, Deserialize)]
pub struct Ready {
    pub user: User,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub users: Vec<User>,
    #[serde(default)]
    pub guilds: Vec<Box<RawValue>>,
    #[serde(skip)]
    pub parsed_guilds: Vec<Guild>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub private_channels: Vec<Channel>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub relationships: Vec<Relationship>,
    #[serde(default)]
    pub read_state: ReadStates,
    /// One list per guild, in the order of `guilds`.
    #[serde(default, deserialize_with = "lenient_nested")]
    pub merged_members: Vec<Vec<Member>>,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub user_guild_settings: GuildSettingsList,
}

/// READY's `user_guild_settings`: an object with `entries` when versioned settings are
/// asked for, a plain list otherwise. Kept raw and read item by item, so a shape rcord does
/// not expect costs the settings and never the whole READY.
#[derive(Debug, Default, Deserialize)]
#[serde(transparent)]
pub struct GuildSettingsList(Option<Box<RawValue>>);

impl GuildSettingsList {
    pub fn into_vec(self) -> Vec<UserGuildSettings> {
        #[derive(Deserialize)]
        struct Versioned {
            #[serde(default)]
            entries: Vec<Box<RawValue>>,
        }
        let Some(raw) = self.0 else {
            return Vec::new();
        };
        let items: Vec<Box<RawValue>> = serde_json::from_str::<Versioned>(raw.get())
            .map(|v| v.entries)
            .or_else(|_| serde_json::from_str(raw.get()))
            .unwrap_or_default();
        items
            .into_iter()
            .filter_map(|item| {
                serde_json::from_str(item.get())
                    .map_err(|e| tracing::warn!("skipping a server's notification settings: {e}"))
                    .ok()
            })
            .collect()
    }
}

/// Reads a list item by item, skipping items that do not parse, so one object in a shape
/// rcord does not expect costs that object and not the whole connection.
fn lenient_vec<'de, D, T>(de: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let raw: Option<Vec<Box<RawValue>>> = Deserialize::deserialize(de)?;
    Ok(raw
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| {
            serde_json::from_str(item.get())
                .map_err(|e| tracing::warn!("skipping an item in READY: {e}"))
                .ok()
        })
        .collect())
}

fn lenient_nested<'de, D, T>(de: D) -> Result<Vec<Vec<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let raw: Option<Vec<Vec<Box<RawValue>>>> = Deserialize::deserialize(de)?;
    Ok(raw
        .unwrap_or_default()
        .into_iter()
        .map(|list| {
            list.into_iter()
                .filter_map(|item| {
                    serde_json::from_str(item.get())
                        .map_err(|e| tracing::warn!("skipping a member in READY: {e}"))
                        .ok()
                })
                .collect()
        })
        .collect())
}

/// With versioned read states this is `{entries: [...]}`; without, a bare list.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ReadStates {
    Versioned { entries: Vec<ReadStateEntry> },
    List(Vec<ReadStateEntry>),
}

impl Default for ReadStates {
    fn default() -> Self {
        Self::List(Vec::new())
    }
}

impl ReadStates {
    pub fn entries(&self) -> &[ReadStateEntry] {
        match self {
            Self::Versioned { entries } | Self::List(entries) => entries,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReadStateEntry {
    pub id: Snowflake,
    /// `0` means never read, which is not a valid snowflake.
    #[serde(default, deserialize_with = "loose_snowflake")]
    pub last_message_id: Option<Snowflake>,
    #[serde(default)]
    pub mention_count: u32,
    /// 0 for channels; other types track things like notification centers.
    #[serde(default)]
    pub read_state_type: u8,
}

fn loose_snowflake<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Snowflake>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

#[derive(Debug, Default, Deserialize)]
pub struct ReadySupplemental {
    #[serde(default)]
    pub merged_presences: MergedPresences,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub guilds: Vec<SupplementalGuild>,
    /// Members for the users in `guilds`' voice states, one list per guild in that order.
    #[serde(default, deserialize_with = "lenient_nested")]
    pub merged_members: Vec<Vec<Member>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MergedPresences {
    #[serde(default)]
    pub friends: Vec<Presence>,
    #[serde(default)]
    pub guilds: Vec<Vec<Presence>>,
}

#[derive(Debug, Deserialize)]
pub struct SupplementalGuild {
    pub id: Snowflake,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub voice_states: Vec<VoiceState>,
}

#[derive(Debug, Deserialize)]
pub struct MembersChunk {
    pub guild_id: Snowflake,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReactionEvent {
    pub user_id: Snowflake,
    pub channel_id: Snowflake,
    pub message_id: Snowflake,
    pub emoji: Emoji,
    #[serde(default)]
    pub burst: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TypingStart {
    pub channel_id: Snowflake,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    pub user_id: Snowflake,
    pub timestamp: u64,
    #[serde(default)]
    pub member: Option<Member>,
}

#[derive(Debug, Deserialize)]
pub struct MemberListUpdate {
    pub guild_id: Snowflake,
    pub id: String,
    #[serde(default)]
    pub member_count: u32,
    #[serde(default)]
    pub online_count: u32,
    #[serde(default)]
    pub groups: Vec<MemberListGroup>,
    #[serde(default)]
    pub ops: Vec<MemberListOp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MemberListGroup {
    /// A role ID, `online` or `offline`.
    pub id: String,
    #[serde(default)]
    pub count: u32,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "UPPERCASE")]
pub enum MemberListOp {
    Sync {
        range: [u32; 2],
        items: Vec<MemberListItem>,
    },
    Insert {
        index: u32,
        item: MemberListItem,
    },
    Update {
        index: u32,
        item: MemberListItem,
    },
    Delete {
        index: u32,
    },
    Invalidate {
        range: [u32; 2],
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemberListItem {
    Group(MemberListGroup),
    Member(Box<ListMember>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListMember {
    #[serde(flatten)]
    pub member: Member,
    #[serde(default)]
    pub presence: Option<Presence>,
}

#[derive(Deserialize)]
struct Ids {
    id: Snowflake,
    channel_id: Snowflake,
}

#[derive(Deserialize)]
struct BulkIds {
    ids: Vec<Snowflake>,
    channel_id: Snowflake,
}

#[derive(Deserialize)]
struct MessageIds {
    channel_id: Snowflake,
    message_id: Snowflake,
}

#[derive(Deserialize)]
struct Ack {
    channel_id: Snowflake,
    message_id: Snowflake,
    #[serde(default)]
    mention_count: u32,
}

#[derive(Deserialize)]
struct GuildGone {
    id: Snowflake,
    #[serde(default)]
    unavailable: bool,
}

#[derive(Deserialize)]
struct RoleEvent {
    guild_id: Snowflake,
    role: Role,
}

#[derive(Deserialize)]
struct RoleDelete {
    guild_id: Snowflake,
    role_id: Snowflake,
}

#[derive(Deserialize)]
struct Emojis {
    guild_id: Snowflake,
    emojis: Vec<Emoji>,
}

#[derive(Deserialize)]
struct Stickers {
    guild_id: Snowflake,
    stickers: Vec<Sticker>,
}

#[derive(Deserialize)]
struct MemberUpdate {
    guild_id: Snowflake,
    #[serde(flatten)]
    member: Member,
}

#[derive(Deserialize)]
struct ChannelGone {
    id: Snowflake,
    #[serde(default)]
    guild_id: Option<Snowflake>,
}

#[derive(Deserialize)]
struct ThreadSync {
    guild_id: Snowflake,
    #[serde(default)]
    threads: Vec<Channel>,
}

#[derive(Deserialize)]
struct RelationshipGone {
    id: Snowflake,
}

/// Parses a dispatch payload. Unknown events become [`Update::Ignored`].
pub fn parse(name: &str, data: &RawValue) -> serde_json::Result<Update> {
    use serde_json::from_str as p;
    let d = data.get();
    Ok(match name {
        "READY" => {
            let mut ready: Ready = p(d)?;
            ready.parsed_guilds = ready
                .guilds
                .drain(..)
                .filter_map(|g| {
                    serde_json::from_str::<serde_json::Value>(g.get())
                        .and_then(Guild::from_gateway)
                        .map_err(|e| tracing::warn!("skipping guild in READY: {e}"))
                        .ok()
                })
                .collect();
            Update::Ready(Box::new(ready))
        }
        "READY_SUPPLEMENTAL" => Update::ReadySupplemental(Box::new(p(d)?)),
        "RESUMED" => Update::Resumed,
        "MESSAGE_CREATE" => Update::MessageCreate(Box::new(p(d)?)),
        "MESSAGE_UPDATE" => Update::MessageUpdate(Box::new(p(d)?)),
        "MESSAGE_DELETE" => {
            let ids: Ids = p(d)?;
            Update::MessageDelete {
                id: ids.id,
                channel_id: ids.channel_id,
            }
        }
        "MESSAGE_DELETE_BULK" => {
            let ids: BulkIds = p(d)?;
            Update::MessageDeleteBulk {
                ids: ids.ids,
                channel_id: ids.channel_id,
            }
        }
        "MESSAGE_REACTION_ADD" => Update::ReactionAdd(p(d)?),
        "MESSAGE_REACTION_REMOVE" => Update::ReactionRemove(p(d)?),
        "MESSAGE_REACTION_REMOVE_ALL" => {
            let ids: MessageIds = p(d)?;
            Update::ReactionRemoveAll {
                channel_id: ids.channel_id,
                message_id: ids.message_id,
            }
        }
        "MESSAGE_ACK" => {
            let ack: Ack = p(d)?;
            Update::MessageAck {
                channel_id: ack.channel_id,
                message_id: ack.message_id,
                mention_count: ack.mention_count,
            }
        }
        "GUILD_CREATE" => Update::GuildCreate(Box::new(Guild::from_gateway(p(d)?)?)),
        "GUILD_UPDATE" => Update::GuildUpdate(Box::new(p(d)?)),
        "GUILD_DELETE" => {
            let g: GuildGone = p(d)?;
            Update::GuildDelete {
                id: g.id,
                unavailable: g.unavailable,
            }
        }
        "GUILD_ROLE_CREATE" | "GUILD_ROLE_UPDATE" => {
            let r: RoleEvent = p(d)?;
            Update::GuildRoleUpsert {
                guild_id: r.guild_id,
                role: r.role,
            }
        }
        "GUILD_ROLE_DELETE" => {
            let r: RoleDelete = p(d)?;
            Update::GuildRoleDelete {
                guild_id: r.guild_id,
                role_id: r.role_id,
            }
        }
        "GUILD_EMOJIS_UPDATE" => {
            let e: Emojis = p(d)?;
            Update::GuildEmojisUpdate {
                guild_id: e.guild_id,
                emojis: e.emojis,
            }
        }
        "GUILD_STICKERS_UPDATE" => {
            let s: Stickers = p(d)?;
            Update::GuildStickersUpdate {
                guild_id: s.guild_id,
                stickers: s.stickers,
            }
        }
        "GUILD_MEMBER_UPDATE" => {
            let m: MemberUpdate = p(d)?;
            Update::GuildMemberUpdate {
                guild_id: m.guild_id,
                member: m.member,
            }
        }
        "GUILD_MEMBER_LIST_UPDATE" => Update::GuildMemberListUpdate(Box::new(p(d)?)),
        "CHANNEL_CREATE" | "THREAD_CREATE" => Update::ChannelCreate(Box::new(p(d)?)),
        "CHANNEL_UPDATE" | "THREAD_UPDATE" => Update::ChannelUpdate(Box::new(p(d)?)),
        "CHANNEL_DELETE" | "THREAD_DELETE" => {
            let c: ChannelGone = p(d)?;
            Update::ChannelDelete {
                id: c.id,
                guild_id: c.guild_id,
            }
        }
        "THREAD_LIST_SYNC" => {
            let t: ThreadSync = p(d)?;
            Update::ThreadListSync {
                guild_id: t.guild_id,
                threads: t.threads,
            }
        }
        "PRESENCE_UPDATE" => Update::PresenceUpdate(Box::new(p(d)?)),
        "USER_GUILD_SETTINGS_UPDATE" => Update::GuildSettings(Box::new(p(d)?)),
        "TYPING_START" => Update::TypingStart(p(d)?),
        "VOICE_STATE_UPDATE" => Update::VoiceStateUpdate(Box::new(p(d)?)),
        "GUILD_MEMBERS_CHUNK" => Update::GuildMembersChunk(Box::new(p(d)?)),
        "VOICE_SERVER_UPDATE" => Update::VoiceServerUpdate(Box::new(p(d)?)),
        "STREAM_CREATE" => Update::StreamCreate(Box::new(p(d)?)),
        "STREAM_UPDATE" => Update::StreamUpdate(Box::new(p(d)?)),
        "STREAM_SERVER_UPDATE" => Update::StreamServerUpdate(Box::new(p(d)?)),
        "STREAM_DELETE" => Update::StreamDelete(Box::new(p(d)?)),
        "USER_UPDATE" => Update::UserUpdate(Box::new(p(d)?)),
        "RELATIONSHIP_ADD" => Update::RelationshipAdd(Box::new(p(d)?)),
        "RELATIONSHIP_REMOVE" => {
            let r: RelationshipGone = p(d)?;
            Update::RelationshipRemove { id: r.id }
        }
        _ => Update::Ignored,
    })
}
