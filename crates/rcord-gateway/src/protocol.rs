use std::collections::BTreeMap;

use rcord_model::{Activity, Snowflake, Status};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub mod op {
    pub const DISPATCH: u8 = 0;
    pub const HEARTBEAT: u8 = 1;
    pub const IDENTIFY: u8 = 2;
    pub const PRESENCE_UPDATE: u8 = 3;
    pub const VOICE_STATE_UPDATE: u8 = 4;
    pub const RESUME: u8 = 6;
    pub const RECONNECT: u8 = 7;
    pub const REQUEST_GUILD_MEMBERS: u8 = 8;
    pub const INVALID_SESSION: u8 = 9;
    pub const HELLO: u8 = 10;
    pub const HEARTBEAT_ACK: u8 = 11;
    pub const CALL_CONNECT: u8 = 13;
    pub const STREAM_CREATE: u8 = 18;
    pub const STREAM_DELETE: u8 = 19;
    pub const STREAM_WATCH: u8 = 20;
    pub const STREAM_SET_PAUSED: u8 = 22;
    pub const GUILD_SUBSCRIPTIONS_BULK: u8 = 37;
}

/// An incoming payload. `d` stays raw so each event is parsed once, straight into its type.
#[derive(Debug, Deserialize)]
pub struct Incoming<'a> {
    pub op: u8,
    #[serde(borrow, default)]
    pub d: Option<&'a RawValue>,
    #[serde(default)]
    pub s: Option<u64>,
    #[serde(default)]
    pub t: Option<&'a str>,
}

#[derive(Debug, Serialize)]
pub struct Outgoing<T: Serialize> {
    pub op: u8,
    pub d: T,
}

#[derive(Debug, Deserialize)]
pub struct Hello {
    pub heartbeat_interval: u64,
}

/// The fields of READY the connection itself needs.
#[derive(Debug, Deserialize)]
pub struct ReadySession {
    pub session_id: String,
    #[serde(default)]
    pub resume_gateway_url: Option<String>,
}

bitflags::bitflags! {
    /// Gateway capabilities a user client announces in IDENTIFY. They change the shape of
    /// READY, so the cache is written against exactly this set.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Capabilities: u64 {
        const LAZY_USER_NOTES = 1 << 0;
        const NO_AFFINE_USER_IDS = 1 << 1;
        const VERSIONED_READ_STATES = 1 << 2;
        const VERSIONED_USER_GUILD_SETTINGS = 1 << 3;
        const DEDUPE_USER_OBJECTS = 1 << 4;
        const PRIORITIZED_READY_PAYLOAD = 1 << 5;
        const MULTIPLE_GUILD_EXPERIMENT_POPULATIONS = 1 << 6;
        const NON_CHANNEL_READ_STATES = 1 << 7;
        const AUTH_TOKEN_REFRESH = 1 << 8;
        const USER_SETTINGS_PROTO = 1 << 9;
        const CLIENT_STATE_V2 = 1 << 10;
        const PASSIVE_GUILD_UPDATE = 1 << 11;
    }
}

impl Default for Capabilities {
    fn default() -> Self {
        Self::LAZY_USER_NOTES
            | Self::NO_AFFINE_USER_IDS
            | Self::VERSIONED_READ_STATES
            | Self::VERSIONED_USER_GUILD_SETTINGS
            | Self::DEDUPE_USER_OBJECTS
            | Self::NON_CHANNEL_READ_STATES
    }
}

/// The Chrome release rcord presents itself as. Discord's web client is what the build
/// number is read from, so rcord reports the same browser shape it does.
pub const CHROME_VERSION: &str = "142.0.0.0";

/// The user agent of the browser rcord presents itself as, matching [`ClientProperties`].
pub fn browser_user_agent() -> String {
    let platform = match std::env::consts::OS {
        "windows" => "Windows NT 10.0; Win64; x64",
        _ => "X11; Linux x86_64",
    };
    format!(
        "Mozilla/5.0 ({platform}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{CHROME_VERSION} Safari/537.36"
    )
}

/// What the client reports about itself, in the shape Discord's web client sends. Discord
/// uses these for analytics and the session list under Settings > Devices, and checks that
/// they agree with the user agent.
#[derive(Debug, Clone, Serialize)]
pub struct ClientProperties {
    pub os: String,
    pub browser: String,
    pub device: String,
    pub system_locale: String,
    pub has_client_mods: bool,
    pub browser_user_agent: String,
    pub browser_version: String,
    pub os_version: String,
    pub referrer: String,
    pub referring_domain: String,
    pub referrer_current: String,
    pub referring_domain_current: String,
    pub release_channel: String,
    pub client_build_number: u64,
    pub client_event_source: Option<String>,
}

impl ClientProperties {
    pub fn for_this_system(build_number: u64, locale: &str) -> Self {
        let os = match std::env::consts::OS {
            "windows" => "Windows",
            _ => "Linux",
        };
        Self {
            os: os.into(),
            browser: "Chrome".into(),
            device: String::new(),
            system_locale: locale.into(),
            has_client_mods: false,
            browser_user_agent: browser_user_agent(),
            browser_version: CHROME_VERSION.into(),
            os_version: if os == "Windows" {
                "10".into()
            } else {
                String::new()
            },
            referrer: String::new(),
            referring_domain: String::new(),
            referrer_current: String::new(),
            referring_domain_current: String::new(),
            release_channel: "stable".into(),
            client_build_number: build_number,
            client_event_source: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PresenceUpdate {
    pub status: Status,
    pub since: u64,
    pub activities: Vec<Activity>,
    pub afk: bool,
}

impl Default for PresenceUpdate {
    fn default() -> Self {
        Self {
            status: Status::Online,
            since: 0,
            activities: Vec::new(),
            afk: false,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Identify<'a> {
    pub token: &'a str,
    pub capabilities: u64,
    pub properties: &'a ClientProperties,
    pub presence: &'a PresenceUpdate,
    pub compress: bool,
    pub client_state: ClientState,
}

#[derive(Debug, Default, Serialize)]
pub struct ClientState {
    pub guild_versions: BTreeMap<String, u64>,
}

#[derive(Debug, Serialize)]
pub struct Resume<'a> {
    pub token: &'a str,
    pub session_id: &'a str,
    pub seq: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct VoiceStateUpdate {
    pub guild_id: Option<Snowflake>,
    pub channel_id: Option<Snowflake>,
    pub self_mute: bool,
    pub self_deaf: bool,
    pub self_video: bool,
}

/// Which parts of a guild to receive events for. User clients are not sent member lists,
/// typing or presences for a guild until they subscribe.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GuildSubscription {
    pub typing: bool,
    pub activities: bool,
    pub threads: bool,
    pub member_updates: bool,
    /// Member list ranges per channel, in blocks of 100: `[[0, 99], [100, 199]]`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub channels: BTreeMap<Snowflake, Vec<[u32; 2]>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GuildSubscriptionsBulk {
    pub subscriptions: BTreeMap<Snowflake, GuildSubscription>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestGuildMembers {
    pub guild_id: Vec<Snowflake>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_ids: Option<Vec<Snowflake>>,
    pub limit: u32,
    pub presences: bool,
}

/// Commands the client can send once connected.
#[derive(Debug, Clone)]
pub enum Command {
    UpdatePresence(PresenceUpdate),
    UpdateVoiceState(VoiceStateUpdate),
    SubscribeGuilds(GuildSubscriptionsBulk),
    RequestGuildMembers(RequestGuildMembers),
}

impl Command {
    pub(crate) fn encode(&self) -> serde_json::Result<String> {
        match self {
            Self::UpdatePresence(d) => serde_json::to_string(&Outgoing {
                op: op::PRESENCE_UPDATE,
                d,
            }),
            Self::UpdateVoiceState(d) => serde_json::to_string(&Outgoing {
                op: op::VOICE_STATE_UPDATE,
                d,
            }),
            Self::SubscribeGuilds(d) => serde_json::to_string(&Outgoing {
                op: op::GUILD_SUBSCRIPTIONS_BULK,
                d,
            }),
            Self::RequestGuildMembers(d) => serde_json::to_string(&Outgoing {
                op: op::REQUEST_GUILD_MEMBERS,
                d,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dispatch_lazily() {
        let p: Incoming<'_> =
            serde_json::from_str(r#"{"op":0,"s":42,"t":"MESSAGE_CREATE","d":{"id":"1"}}"#).unwrap();
        assert_eq!(p.op, op::DISPATCH);
        assert_eq!(p.s, Some(42));
        assert_eq!(p.t, Some("MESSAGE_CREATE"));
        assert_eq!(p.d.unwrap().get(), r#"{"id":"1"}"#);
    }

    #[test]
    fn encodes_subscriptions() {
        let g = Snowflake::new(1).unwrap();
        let c = Snowflake::new(2).unwrap();
        let mut sub = GuildSubscription {
            typing: true,
            activities: true,
            threads: true,
            member_updates: false,
            channels: BTreeMap::new(),
        };
        sub.channels.insert(c, vec![[0, 99]]);
        let cmd = Command::SubscribeGuilds(GuildSubscriptionsBulk {
            subscriptions: [(g, sub)].into(),
        });
        assert_eq!(
            cmd.encode().unwrap(),
            r#"{"op":37,"d":{"subscriptions":{"1":{"typing":true,"activities":true,"threads":true,"member_updates":false,"channels":{"2":[[0,99]]}}}}}"#
        );
    }
}
