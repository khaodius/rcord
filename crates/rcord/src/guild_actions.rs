//! What the channel and server right-click menus do: mutes and notification levels, invites,
//! creating, renaming, duplicating and deleting channels, and leaving a server. Each request
//! reports a failure as a toast; Discord's gateway events then update the cache.

use gpui::{ClipboardItem, Context};
use rcord_model::{ChannelKind, NotifyLevel, Snowflake};
use rcord_rest::routes::ChannelSpec;
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::net::Net;
use crate::state::AppState;

/// The mute lengths Discord offers, in minutes; `None` lasts until turned off.
pub const MUTE_LENGTHS: [(&str, Option<i64>); 6] = [
    ("For 15 Minutes", Some(15)),
    ("For 1 Hour", Some(60)),
    ("For 3 Hours", Some(180)),
    ("For 8 Hours", Some(480)),
    ("For 24 Hours", Some(1440)),
    ("Until I turn it back on", None),
];

fn mute_body(minutes: Option<i64>) -> Value {
    let end = minutes.and_then(|m| {
        (OffsetDateTime::now_utc() + time::Duration::minutes(m))
            .format(&Rfc3339)
            .ok()
    });
    json!({
        "muted": true,
        "mute_config": {
            "selected_time_window": minutes.map_or(-1, |m| m * 60),
            "end_time": end,
        },
    })
}

impl AppState {
    /// Runs a REST call and toasts `what` failed if it does.
    fn guild_request<F, Fut, T>(
        &mut self,
        cx: &mut Context<Self>,
        what: &'static str,
        f: F,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) where
        F: FnOnce(rcord_rest::Client) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = rcord_rest::Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        let Some(rest) = cx.global::<Net>().rest() else {
            return;
        };
        let task = cx.global::<Net>().run(async move { f(rest).await });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |s, cx| match result {
                Some(Ok(v)) => done(s, v, cx),
                Some(Err(e)) => s.toast(format!("Could not {what}: {e}"), cx),
                None => {}
            })
            .ok();
        })
        .detach();
    }

    fn patch_guild_settings(&mut self, guild: Snowflake, body: Value, cx: &mut Context<Self>) {
        self.guild_request(
            cx,
            "change the notification settings",
            move |rest| async move { rest.update_guild_settings(guild, &body).await },
            |_, (), _| {},
        );
    }

    fn channel_guild(&self, channel: Snowflake) -> Option<Snowflake> {
        self.store.channel(channel).and_then(|c| c.guild_id)
    }

    pub fn mute_channel(
        &mut self,
        channel: Snowflake,
        minutes: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let Some(guild) = self.channel_guild(channel) else {
            return;
        };
        let body = json!({ "channel_overrides": { channel.to_string(): mute_body(minutes) } });
        self.patch_guild_settings(guild, body, cx);
    }

    pub fn unmute_channel(&mut self, channel: Snowflake, cx: &mut Context<Self>) {
        let Some(guild) = self.channel_guild(channel) else {
            return;
        };
        let body = json!({ "channel_overrides": { channel.to_string(): { "muted": false } } });
        self.patch_guild_settings(guild, body, cx);
    }

    pub fn set_channel_notify(
        &mut self,
        channel: Snowflake,
        level: NotifyLevel,
        cx: &mut Context<Self>,
    ) {
        let Some(guild) = self.channel_guild(channel) else {
            return;
        };
        let body = json!({ "channel_overrides": {
            channel.to_string(): { "message_notifications": u8::from(level) }
        } });
        self.patch_guild_settings(guild, body, cx);
    }

    pub fn mute_guild(&mut self, guild: Snowflake, minutes: Option<i64>, cx: &mut Context<Self>) {
        self.patch_guild_settings(guild, mute_body(minutes), cx);
    }

    pub fn unmute_guild(&mut self, guild: Snowflake, cx: &mut Context<Self>) {
        self.patch_guild_settings(guild, json!({ "muted": false }), cx);
    }

    pub fn set_guild_notify(
        &mut self,
        guild: Snowflake,
        level: NotifyLevel,
        cx: &mut Context<Self>,
    ) {
        self.patch_guild_settings(
            guild,
            json!({ "message_notifications": u8::from(level) }),
            cx,
        );
    }

    pub fn set_hide_muted(&mut self, guild: Snowflake, hide: bool, cx: &mut Context<Self>) {
        self.patch_guild_settings(guild, json!({ "hide_muted_channels": hide }), cx);
    }

    /// Makes a seven-day invite to `channel` and copies its link.
    pub fn copy_invite(&mut self, channel: Snowflake, cx: &mut Context<Self>) {
        self.guild_request(
            cx,
            "make an invite",
            move |rest| async move { rest.create_invite(channel).await },
            |s, code, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(format!(
                    "https://discord.gg/{code}"
                )));
                s.toast("Invite link copied. It lasts 7 days.", cx);
            },
        );
    }

    /// The channel invites to a server go to: the one on screen if it is there, else the
    /// first text channel you can see.
    pub fn invite_channel(&self, guild: Snowflake) -> Option<Snowflake> {
        if let Some(c) = self.channel
            && self.channel_guild(c) == Some(guild)
        {
            return Some(c);
        }
        let entry = self.store.guild(guild)?;
        entry
            .channel_ids
            .iter()
            .copied()
            .filter(|id| {
                self.store
                    .channel(*id)
                    .is_some_and(|c| c.kind == ChannelKind::Text)
                    && self.store.can_view(*id)
            })
            .min_by_key(|id| self.store.channel(*id).map_or(i32::MAX, |c| c.position))
    }

    pub fn create_channel(
        &mut self,
        guild: Snowflake,
        name: String,
        category: bool,
        parent: Option<Snowflake>,
        cx: &mut Context<Self>,
    ) {
        let name = if category {
            name.trim().to_owned()
        } else {
            channel_slug(&name)
        };
        if name.is_empty() {
            return;
        }
        let spec = ChannelSpec {
            name: Some(name),
            kind: Some(if category { 4 } else { 0 }),
            parent_id: parent.filter(|_| !category),
            ..ChannelSpec::default()
        };
        self.guild_request(
            cx,
            "create the channel",
            move |rest| async move { rest.create_channel(guild, &spec).await },
            |_, _, _| {},
        );
    }

    pub fn rename_channel(&mut self, channel: Snowflake, name: String, cx: &mut Context<Self>) {
        let category = self
            .store
            .channel(channel)
            .is_some_and(|c| c.kind == ChannelKind::Category);
        let name = if category {
            name.trim().to_owned()
        } else {
            channel_slug(&name)
        };
        if name.is_empty() {
            return;
        }
        let spec = ChannelSpec {
            name: Some(name),
            ..ChannelSpec::default()
        };
        self.guild_request(
            cx,
            "rename the channel",
            move |rest| async move { rest.edit_channel(channel, &spec).await },
            |_, _, _| {},
        );
    }

    /// A copy with the same name, kind, category, topic and permissions, as Discord's
    /// Duplicate Channel makes.
    pub fn duplicate_channel(&mut self, channel: Snowflake, cx: &mut Context<Self>) {
        let Some(c) = self.store.channel(channel) else {
            return;
        };
        let Some(guild) = c.guild_id else {
            return;
        };
        let spec = ChannelSpec {
            name: c.name.clone(),
            kind: Some(c.kind.into()),
            topic: c.topic.clone(),
            parent_id: c.parent_id,
            nsfw: Some(c.nsfw),
            rate_limit_per_user: c.rate_limit_per_user,
            bitrate: c.bitrate,
            user_limit: c.user_limit,
            permission_overwrites: serde_json::to_value(&c.permission_overwrites).ok(),
        };
        self.guild_request(
            cx,
            "duplicate the channel",
            move |rest| async move { rest.create_channel(guild, &spec).await },
            |_, _, _| {},
        );
    }

    pub fn delete_channel(&mut self, channel: Snowflake, cx: &mut Context<Self>) {
        self.guild_request(
            cx,
            "delete the channel",
            move |rest| async move { rest.delete_channel(channel).await },
            |_, (), _| {},
        );
    }

    pub fn leave_guild(&mut self, guild: Snowflake, cx: &mut Context<Self>) {
        self.guild_request(
            cx,
            "leave the server",
            move |rest| async move { rest.leave_guild(guild).await },
            |_, (), _| {},
        );
    }
}

impl AppState {
    /// Servers in the order the server list shows them.
    pub fn ordered_guilds(&self) -> Vec<Snowflake> {
        let order = &self.settings.layout.guild_order;
        let mut ids: Vec<Snowflake> = self.store.guilds().map(|g| g.guild.id).collect();
        ids.sort_by_key(|g| {
            order
                .iter()
                .position(|id| *id == g.get())
                .unwrap_or(usize::MAX)
        });
        ids
    }

    pub fn move_guild_to(&mut self, guild: Snowflake, index: usize, cx: &mut Context<Self>) {
        let mut ids = self.ordered_guilds();
        ids.retain(|g| *g != guild);
        ids.insert(index.min(ids.len()), guild);
        self.update_settings(cx, |s| {
            s.layout.guild_order = ids.iter().map(|g| g.get()).collect();
        });
    }
}

/// Text channel names as Discord makes them: lowercase, spaces as dashes.
pub fn channel_slug(name: &str) -> String {
    let mut out = String::new();
    for word in name.split_whitespace() {
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&word.to_lowercase());
    }
    out.chars().take(100).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_names() {
        assert_eq!(channel_slug("  Game Night  Chat "), "game-night-chat");
        assert_eq!(channel_slug(""), "");
    }

    #[test]
    fn mute_bodies() {
        let forever = mute_body(None);
        assert_eq!(forever["mute_config"]["selected_time_window"], -1);
        assert!(forever["mute_config"]["end_time"].is_null());
        let hour = mute_body(Some(60));
        assert_eq!(hour["mute_config"]["selected_time_window"], 3600);
        assert!(hour["mute_config"]["end_time"].is_string());
    }
}
