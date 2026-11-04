//! Typed endpoints. Each one is a thin wrapper over [`Client::request`].

use http::Method;
use rcord_model::{Channel, Emoji, Message, Snowflake, StickerPack, User, UserProfile};
use serde::{Deserialize, Serialize};

use crate::{Client, Result};

/// Percent-encodes a path segment, such as a Unicode emoji in a reactions URL.
pub fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b':') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Around {
    Before(Snowflake),
    After(Snowflake),
    Around(Snowflake),
    Latest,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CreateMessage {
    pub content: String,
    /// Matches the echoed MESSAGE_CREATE to the pending message shown while sending.
    pub nonce: String,
    pub tts: bool,
    pub flags: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_mentions: Option<AllowedMentions>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sticker_ids: Vec<Snowflake>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reference {
    pub message_id: Snowflake,
    pub channel_id: Snowflake,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<Snowflake>,
    pub fail_if_not_exists: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AllowedMentions {
    pub parse: Vec<&'static str>,
    pub replied_user: bool,
}

impl AllowedMentions {
    /// Discord's default with reply pings turned off.
    pub fn without_reply_ping() -> Self {
        Self {
            parse: vec!["users", "roles", "everyone"],
            replied_user: false,
        }
    }
}

/// One result of the GIF picker's search.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Gif {
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// The page link, which is what gets sent so the message embeds it.
    pub url: String,
    /// The media in the requested format.
    pub src: String,
    #[serde(default)]
    pub gif_src: Option<String>,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub preview: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrendingGifs {
    #[serde(default)]
    pub categories: Vec<GifCategory>,
    #[serde(default)]
    pub gifs: Vec<Gif>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GifCategory {
    pub name: String,
    pub src: String,
}

#[derive(Debug, Clone, Deserialize)]
struct StickerPacks {
    sticker_packs: Vec<StickerPack>,
}

#[derive(Debug, Serialize)]
struct Ack {
    token: Option<String>,
}

#[derive(Debug, Serialize)]
struct EditMessage<'a> {
    content: &'a str,
}

impl Client {
    pub async fn current_user(&self) -> Result<User> {
        self.get("/users/@me").await
    }

    /// Up to 100 messages, newest first.
    pub async fn messages(
        &self,
        channel: Snowflake,
        at: Around,
        limit: u8,
    ) -> Result<Vec<Message>> {
        let limit = limit.clamp(1, 100);
        let anchor = match at {
            Around::Before(id) => format!("&before={id}"),
            Around::After(id) => format!("&after={id}"),
            Around::Around(id) => format!("&around={id}"),
            Around::Latest => String::new(),
        };
        self.get(&format!(
            "/channels/{channel}/messages?limit={limit}{anchor}"
        ))
        .await
    }

    pub async fn send_message(
        &self,
        channel: Snowflake,
        message: &CreateMessage,
    ) -> Result<Message> {
        self.post(&format!("/channels/{channel}/messages"), message)
            .await
    }

    pub async fn edit_message(
        &self,
        channel: Snowflake,
        message: Snowflake,
        content: &str,
    ) -> Result<Message> {
        self.patch(
            &format!("/channels/{channel}/messages/{message}"),
            &EditMessage { content },
        )
        .await
    }

    pub async fn delete_message(&self, channel: Snowflake, message: Snowflake) -> Result<()> {
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/channels/{channel}/messages/{message}"),
            None,
        )
        .await
    }

    pub async fn add_reaction(
        &self,
        channel: Snowflake,
        message: Snowflake,
        emoji: &Emoji,
    ) -> Result<()> {
        let e = encode_segment(&emoji.to_reaction_form());
        self.send_empty::<()>(
            Method::PUT,
            &format!("/channels/{channel}/messages/{message}/reactions/{e}/@me?location=Message"),
            None,
        )
        .await
    }

    pub async fn remove_reaction(
        &self,
        channel: Snowflake,
        message: Snowflake,
        emoji: &Emoji,
    ) -> Result<()> {
        let e = encode_segment(&emoji.to_reaction_form());
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/channels/{channel}/messages/{message}/reactions/{e}/@me?location=Message"),
            None,
        )
        .await
    }

    pub async fn trigger_typing(&self, channel: Snowflake) -> Result<()> {
        self.send_empty::<()>(Method::POST, &format!("/channels/{channel}/typing"), None)
            .await
    }

    /// Marks a channel read up to `message`.
    pub async fn ack(&self, channel: Snowflake, message: Snowflake) -> Result<()> {
        self.send_empty(
            Method::POST,
            &format!("/channels/{channel}/messages/{message}/ack"),
            Some(&Ack { token: None }),
        )
        .await
    }

    /// Opens (or finds) the DM with `user`.
    pub async fn create_dm(&self, user: Snowflake) -> Result<Channel> {
        self.post(
            "/users/@me/channels",
            &serde_json::json!({ "recipient_id": user.to_string() }),
        )
        .await
    }

    /// Accepts a friend request from `user`, or sends one to them.
    pub async fn add_friend(&self, user: Snowflake) -> Result<()> {
        self.send_empty(
            Method::PUT,
            &format!("/users/@me/relationships/{user}"),
            Some(&serde_json::json!({})),
        )
        .await
    }

    /// Sends a friend request by username.
    pub async fn send_friend_request(&self, username: &str) -> Result<()> {
        self.send_empty(
            Method::POST,
            "/users/@me/relationships",
            Some(&serde_json::json!({ "username": username, "discriminator": null })),
        )
        .await
    }

    /// Removes a friend, ignores or cancels a request, or unblocks.
    pub async fn remove_relationship(&self, user: Snowflake) -> Result<()> {
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/users/@me/relationships/{user}"),
            None,
        )
        .await
    }

    /// Changes a member's voice state as a moderator: server mute, server deafen, or the
    /// voice channel they are in (`Some(None)` disconnects them).
    pub async fn modify_member_voice(
        &self,
        guild: Snowflake,
        user: Snowflake,
        change: MemberVoiceChange,
    ) -> Result<()> {
        let mut body = serde_json::Map::new();
        if let Some(mute) = change.mute {
            body.insert("mute".into(), mute.into());
        }
        if let Some(deaf) = change.deaf {
            body.insert("deaf".into(), deaf.into());
        }
        if let Some(channel) = change.channel {
            body.insert(
                "channel_id".into(),
                channel.map_or(serde_json::Value::Null, |c| c.to_string().into()),
            );
        }
        self.send_empty(
            Method::PATCH,
            &format!("/guilds/{guild}/members/{user}"),
            Some(&body),
        )
        .await
    }

    /// Removes `user` from the server. They can rejoin with an invite.
    pub async fn kick_member(&self, guild: Snowflake, user: Snowflake) -> Result<()> {
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/guilds/{guild}/members/{user}"),
            None,
        )
        .await
    }

    /// Bans `user` from the server without deleting their messages.
    pub async fn ban_member(&self, guild: Snowflake, user: Snowflake) -> Result<()> {
        self.send_empty(
            Method::PUT,
            &format!("/guilds/{guild}/bans/{user}"),
            Some(&serde_json::json!({ "delete_message_seconds": 0 })),
        )
        .await
    }

    /// The latest thumbnail of a Go Live stream, as an image URL. Discord updates it every
    /// few minutes while the stream runs; a stream that just started may have none yet.
    pub async fn stream_preview(&self, stream_key: &str) -> Result<Option<String>> {
        #[derive(Deserialize)]
        struct Preview {
            url: Option<String>,
        }
        let p: Preview = self
            .get(&format!("/streams/{}/preview", encode_segment(stream_key)))
            .await?;
        Ok(p.url)
    }

    /// The full profile, with badges, bio, connections and mutual servers.
    pub async fn user_profile(
        &self,
        user: Snowflake,
        guild: Option<Snowflake>,
    ) -> Result<UserProfile> {
        let guild = guild.map(|g| format!("&guild_id={g}")).unwrap_or_default();
        self.get(&format!(
            "/users/{user}/profile?with_mutual_guilds=true&with_mutual_friends_count=true{guild}"
        ))
        .await
    }

    /// The standard sticker packs every account can use.
    pub async fn sticker_packs(&self) -> Result<Vec<StickerPack>> {
        let packs: StickerPacks = self.get("/sticker-packs").await?;
        Ok(packs.sticker_packs)
    }

    pub async fn trending_gifs(&self, locale: &str) -> Result<TrendingGifs> {
        self.get(&format!(
            "/gifs/trending?provider=tenor&locale={}&media_format=gif",
            encode_segment(locale)
        ))
        .await
    }

    pub async fn search_gifs(&self, query: &str, locale: &str) -> Result<Vec<Gif>> {
        self.get(&format!(
            "/gifs/search?q={}&provider=tenor&locale={}&media_format=gif",
            encode_segment(query),
            encode_segment(locale)
        ))
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_emoji() {
        assert_eq!(encode_segment("🔥"), "%F0%9F%94%A5");
        assert_eq!(encode_segment("LUL:123"), "LUL:123");
        assert_eq!(encode_segment("a b"), "a%20b");
    }

    #[test]
    fn create_message_skips_empty_fields() {
        let m = CreateMessage {
            content: "hi".into(),
            nonce: "1".into(),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&m).unwrap(),
            r#"{"content":"hi","nonce":"1","tts":false,"flags":0}"#
        );
    }
}

/// A moderator's change to someone's voice state; fields left `None` stay as they are.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemberVoiceChange {
    pub mute: Option<bool>,
    pub deaf: Option<bool>,
    /// `Some(None)` disconnects them from voice.
    pub channel: Option<Option<Snowflake>>,
}
