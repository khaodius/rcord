//! The in-memory cache of everything the account can see, kept current by gateway events.
//! Lookups are by snowflake in hash maps; message history is a bounded window per channel.

pub mod events;

use std::collections::VecDeque;

use rcord_model::{
    Channel, ChannelKind, Emoji, Guild, Member, Message, NotifyLevel, Permissions, Presence,
    Reaction, ReactionCounts, Relationship, RelationshipKind, Snowflake, Status, User,
    UserGuildSettings, VoiceState, base_permissions, channel_permissions,
};
use rustc_hash::FxHashMap;

pub use events::{
    MemberListGroup, StreamDelete, StreamInfo, StreamServerUpdate, Update, VoiceServerUpdate, parse,
};
use events::{MemberListItem, MemberListOp, MemberListUpdate, ReactionEvent};

/// What an update changed, so views redraw only what they show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Everything,
    Guilds,
    Guild(Snowflake),
    Channels(Option<Snowflake>),
    Messages(Snowflake),
    ReadState(Snowflake),
    Members(Snowflake),
    Presence(Snowflake),
    Typing(Snowflake),
    Voice(Option<Snowflake>),
    Me,
    Relationships,
    None,
}

#[derive(Debug)]
pub struct GuildEntry {
    pub guild: Guild,
    pub channel_ids: Vec<Snowflake>,
    pub members: FxHashMap<Snowflake, Member>,
}

#[derive(Debug, Default)]
pub struct ChannelMessages {
    /// Oldest first.
    pub messages: VecDeque<Message>,
    /// The channel's first message is loaded; there is nothing older to fetch.
    pub reached_start: bool,
    /// The newest messages are loaded, so live events can be appended.
    pub live: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReadState {
    pub last_read: Option<Snowflake>,
    pub mentions: u32,
}

/// A row of the member list sidebar.
#[derive(Debug, Clone)]
pub enum MemberRow {
    Group { id: String, count: u32 },
    Member(Snowflake),
}

#[derive(Debug, Default)]
pub struct MemberList {
    pub id: String,
    pub rows: Vec<Option<MemberRow>>,
    pub online: u32,
    pub total: u32,
}

/// A node of the channel sidebar: a category with its children, or a top-level channel.
#[derive(Debug, Clone)]
pub struct ChannelGroup {
    pub category: Option<Snowflake>,
    pub channels: Vec<Snowflake>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingMessage {
    pub nonce: String,
    pub content: String,
    pub failed: bool,
}

#[derive(Debug)]
pub struct Store {
    pub me: Option<User>,
    users: FxHashMap<Snowflake, User>,
    guilds: FxHashMap<Snowflake, GuildEntry>,
    guild_order: Vec<Snowflake>,
    channels: FxHashMap<Snowflake, Channel>,
    private_channels: Vec<Snowflake>,
    messages: FxHashMap<Snowflake, ChannelMessages>,
    pending: FxHashMap<Snowflake, Vec<PendingMessage>>,
    read_states: FxHashMap<Snowflake, ReadState>,
    presences: FxHashMap<Snowflake, Presence>,
    relationships: FxHashMap<Snowflake, Relationship>,
    typing: FxHashMap<Snowflake, Vec<(Snowflake, u64)>>,
    voice: FxHashMap<Snowflake, VoiceState>,
    /// Every member list Discord keeps us subscribed to, by guild and list ID. A guild has one
    /// list per distinct set of channel permissions.
    member_lists: FxHashMap<(Snowflake, String), MemberList>,
    /// The list shown for each guild: the one Discord last sent a full SYNC of, which answers
    /// the subscription for the channel on screen.
    current_member_list: FxHashMap<Snowflake, String>,
    /// Your notification settings by server; direct messages are under the zero ID.
    guild_settings: FxHashMap<u64, UserGuildSettings>,
    message_cap: usize,
}

impl Default for Store {
    fn default() -> Self {
        Self::new(250)
    }
}

const TYPING_MS: u64 = 10_000;

impl Store {
    pub fn new(messages_per_channel: usize) -> Self {
        Self {
            me: None,
            users: FxHashMap::default(),
            guilds: FxHashMap::default(),
            guild_order: Vec::new(),
            channels: FxHashMap::default(),
            private_channels: Vec::new(),
            messages: FxHashMap::default(),
            pending: FxHashMap::default(),
            read_states: FxHashMap::default(),
            presences: FxHashMap::default(),
            relationships: FxHashMap::default(),
            typing: FxHashMap::default(),
            voice: FxHashMap::default(),
            member_lists: FxHashMap::default(),
            current_member_list: FxHashMap::default(),
            guild_settings: FxHashMap::default(),
            message_cap: messages_per_channel.max(50),
        }
    }

    pub fn set_message_cap(&mut self, cap: usize) {
        self.message_cap = cap.max(50);
    }

    // ---- reads --------------------------------------------------------------------------

    pub fn me_id(&self) -> Option<Snowflake> {
        self.me.as_ref().map(|u| u.id)
    }

    pub fn user(&self, id: Snowflake) -> Option<&User> {
        self.users.get(&id)
    }

    pub fn guild(&self, id: Snowflake) -> Option<&GuildEntry> {
        self.guilds.get(&id)
    }

    pub fn guilds(&self) -> impl Iterator<Item = &GuildEntry> {
        self.guild_order.iter().filter_map(|id| self.guilds.get(id))
    }

    pub fn channel(&self, id: Snowflake) -> Option<&Channel> {
        self.channels.get(&id)
    }

    /// DMs and group DMs, most recently active first.
    pub fn private_channels(&self) -> impl Iterator<Item = &Channel> {
        self.private_channels
            .iter()
            .filter_map(|id| self.channels.get(id))
    }

    pub fn messages(&self, channel: Snowflake) -> Option<&ChannelMessages> {
        self.messages.get(&channel)
    }

    pub fn message(&self, channel: Snowflake, id: Snowflake) -> Option<&Message> {
        let list = &self.messages.get(&channel)?.messages;
        list.binary_search_by_key(&id, |m| m.id)
            .ok()
            .map(|i| &list[i])
    }

    pub fn pending(&self, channel: Snowflake) -> &[PendingMessage] {
        self.pending.get(&channel).map_or(&[], Vec::as_slice)
    }

    pub fn presence(&self, user: Snowflake) -> Option<&Presence> {
        self.presences.get(&user)
    }

    pub fn status(&self, user: Snowflake) -> Status {
        self.presences
            .get(&user)
            .map_or(Status::Offline, |p| p.status)
    }

    pub fn relationships(&self) -> impl Iterator<Item = &Relationship> {
        self.relationships.values()
    }

    pub fn friends(&self) -> impl Iterator<Item = &User> {
        self.relationships
            .values()
            .filter(|r| r.kind == RelationshipKind::Friend)
            .filter_map(|r| self.users.get(&r.id))
    }

    pub fn member(&self, guild: Snowflake, user: Snowflake) -> Option<&Member> {
        self.guilds.get(&guild)?.members.get(&user)
    }

    pub fn member_list(&self, guild: Snowflake) -> Option<&MemberList> {
        let id = self.current_member_list.get(&guild)?;
        self.member_lists.get(&(guild, id.clone()))
    }

    pub fn voice_states_in(&self, channel: Snowflake) -> impl Iterator<Item = &VoiceState> {
        self.voice
            .values()
            .filter(move |v| v.channel_id == Some(channel))
    }

    /// Everyone in a voice channel that this client knows of.
    pub fn voice_states(&self) -> impl Iterator<Item = &VoiceState> {
        self.voice.values()
    }

    pub fn voice_state(&self, user: Snowflake) -> Option<&VoiceState> {
        self.voice.get(&user)
    }

    pub fn my_voice_state(&self) -> Option<&VoiceState> {
        self.me_id().and_then(|id| self.voice.get(&id))
    }

    /// The name shown for `user` in `guild`: nickname, then display name, then username.
    pub fn display_name(&self, guild: Option<Snowflake>, user: Snowflake) -> Option<&str> {
        if let Some(nick) = guild
            .and_then(|g| self.member(g, user))
            .and_then(|m| m.nick.as_deref())
        {
            return Some(nick);
        }
        if let Some(r) = self.relationships.get(&user)
            && let Some(nick) = r.nickname.as_deref()
        {
            return Some(nick);
        }
        self.users.get(&user).map(User::display_name)
    }

    pub fn name_color(&self, guild: Snowflake, user: Snowflake) -> Option<u32> {
        let entry = self.guilds.get(&guild)?;
        entry.members.get(&user)?.color(&entry.guild)
    }

    /// The name shown for a channel: `#name` style text for guild channels, the other
    /// person's name for a DM, or the joined names of a group DM.
    pub fn channel_name(&self, id: Snowflake) -> String {
        let Some(c) = self.channels.get(&id) else {
            return "unknown".into();
        };
        if let Some(name) = c.name.as_deref().filter(|n| !n.is_empty()) {
            return name.to_owned();
        }
        let names: Vec<&str> = c
            .recipient_ids()
            .filter_map(|u| self.display_name(None, u))
            .collect();
        if names.is_empty() {
            "Unnamed".into()
        } else {
            names.join(", ")
        }
    }

    pub fn permissions(&self, channel: Snowflake) -> Permissions {
        let Some(c) = self.channels.get(&channel) else {
            return Permissions::empty();
        };
        let (Some(guild_id), Some(me)) = (c.guild_id, self.me_id()) else {
            return Permissions::all();
        };
        let Some(entry) = self.guilds.get(&guild_id) else {
            return Permissions::empty();
        };
        let roles = entry
            .members
            .get(&me)
            .map(|m| m.roles.as_slice())
            .unwrap_or_default();
        let base = base_permissions(&entry.guild, me, roles);
        // Threads inherit their parent's overwrites.
        let source = if c.kind.is_thread() {
            c.parent_id.and_then(|p| self.channels.get(&p)).unwrap_or(c)
        } else {
            c
        };
        channel_permissions(base, guild_id, source, me, roles)
    }

    /// Your permissions across `guild`, before any channel's overwrites.
    pub fn guild_permissions(&self, guild: Snowflake) -> Permissions {
        let (Some(entry), Some(me)) = (self.guilds.get(&guild), self.me_id()) else {
            return Permissions::empty();
        };
        let roles = entry
            .members
            .get(&me)
            .map(|m| m.roles.as_slice())
            .unwrap_or_default();
        base_permissions(&entry.guild, me, roles)
    }

    pub fn can_view(&self, channel: Snowflake) -> bool {
        self.permissions(channel)
            .contains(Permissions::VIEW_CHANNEL)
    }

    /// The sidebar for `guild`: uncategorized channels first, then each category in order,
    /// leaving out what the account cannot see and categories left empty by that.
    pub fn channel_groups(&self, guild: Snowflake) -> Vec<ChannelGroup> {
        let Some(entry) = self.guilds.get(&guild) else {
            return Vec::new();
        };
        let mut categories: Vec<&Channel> = Vec::new();
        let mut children: FxHashMap<Option<Snowflake>, Vec<&Channel>> = FxHashMap::default();
        for c in entry
            .channel_ids
            .iter()
            .filter_map(|id| self.channels.get(id))
        {
            if c.kind.is_thread() {
                continue;
            }
            if c.kind == ChannelKind::Category {
                categories.push(c);
            } else if self.can_view(c.id) {
                children.entry(c.parent_id).or_default().push(c);
            }
        }
        categories.sort_by_key(|c| (c.position, c.id));
        let mut sorted = |key: Option<Snowflake>| {
            let mut list = children.remove(&key).unwrap_or_default();
            list.sort_by_key(|c| c.sort_key());
            list.into_iter().map(|c| c.id).collect::<Vec<_>>()
        };
        let mut groups = vec![ChannelGroup {
            category: None,
            channels: sorted(None),
        }];
        for cat in categories {
            let channels = sorted(Some(cat.id));
            if !channels.is_empty() {
                groups.push(ChannelGroup {
                    category: Some(cat.id),
                    channels,
                });
            }
        }
        // Children whose category is gone or hidden still show, at the top.
        let orphans: Vec<Snowflake> = children.into_values().flatten().map(|c| c.id).collect();
        groups[0].channels.extend(orphans);
        if groups[0].channels.is_empty() {
            groups.remove(0);
        }
        groups
    }

    pub fn read_state(&self, channel: Snowflake) -> ReadState {
        self.read_states.get(&channel).copied().unwrap_or_default()
    }

    /// Whether `channel` counts toward unread markers. Voice channels do not: their text
    /// chat cannot be opened here, so a marker could never be cleared. Muted channels, and
    /// the channels of muted servers, do not either, as in Discord; their mentions still count.
    pub fn tracks_unread(&self, channel: Snowflake) -> bool {
        self.channels
            .get(&channel)
            .is_some_and(|c| !c.kind.is_voice())
            && !self.is_channel_muted(channel)
            && !self
                .channels
                .get(&channel)
                .and_then(|c| c.guild_id)
                .is_some_and(|g| self.is_guild_muted(g))
    }

    pub fn guild_settings(&self, guild: Option<Snowflake>) -> Option<&UserGuildSettings> {
        self.guild_settings.get(&guild.map_or(0, Snowflake::get))
    }

    pub fn is_guild_muted(&self, guild: Snowflake) -> bool {
        self.guild_settings(Some(guild))
            .is_some_and(|s| s.is_muted(time::OffsetDateTime::now_utc()))
    }

    /// The channel itself is muted, or the category it sits in.
    pub fn is_channel_muted(&self, channel: Snowflake) -> bool {
        let Some(c) = self.channels.get(&channel) else {
            return false;
        };
        let Some(settings) = self.guild_settings(c.guild_id) else {
            return false;
        };
        let now = time::OffsetDateTime::now_utc();
        settings.is_channel_muted(channel, now)
            || c.parent_id
                .is_some_and(|p| settings.is_channel_muted(p, now))
    }

    /// The level a channel is set to, before falling back to its server.
    pub fn channel_notify_level(&self, channel: Snowflake) -> NotifyLevel {
        let guild = self.channels.get(&channel).and_then(|c| c.guild_id);
        self.guild_settings(guild)
            .and_then(|s| s.channel(channel))
            .map_or(NotifyLevel::Inherit, |o| o.message_notifications)
    }

    /// The level a server gives, with its default filled in.
    pub fn guild_notify_level(&self, guild: Snowflake) -> NotifyLevel {
        match self
            .guild_settings(Some(guild))
            .map_or(NotifyLevel::Inherit, |s| s.message_notifications)
        {
            NotifyLevel::Inherit => match self.guilds.get(&guild) {
                Some(g) if g.guild.default_message_notifications == 0 => NotifyLevel::All,
                _ => NotifyLevel::Mentions,
            },
            level => level,
        }
    }

    pub fn hides_muted_channels(&self, guild: Snowflake) -> bool {
        self.guild_settings(Some(guild))
            .is_some_and(|s| s.hide_muted_channels)
    }

    pub fn is_unread(&self, channel: Snowflake) -> bool {
        if !self.tracks_unread(channel) {
            return false;
        }
        let Some(last) = self.channels.get(&channel).and_then(|c| c.last_message_id) else {
            return false;
        };
        match self.read_states.get(&channel).and_then(|r| r.last_read) {
            Some(read) => last > read,
            // Channels never opened count as read, as in Discord.
            None => false,
        }
    }

    pub fn guild_unread(&self, guild: Snowflake) -> (bool, u32) {
        let Some(entry) = self.guilds.get(&guild) else {
            return (false, 0);
        };
        let mut unread = false;
        let mut mentions = 0;
        for id in &entry.channel_ids {
            if self.channels.get(id).is_some_and(|c| !c.kind.is_voice()) {
                mentions += self.read_state(*id).mentions;
            }
            if !self.tracks_unread(*id) {
                continue;
            }
            if !unread && self.is_unread(*id) && self.can_view(*id) {
                unread = true;
            }
        }
        (unread, mentions)
    }

    /// Users typing in `channel` at `now_ms`, excluding the account itself.
    pub fn typing(&self, channel: Snowflake, now_ms: u64) -> Vec<Snowflake> {
        let me = self.me_id();
        self.typing
            .get(&channel)
            .map(|list| {
                list.iter()
                    .filter(|(u, until)| *until > now_ms && Some(*u) != me)
                    .map(|(u, _)| *u)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every custom emoji the account can use, by guild.
    pub fn emojis(&self) -> impl Iterator<Item = (&Guild, &Emoji)> {
        self.guilds()
            .flat_map(|g| g.guild.emojis.iter().map(move |e| (&g.guild, e)))
            .filter(|(_, e)| e.available)
    }

    // ---- local writes -------------------------------------------------------------------

    /// Shows a message as sending until the gateway echoes it back with the same nonce.
    pub fn add_pending(&mut self, channel: Snowflake, nonce: String, content: String) {
        self.pending
            .entry(channel)
            .or_default()
            .push(PendingMessage {
                nonce,
                content,
                failed: false,
            });
    }

    pub fn fail_pending(&mut self, channel: Snowflake, nonce: &str) {
        if let Some(p) = self
            .pending
            .get_mut(&channel)
            .and_then(|l| l.iter_mut().find(|p| p.nonce == nonce))
        {
            p.failed = true;
        }
    }

    /// Drops the cached history of a channel. Read state and unsent messages stay.
    pub fn forget_messages(&mut self, channel: Snowflake) {
        self.messages.remove(&channel);
    }

    pub fn remove_pending(&mut self, channel: Snowflake, nonce: &str) {
        if let Some(list) = self.pending.get_mut(&channel) {
            list.retain(|p| p.nonce != nonce);
        }
    }

    /// Merges a page of history from the REST API, which returns newest first.
    pub fn insert_history(
        &mut self,
        channel: Snowflake,
        mut page: Vec<Message>,
        requested: u8,
        latest: bool,
    ) {
        let reached_start = page.len() < usize::from(requested);
        for m in &page {
            self.remember_user(&m.author);
        }
        page.sort_by_key(|m| m.id);
        let entry = self.messages.entry(channel).or_default();
        if latest {
            entry.live = true;
        }
        if reached_start && !latest {
            entry.reached_start = true;
        }
        if latest && reached_start {
            entry.reached_start = true;
        }
        for m in page {
            match entry.messages.binary_search_by_key(&m.id, |x| x.id) {
                Ok(i) => entry.messages[i] = m,
                Err(i) => entry.messages.insert(i, m),
            }
        }
        // Loading older history is the one case where the window grows past the cap, so
        // scrolling up keeps working; trimming happens when new messages arrive instead.
    }

    pub fn mark_read(&mut self, channel: Snowflake) {
        if let Some(last) = self.channels.get(&channel).and_then(|c| c.last_message_id) {
            let state = self.read_states.entry(channel).or_default();
            state.last_read = Some(last);
            state.mentions = 0;
        }
    }

    fn remember_user(&mut self, user: &User) {
        match self.users.get(&user.id) {
            Some(existing) if existing == user => {}
            _ => {
                self.users.insert(user.id, user.clone());
            }
        }
    }

    fn insert_channel(&mut self, mut channel: Channel, guild: Option<Snowflake>) {
        if channel.guild_id.is_none() {
            channel.guild_id = guild;
        }
        for u in std::mem::take(&mut channel.recipients) {
            if !channel.recipient_ids.contains(&u.id) {
                channel.recipient_ids.push(u.id);
            }
            self.remember_user(&u);
        }
        let id = channel.id;
        match channel.guild_id {
            Some(g) => {
                if let Some(entry) = self.guilds.get_mut(&g)
                    && !entry.channel_ids.contains(&id)
                {
                    entry.channel_ids.push(id);
                }
            }
            None if channel.kind.is_private() && !self.private_channels.contains(&id) => {
                self.private_channels.push(id);
            }
            None => {}
        }
        self.channels.insert(id, channel);
    }

    fn sort_private_channels(&mut self) {
        let channels = &self.channels;
        self.private_channels.sort_by_key(|id| {
            std::cmp::Reverse(
                channels
                    .get(id)
                    .and_then(|c| c.last_message_id)
                    .unwrap_or(*id),
            )
        });
    }

    fn insert_guild(&mut self, mut guild: Guild) {
        let id = guild.id;
        let voice = std::mem::take(&mut guild.voice_states);
        let channels = std::mem::take(&mut guild.channels);
        let threads = std::mem::take(&mut guild.threads);
        let members = self
            .guilds
            .remove(&id)
            .map(|e| e.members)
            .unwrap_or_default();
        if !self.guild_order.contains(&id) {
            self.guild_order.push(id);
        }
        self.guilds.insert(
            id,
            GuildEntry {
                guild,
                channel_ids: Vec::with_capacity(channels.len()),
                members,
            },
        );
        for c in channels.into_iter().chain(threads) {
            self.insert_channel(c, Some(id));
        }
        if !voice.is_empty() {
            self.voice
                .retain(|_, v| v.guild_id != Some(id) || v.channel_id.is_none());
            for mut v in voice {
                v.guild_id.get_or_insert(id);
                self.insert_voice_state(v);
            }
        }
    }

    fn insert_member(&mut self, guild: Snowflake, mut member: Member) -> Option<Snowflake> {
        if let Some(u) = member.user.take() {
            member.user_id = Some(u.id);
            self.remember_user(&u);
        }
        let id = member.user_id?;
        self.guilds.get_mut(&guild)?.members.insert(id, member);
        Some(id)
    }

    fn insert_presence(&mut self, presence: Presence) -> Option<Snowflake> {
        let id = presence.user_id()?;
        self.presences.insert(id, presence);
        Some(id)
    }

    fn insert_voice_state(&mut self, mut state: VoiceState) {
        if let (Some(guild), Some(mut member)) = (state.guild_id, state.member.take()) {
            if member.user.is_none() {
                member.user_id.get_or_insert(state.user_id);
            }
            self.insert_member(guild, member);
        }
        if state.channel_id.is_some() {
            self.voice.insert(state.user_id, state);
        } else {
            self.voice.remove(&state.user_id);
        }
    }

    fn trim(&mut self, channel: Snowflake) {
        let cap = self.message_cap;
        if let Some(entry) = self.messages.get_mut(&channel) {
            let excess = entry.messages.len().saturating_sub(cap);
            if excess > 0 {
                entry.messages.drain(..excess);
                entry.reached_start = false;
            }
        }
    }

    fn update_reaction(&mut self, e: &ReactionEvent, added: bool) -> Change {
        let me = self.me_id() == Some(e.user_id);
        let Some(entry) = self.messages.get_mut(&e.channel_id) else {
            return Change::None;
        };
        let Ok(i) = entry.messages.binary_search_by_key(&e.message_id, |m| m.id) else {
            return Change::None;
        };
        let reactions = &mut entry.messages[i].reactions;
        let same = |r: &Reaction| match (r.emoji.id, e.emoji.id) {
            (Some(a), Some(b)) => a == b,
            (None, None) => r.emoji.name == e.emoji.name,
            _ => false,
        };
        let pos = reactions.iter().position(same);
        match (pos, added) {
            (Some(i), true) => {
                let r = &mut reactions[i];
                r.count += 1;
                if e.burst {
                    r.count_details.burst += 1;
                    r.me_burst |= me;
                } else {
                    r.count_details.normal += 1;
                    r.me |= me;
                }
            }
            (None, true) => reactions.push(Reaction {
                count: 1,
                count_details: if e.burst {
                    ReactionCounts {
                        burst: 1,
                        normal: 0,
                    }
                } else {
                    ReactionCounts {
                        burst: 0,
                        normal: 1,
                    }
                },
                me: me && !e.burst,
                me_burst: me && e.burst,
                emoji: e.emoji.clone(),
                burst_colors: Vec::new(),
            }),
            (Some(i), false) => {
                let r = &mut reactions[i];
                r.count = r.count.saturating_sub(1);
                if e.burst {
                    r.count_details.burst = r.count_details.burst.saturating_sub(1);
                    if me {
                        r.me_burst = false;
                    }
                } else {
                    r.count_details.normal = r.count_details.normal.saturating_sub(1);
                    if me {
                        r.me = false;
                    }
                }
                if r.count == 0 {
                    reactions.remove(i);
                }
            }
            (None, false) => return Change::None,
        }
        Change::Messages(e.channel_id)
    }

    fn apply_member_list(&mut self, update: MemberListUpdate) -> Change {
        let guild = update.guild_id;
        let key = (guild, update.id.clone());
        // Updates for another channel's list used to replace the one on screen, which left it
        // empty until the next full sync. Each list is kept on its own now.
        let mut list = self
            .member_lists
            .remove(&key)
            .unwrap_or_else(|| MemberList {
                id: update.id.clone(),
                ..MemberList::default()
            });
        if update
            .ops
            .iter()
            .any(|op| matches!(op, MemberListOp::Sync { .. }))
        {
            self.current_member_list.insert(guild, update.id.clone());
        } else {
            self.current_member_list
                .entry(guild)
                .or_insert_with(|| update.id.clone());
        }
        list.online = update.online_count;
        list.total = update.member_count;
        let convert = |store: &mut Self, item: MemberListItem| -> Option<MemberRow> {
            match item {
                MemberListItem::Group(g) => Some(MemberRow::Group {
                    id: g.id,
                    count: g.count,
                }),
                MemberListItem::Member(m) => {
                    let m = *m;
                    if let Some(p) = m.presence {
                        let mut p = p;
                        if p.user.is_none() && p.user_id.is_none() {
                            p.user_id = m.member.id();
                        }
                        store.insert_presence(p);
                    }
                    store.insert_member(guild, m.member).map(MemberRow::Member)
                }
            }
        };
        for op in update.ops {
            match op {
                MemberListOp::Sync { range, items } => {
                    let start = range[0] as usize;
                    let end = start + items.len();
                    if list.rows.len() < end {
                        list.rows.resize(end, None);
                    }
                    for (offset, item) in items.into_iter().enumerate() {
                        list.rows[start + offset] = convert(self, item);
                    }
                }
                MemberListOp::Insert { index, item } => {
                    let i = (index as usize).min(list.rows.len());
                    let row = convert(self, item);
                    list.rows.insert(i, row);
                }
                MemberListOp::Update { index, item } => {
                    let i = index as usize;
                    let row = convert(self, item);
                    if i < list.rows.len() {
                        list.rows[i] = row;
                    }
                }
                MemberListOp::Delete { index } => {
                    let i = index as usize;
                    if i < list.rows.len() {
                        list.rows.remove(i);
                    }
                }
                MemberListOp::Invalidate { range } => {
                    let end = (range[1] as usize + 1).min(list.rows.len());
                    for row in list.rows.iter_mut().take(end).skip(range[0] as usize) {
                        *row = None;
                    }
                }
            }
        }
        self.member_lists.insert(key, list);
        Change::Members(guild)
    }

    // ---- gateway ------------------------------------------------------------------------

    /// Applies one gateway update. `now_ms` is the current Unix time, used for typing.
    pub fn apply(&mut self, update: Update, now_ms: u64) -> Change {
        match update {
            Update::Ready(ready) => {
                let ready = *ready;
                *self = Self::new(self.message_cap);
                self.remember_user(&ready.user);
                self.me = Some(ready.user);
                for u in &ready.users {
                    self.remember_user(u);
                }
                let guild_ids: Vec<Snowflake> = ready.parsed_guilds.iter().map(|g| g.id).collect();
                for g in ready.parsed_guilds {
                    if !g.unavailable {
                        self.insert_guild(g);
                    }
                }
                for (gid, members) in guild_ids.iter().zip(ready.merged_members) {
                    for m in members {
                        self.insert_member(*gid, m);
                    }
                }
                for c in ready.private_channels {
                    self.insert_channel(c, None);
                }
                self.sort_private_channels();
                for r in ready.relationships {
                    if let Some(u) = &r.user {
                        self.remember_user(u);
                    }
                    self.relationships.insert(r.id, r);
                }
                for g in ready.user_guild_settings.into_vec() {
                    self.guild_settings
                        .insert(g.guild_id.map_or(0, Snowflake::get), g);
                }
                for e in ready.read_state.entries() {
                    if e.read_state_type == 0 {
                        self.read_states.insert(
                            e.id,
                            ReadState {
                                last_read: e.last_message_id,
                                mentions: e.mention_count,
                            },
                        );
                    }
                }
                Change::Everything
            }
            Update::ReadySupplemental(sup) => {
                let sup = *sup;
                for p in sup.merged_presences.friends {
                    self.insert_presence(p);
                }
                for list in sup.merged_presences.guilds {
                    for p in list {
                        self.insert_presence(p);
                    }
                }
                for (g, members) in sup.guilds.iter().zip(sup.merged_members) {
                    for m in members {
                        self.insert_member(g.id, m);
                    }
                }
                for g in sup.guilds {
                    for mut v in g.voice_states {
                        v.guild_id.get_or_insert(g.id);
                        self.insert_voice_state(v);
                    }
                }
                Change::Everything
            }
            Update::Resumed
            | Update::VoiceServerUpdate(_)
            | Update::StreamCreate(_)
            | Update::StreamUpdate(_)
            | Update::StreamServerUpdate(_)
            | Update::StreamDelete(_)
            | Update::Ignored => Change::None,
            Update::MessageCreate(message) => {
                let message = *message;
                let channel_id = message.channel_id;
                self.remember_user(&message.author);
                if let (Some(g), Some(m)) = (message.guild_id, message.member.clone())
                    && let Some(entry) = self.guilds.get_mut(&g)
                {
                    entry.members.insert(
                        message.author.id,
                        Member {
                            user_id: Some(message.author.id),
                            ..m
                        },
                    );
                }
                if let Some(c) = self.channels.get_mut(&channel_id) {
                    c.last_message_id =
                        Some(c.last_message_id.map_or(message.id, |l| l.max(message.id)));
                }
                if let Some(rcord_model::Nonce::Str(nonce)) = &message.nonce {
                    self.remove_pending(channel_id, nonce);
                }
                if let Some(list) = self.typing.get_mut(&channel_id) {
                    list.retain(|(u, _)| *u != message.author.id);
                }
                let me = self.me_id();
                if Some(message.author.id) == me {
                    self.mark_read(channel_id);
                } else if me.is_some_and(|me| message.mentions_user(me)) {
                    self.read_states.entry(channel_id).or_default().mentions += 1;
                }
                if self
                    .channels
                    .get(&channel_id)
                    .is_some_and(|c| c.kind.is_private())
                {
                    if let Some(pos) = self.private_channels.iter().position(|c| *c == channel_id) {
                        self.private_channels.remove(pos);
                    }
                    self.private_channels.insert(0, channel_id);
                }
                if let Some(entry) = self.messages.get_mut(&channel_id)
                    && entry.live
                {
                    match entry.messages.binary_search_by_key(&message.id, |m| m.id) {
                        Ok(i) => entry.messages[i] = message,
                        Err(i) => entry.messages.insert(i, message),
                    }
                    self.trim(channel_id);
                }
                Change::Messages(channel_id)
            }
            Update::MessageUpdate(patch) => {
                let patch = *patch;
                let ids = (
                    patch
                        .get("channel_id")
                        .and_then(|v| serde_json::from_value::<Snowflake>(v.clone()).ok()),
                    patch
                        .get("id")
                        .and_then(|v| serde_json::from_value::<Snowflake>(v.clone()).ok()),
                );
                let (Some(channel_id), Some(id)) = ids else {
                    return Change::None;
                };
                let Some(entry) = self.messages.get_mut(&channel_id) else {
                    return Change::None;
                };
                let Ok(i) = entry.messages.binary_search_by_key(&id, |m| m.id) else {
                    return Change::None;
                };
                let Ok(mut current) = serde_json::to_value(&entry.messages[i]) else {
                    return Change::None;
                };
                if let (Some(obj), serde_json::Value::Object(fields)) =
                    (current.as_object_mut(), patch)
                {
                    for (k, v) in fields {
                        obj.insert(k, v);
                    }
                }
                match serde_json::from_value(current) {
                    Ok(m) => {
                        entry.messages[i] = m;
                        Change::Messages(channel_id)
                    }
                    Err(e) => {
                        tracing::warn!("could not apply MESSAGE_UPDATE: {e}");
                        Change::None
                    }
                }
            }
            Update::MessageDelete { id, channel_id } => {
                if let Some(entry) = self.messages.get_mut(&channel_id)
                    && let Ok(i) = entry.messages.binary_search_by_key(&id, |m| m.id)
                {
                    entry.messages.remove(i);
                    return Change::Messages(channel_id);
                }
                Change::None
            }
            Update::MessageDeleteBulk { ids, channel_id } => {
                if let Some(entry) = self.messages.get_mut(&channel_id) {
                    entry.messages.retain(|m| !ids.contains(&m.id));
                }
                Change::Messages(channel_id)
            }
            Update::ReactionAdd(e) => self.update_reaction(&e, true),
            Update::ReactionRemove(e) => self.update_reaction(&e, false),
            Update::ReactionRemoveAll {
                channel_id,
                message_id,
            } => {
                if let Some(entry) = self.messages.get_mut(&channel_id)
                    && let Ok(i) = entry.messages.binary_search_by_key(&message_id, |m| m.id)
                {
                    entry.messages[i].reactions.clear();
                }
                Change::Messages(channel_id)
            }
            Update::MessageAck {
                channel_id,
                message_id,
                mention_count,
            } => {
                self.read_states.insert(
                    channel_id,
                    ReadState {
                        last_read: Some(message_id),
                        mentions: mention_count,
                    },
                );
                Change::ReadState(channel_id)
            }
            Update::GuildCreate(guild) => {
                let guild = *guild;
                let id = guild.id;
                self.insert_guild(guild);
                Change::Guild(id)
            }
            Update::GuildUpdate(patch) => {
                let patch = *patch;
                let Some(id) = patch
                    .get("id")
                    .and_then(|v| serde_json::from_value::<Snowflake>(v.clone()).ok())
                else {
                    return Change::None;
                };
                let Some(entry) = self.guilds.get_mut(&id) else {
                    return Change::None;
                };
                let Ok(mut current) = serde_json::to_value(&entry.guild) else {
                    return Change::None;
                };
                if let (Some(obj), serde_json::Value::Object(fields)) =
                    (current.as_object_mut(), patch)
                {
                    for (k, v) in fields {
                        obj.insert(k, v);
                    }
                }
                if let Ok(g) = Guild::from_gateway(current) {
                    entry.guild = g;
                }
                Change::Guild(id)
            }
            Update::GuildDelete { id, unavailable } => {
                if unavailable {
                    return Change::Guild(id);
                }
                if let Some(entry) = self.guilds.remove(&id) {
                    for c in entry.channel_ids {
                        self.channels.remove(&c);
                        self.messages.remove(&c);
                    }
                }
                self.guild_order.retain(|g| *g != id);
                self.member_lists.retain(|(g, _), _| *g != id);
                self.current_member_list.remove(&id);
                Change::Guilds
            }
            Update::GuildRoleUpsert { guild_id, role } => {
                if let Some(entry) = self.guilds.get_mut(&guild_id) {
                    match entry.guild.roles.iter_mut().find(|r| r.id == role.id) {
                        Some(r) => *r = role,
                        None => entry.guild.roles.push(role),
                    }
                }
                Change::Guild(guild_id)
            }
            Update::GuildRoleDelete { guild_id, role_id } => {
                if let Some(entry) = self.guilds.get_mut(&guild_id) {
                    entry.guild.roles.retain(|r| r.id != role_id);
                }
                Change::Guild(guild_id)
            }
            Update::GuildEmojisUpdate { guild_id, emojis } => {
                if let Some(entry) = self.guilds.get_mut(&guild_id) {
                    entry.guild.emojis = emojis;
                }
                Change::Guild(guild_id)
            }
            Update::GuildStickersUpdate { guild_id, stickers } => {
                if let Some(entry) = self.guilds.get_mut(&guild_id) {
                    entry.guild.stickers = stickers;
                }
                Change::Guild(guild_id)
            }
            Update::GuildMemberUpdate { guild_id, member } => {
                self.insert_member(guild_id, member);
                Change::Members(guild_id)
            }
            Update::GuildMemberListUpdate(update) => self.apply_member_list(*update),
            Update::ChannelCreate(channel) | Update::ChannelUpdate(channel) => {
                let channel = *channel;
                let guild = channel.guild_id;
                let id = channel.id;
                let last = self.channels.get(&id).and_then(|c| c.last_message_id);
                self.insert_channel(channel, guild);
                if let (Some(c), Some(last)) = (self.channels.get_mut(&id), last)
                    && c.last_message_id.is_none_or(|l| l < last)
                {
                    c.last_message_id = Some(last);
                }
                Change::Channels(guild)
            }
            Update::ChannelDelete { id, guild_id } => {
                self.channels.remove(&id);
                self.messages.remove(&id);
                self.private_channels.retain(|c| *c != id);
                if let Some(entry) = guild_id.and_then(|g| self.guilds.get_mut(&g)) {
                    entry.channel_ids.retain(|c| *c != id);
                }
                Change::Channels(guild_id)
            }
            Update::ThreadListSync { guild_id, threads } => {
                for t in threads {
                    self.insert_channel(t, Some(guild_id));
                }
                Change::Channels(Some(guild_id))
            }
            Update::GuildSettings(settings) => {
                let guild = settings.guild_id;
                self.guild_settings
                    .insert(guild.map_or(0, Snowflake::get), *settings);
                Change::Channels(guild)
            }
            Update::PresenceUpdate(presence) => match self.insert_presence(*presence) {
                Some(id) => Change::Presence(id),
                None => Change::None,
            },
            Update::TypingStart(t) => {
                if let (Some(g), Some(m)) = (t.guild_id, t.member) {
                    self.insert_member(g, m);
                }
                let until = t.timestamp.saturating_mul(1_000).max(now_ms) + TYPING_MS;
                let list = self.typing.entry(t.channel_id).or_default();
                list.retain(|(u, exp)| *u != t.user_id && *exp > now_ms);
                list.push((t.user_id, until));
                Change::Typing(t.channel_id)
            }
            Update::GuildMembersChunk(chunk) => {
                let chunk = *chunk;
                for m in chunk.members {
                    self.insert_member(chunk.guild_id, m);
                }
                Change::Voice(Some(chunk.guild_id))
            }
            Update::VoiceStateUpdate(state) => {
                let guild = state.guild_id;
                self.insert_voice_state(*state);
                Change::Voice(guild)
            }
            Update::UserUpdate(user) => {
                let user = *user;
                self.remember_user(&user);
                if self.me_id() == Some(user.id) {
                    self.me = Some(user);
                }
                Change::Me
            }
            Update::RelationshipAdd(r) => {
                let r = *r;
                if let Some(u) = &r.user {
                    self.remember_user(u);
                }
                self.relationships.insert(r.id, r);
                Change::Relationships
            }
            Update::RelationshipRemove { id } => {
                self.relationships.remove(&id);
                Change::Relationships
            }
        }
    }
}

#[cfg(test)]
mod tests;
