//! Calls from the app's side: joining asks the main gateway for a voice session, and once both
//! halves of it have arrived (our voice state with its session ID, and the voice server with
//! its endpoint and token) the media connection starts on the network runtime. Its events
//! come back to the main thread and drive the voice bar and the speaking rings.

use std::collections::HashSet;
use std::time::Duration;

use gpui::{Context, Task};
use rcord_gateway::{Command, VoiceStateUpdate};
use rcord_model::Snowflake;
use rcord_settings::{InputMode, InputProfile, NoiseSuppression, Settings};
use rcord_store::{Change, Update};
use rcord_voice::{AudioSettings, ConnectionState, Denoise, Transmit, VoiceConfig, VoiceEvent};

use crate::net::Net;
use crate::state::{AppState, StateEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStatus {
    /// Waiting for Discord to assign a voice server.
    Joining,
    Connecting,
    Connected {
        encrypted: bool,
    },
    Reconnecting,
}

/// The call this client is in.
pub struct Call {
    pub guild: Option<Snowflake>,
    pub channel: Snowflake,
    pub status: CallStatus,
    /// Participants making sound right now; this client is included while it transmits.
    pub speaking: HashSet<Snowflake>,
    pub latency: Option<Duration>,
    /// Shown to compare with others in the call when it is end-to-end encrypted.
    pub privacy_code: Option<String>,
    pub input_level: f32,
    session_id: Option<String>,
    server: Option<(String, String)>,
    handle: Option<rcord_voice::VoiceHandle>,
    _events: Option<Task<()>>,
}

impl Call {
    /// The voice session ID, which stream connections identify with too.
    pub(crate) fn session_id(&self) -> Option<String> {
        self.session_id.clone()
    }

    /// Where a watched stream's sound is mixed in.
    pub(crate) fn audio_sink(
        &self,
    ) -> Option<std::sync::mpsc::Sender<rcord_voice::audio::Inbound>> {
        self.handle.as_ref().map(|h| h.audio_sink())
    }

    fn new(guild: Option<Snowflake>, channel: Snowflake) -> Self {
        Self {
            guild,
            channel,
            status: CallStatus::Joining,
            speaking: HashSet::new(),
            latency: None,
            privacy_code: None,
            input_level: -100.0,
            session_id: None,
            server: None,
            handle: None,
            _events: None,
        }
    }
}

/// The audio settings the media loop needs, from the user's settings and the channel's
/// bitrate.
/// Sets the saved volume and local mute of everyone who has one.
fn apply_user_volumes(settings: &Settings, handle: &rcord_voice::VoiceHandle) {
    let v = &settings.voice;
    let ids = v.user_volumes.keys().chain(v.muted_users.iter());
    for id in ids {
        if let Ok(user) = id.parse::<u64>() {
            handle.set_user_volume(user, v.user_volume(id));
        }
    }
}

pub fn audio_settings(settings: &Settings, channel_bitrate: Option<u32>) -> AudioSettings {
    let v = &settings.voice;
    let cap = v.max_bitrate_kbps.max(8);
    AudioSettings {
        input_device: v.input_device.clone(),
        output_device: v.output_device.clone(),
        input_gain: v.input_volume,
        output_gain: v.output_volume,
        transmit: match (v.input_mode, v.input_profile) {
            (InputMode::PushToTalk, _) => Transmit::PushToTalk {
                release_ms: v.push_to_talk_release_ms,
            },
            // An open mic, closed only by digital silence.
            (InputMode::VoiceActivity, InputProfile::Studio) => Transmit::VoiceActivity {
                threshold_db: Some(-90.0),
            },
            (InputMode::VoiceActivity, InputProfile::VoiceIsolation) => {
                Transmit::VoiceActivity { threshold_db: None }
            }
            (InputMode::VoiceActivity, InputProfile::Custom) => Transmit::VoiceActivity {
                threshold_db: (!v.auto_sensitivity).then_some(v.sensitivity_db),
            },
        },
        noise_suppression: match (v.input_profile, v.noise_suppression) {
            (InputProfile::VoiceIsolation, _) => Denoise::Strong,
            (InputProfile::Studio, _) | (InputProfile::Custom, NoiseSuppression::Off) => {
                Denoise::Off
            }
            (InputProfile::Custom, NoiseSuppression::Standard) => Denoise::Standard,
            (InputProfile::Custom, NoiseSuppression::Strong) => Denoise::Strong,
        },
        automatic_gain: match v.input_profile {
            InputProfile::VoiceIsolation => true,
            InputProfile::Studio => false,
            InputProfile::Custom => v.automatic_gain,
        },
        bitrate_kbps: channel_bitrate.map_or(cap, |b| (b / 1000).clamp(8, cap)),
    }
}

impl AppState {
    pub fn join_voice(&mut self, channel: Snowflake, cx: &mut Context<Self>) {
        let Some(gw) = self.gateway.clone() else {
            return;
        };
        if self.call.as_ref().is_some_and(|c| c.channel == channel) {
            return;
        }
        let guild = self.store.channel(channel).and_then(|c| c.guild_id);
        // Switching channels drops the old media connection; Discord moves the voice state.
        self.call = Some(Call::new(guild, channel));
        gw.send(Command::UpdateVoiceState(VoiceStateUpdate {
            guild_id: guild,
            channel_id: Some(channel),
            self_mute: self.self_mute,
            self_deaf: self.self_deaf,
            self_video: false,
        }));
        cx.emit(StateEvent::Store(Change::Voice(guild)));
        cx.notify();
    }

    pub fn leave_voice(&mut self, cx: &mut Context<Self>) {
        let guild = self
            .call
            .as_ref()
            .map(|c| c.guild)
            .or_else(|| self.store.my_voice_state().map(|v| v.guild_id))
            .flatten();
        self.end_streams(cx);
        self.call = None;
        self.forget_my_voice_state(guild);
        if let Some(gw) = &self.gateway {
            gw.send(Command::UpdateVoiceState(VoiceStateUpdate {
                guild_id: guild,
                channel_id: None,
                self_mute: self.self_mute,
                self_deaf: self.self_deaf,
                self_video: false,
            }));
        }
        cx.emit(StateEvent::Store(Change::Voice(guild)));
        cx.notify();
    }

    pub fn set_mute(&mut self, mute: bool, deaf: bool, cx: &mut Context<Self>) {
        self.self_mute = mute || deaf;
        self.self_deaf = deaf;
        if let Some(h) = self.call.as_ref().and_then(|c| c.handle.as_ref()) {
            h.set_muted(self.self_mute);
            h.set_deafened(self.self_deaf);
        }
        if let (Some(gw), Some(v)) = (&self.gateway, self.store.my_voice_state()) {
            gw.send(Command::UpdateVoiceState(VoiceStateUpdate {
                guild_id: v.guild_id,
                channel_id: v.channel_id,
                self_mute: self.self_mute,
                self_deaf: self.self_deaf,
                self_video: false,
            }));
        }
        cx.emit(StateEvent::Store(Change::Me));
        cx.notify();
    }

    /// Takes this account out of the channel list at once. Discord confirms with its own
    /// voice state update, but the list should not wait on it, or keep showing us if it is
    /// lost.
    fn forget_my_voice_state(&mut self, guild: Option<Snowflake>) {
        let Some(me) = self.store.me_id() else {
            return;
        };
        let left = rcord_model::VoiceState {
            guild_id: guild,
            channel_id: None,
            user_id: me,
            session_id: String::new(),
            member: None,
            deaf: false,
            mute: false,
            self_deaf: false,
            self_mute: false,
            self_stream: false,
            self_video: false,
            suppress: false,
        };
        self.store.apply(
            Update::VoiceStateUpdate(Box::new(left)),
            crate::state::now_ms(),
        );
    }

    /// Asks the gateway for voice participants whose profiles are not cached, so the
    /// channel list can show their names and avatars.
    pub(crate) fn request_voice_members(&mut self) {
        let Some(gw) = &self.gateway else {
            return;
        };
        let mut wanted: std::collections::BTreeMap<Snowflake, Vec<Snowflake>> = Default::default();
        for v in self.store.voice_states() {
            if let Some(g) = v.guild_id
                && self.store.user(v.user_id).is_none()
                && self.voice_members_requested.insert(v.user_id)
            {
                wanted.entry(g).or_default().push(v.user_id);
            }
        }
        for (guild, users) in wanted {
            for chunk in users.chunks(100) {
                gw.send(Command::RequestGuildMembers(
                    rcord_gateway::RequestGuildMembers {
                        guild_id: vec![guild],
                        query: None,
                        user_ids: Some(chunk.to_vec()),
                        limit: 0,
                        presences: false,
                    },
                ));
            }
        }
    }

    /// Passes changed voice settings to a running call.
    /// Sets how loud `user` plays for you, 0 to 2, and saves it.
    pub fn set_user_volume(&mut self, user: Snowflake, volume: f32, cx: &mut Context<Self>) {
        let id = user.to_string();
        let volume = volume.clamp(0.0, 2.0);
        let v = &mut self.settings.voice;
        if (volume - 1.0).abs() > 0.005 {
            v.user_volumes.insert(id, volume);
        } else {
            v.user_volumes.remove(&id);
        }
        self.user_audio_changed(user, cx);
    }

    /// Mutes or unmutes `user` for you alone, and saves it.
    pub fn set_local_mute(&mut self, user: Snowflake, muted: bool, cx: &mut Context<Self>) {
        let id = user.to_string();
        let list = &mut self.settings.voice.muted_users;
        list.retain(|u| *u != id);
        if muted {
            list.push(id);
            list.sort();
        }
        self.user_audio_changed(user, cx);
    }

    /// Applies one person's volume to the call and saves it. This skips the full settings
    /// update, since a volume slider sends many changes a second.
    fn user_audio_changed(&mut self, user: Snowflake, cx: &mut Context<Self>) {
        if let Some(h) = self.call.as_ref().and_then(|c| c.handle.as_ref()) {
            h.set_user_volume(
                user.get(),
                self.settings.voice.user_volume(&user.to_string()),
            );
        }
        self.save_settings(cx);
        cx.notify();
    }

    /// Server mutes, server deafens, moves or disconnects `user`, as a moderator.
    pub fn moderate_voice(
        &mut self,
        guild: Snowflake,
        user: Snowflake,
        change: rcord_rest::routes::MemberVoiceChange,
        cx: &mut Context<Self>,
    ) {
        self.member_request(cx, "update their voice", move |rest| async move {
            rest.modify_member_voice(guild, user, change).await
        });
    }

    pub fn kick_member(&mut self, guild: Snowflake, user: Snowflake, cx: &mut Context<Self>) {
        self.member_request(cx, "kick them", move |rest| async move {
            rest.kick_member(guild, user).await
        });
    }

    pub fn ban_member(&mut self, guild: Snowflake, user: Snowflake, cx: &mut Context<Self>) {
        self.member_request(cx, "ban them", move |rest| async move {
            rest.ban_member(guild, user).await
        });
    }

    /// Runs a moderation request and reports a failure, such as a missing permission or a
    /// member whose role is above yours.
    fn member_request<F, Fut>(&mut self, cx: &mut Context<Self>, what: &'static str, f: F)
    where
        F: FnOnce(rcord_rest::Client) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = rcord_rest::Result<()>> + Send + 'static,
    {
        let Some(rest) = cx.global::<Net>().rest() else {
            return;
        };
        let task = cx.global::<Net>().run(async move { f(rest).await });
        cx.spawn(async move |this, cx| {
            if let Some(Err(e)) = task.await {
                this.update(cx, |s, cx| s.toast(format!("Could not {what}: {e}"), cx))
                    .ok();
            }
        })
        .detach();
    }

    pub fn voice_settings_changed(&mut self) {
        let Some(call) = &self.call else {
            return;
        };
        let Some(h) = &call.handle else {
            return;
        };
        let bitrate = self.store.channel(call.channel).and_then(|c| c.bitrate);
        h.update_audio(audio_settings(&self.settings, bitrate));
        apply_user_volumes(&self.settings, h);
    }

    /// Looks at gateway updates that matter to the call before the cache applies them.
    pub(crate) fn voice_gateway_update(&mut self, update: &Update, cx: &mut Context<Self>) {
        let Some(call) = &mut self.call else {
            return;
        };
        match update {
            Update::VoiceStateUpdate(v) if Some(v.user_id) == self.store.me_id() => {
                match v.channel_id {
                    // Kicked, or left from another device.
                    None if call.status != CallStatus::Joining || call.session_id.is_some() => {
                        self.end_streams(cx);
                        self.call = None;
                        cx.notify();
                        return;
                    }
                    None => return,
                    Some(ch) if ch != call.channel => {
                        if call.session_id.is_none() {
                            return; // An older state from before our join.
                        }
                        // Moved by a moderator: follow, and wait for the new server.
                        call.channel = ch;
                        call.guild = v.guild_id;
                        call.handle = None;
                        call._events = None;
                        call.server = None;
                        call.status = CallStatus::Joining;
                    }
                    Some(_) => {}
                }
                if call.session_id.as_deref() != Some(v.session_id.as_str()) {
                    call.session_id = Some(v.session_id.clone());
                }
            }
            Update::VoiceServerUpdate(s) => {
                let target = s.guild_id.or(s.channel_id);
                if target != call.guild.or(Some(call.channel)) {
                    return;
                }
                // A null endpoint means the server is moving; a new update follows.
                let Some(endpoint) = s.endpoint.clone() else {
                    call.handle = None;
                    call._events = None;
                    call.status = CallStatus::Connecting;
                    cx.notify();
                    return;
                };
                call.server = Some((endpoint, s.token.clone()));
                call.handle = None;
                call._events = None;
            }
            _ => return,
        }
        self.start_media(cx);
    }

    fn start_media(&mut self, cx: &mut Context<Self>) {
        let Some(me) = self.store.me_id() else {
            return;
        };
        let bitrate = self
            .call
            .as_ref()
            .and_then(|c| self.store.channel(c.channel))
            .and_then(|c| c.bitrate);
        let audio = audio_settings(&self.settings, bitrate);
        let (mute, deaf) = (self.self_mute, self.self_deaf);
        let Some(call) = &mut self.call else {
            return;
        };
        if call.handle.is_some() {
            return;
        }
        let (Some(session_id), Some((endpoint, token))) = (&call.session_id, &call.server) else {
            return;
        };
        let config = VoiceConfig {
            endpoint: endpoint.clone(),
            server_id: call.guild.unwrap_or(call.channel).get(),
            channel_id: call.channel.get(),
            user_id: me.get(),
            session_id: session_id.clone(),
            token: token.clone(),
            audio,
            self_mute: mute,
            self_deaf: deaf,
        };
        let net = cx.global::<Net>();
        let started = {
            let _guard = net.handle().enter();
            rcord_voice::connect(config)
        };
        let (handle, events) = match started {
            Ok(v) => v,
            Err(e) => {
                self.toast(format!("Voice could not start: {e}"), cx);
                return;
            }
        };
        let channel = call.channel;
        apply_user_volumes(&self.settings, &handle);
        call.handle = Some(handle);
        call.status = CallStatus::Connecting;
        call._events = Some(cx.spawn(async move |this, cx| {
            while let Ok(first) = events.recv().await {
                let ok = this.update(cx, |state, cx| {
                    state.voice_event(channel, first, me, cx);
                    while let Ok(next) = events.try_recv() {
                        state.voice_event(channel, next, me, cx);
                    }
                });
                if ok.is_err() {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn voice_event(
        &mut self,
        channel: Snowflake,
        event: VoiceEvent,
        me: Snowflake,
        cx: &mut Context<Self>,
    ) {
        let Some(call) = self.call.as_mut().filter(|c| c.channel == channel) else {
            return;
        };
        match event {
            VoiceEvent::State(s) => {
                call.status = match s {
                    ConnectionState::Connecting => CallStatus::Connecting,
                    ConnectionState::Connected { encrypted } => CallStatus::Connected { encrypted },
                    ConnectionState::Reconnecting => CallStatus::Reconnecting,
                };
            }
            VoiceEvent::Speaking { user_id, speaking } => {
                let user = if user_id == 0 {
                    Some(me)
                } else {
                    Snowflake::new(user_id)
                };
                if let Some(user) = user {
                    let changed = if speaking {
                        call.speaking.insert(user)
                    } else {
                        call.speaking.remove(&user)
                    };
                    if !changed {
                        return;
                    }
                    let guild = call.guild;
                    cx.emit(StateEvent::Store(Change::Voice(guild)));
                }
            }
            VoiceEvent::Latency(d) => call.latency = Some(d),
            VoiceEvent::PrivacyCode(code) => call.privacy_code = code,
            VoiceEvent::InputLevel(l) => {
                call.input_level = l;
                return; // Only the settings meter reads this; no redraw needed here.
            }
            VoiceEvent::DeviceError(e) => self.toast(e, cx),
            VoiceEvent::VideoFrame => return,
            VoiceEvent::Closed { code, reason } => {
                tracing::info!("voice closed ({code:?}): {reason}");
                let guild = call.guild;
                self.end_streams(cx);
                self.call = None;
                self.forget_my_voice_state(guild);
                if reason != "Disconnected" {
                    self.toast(reason, cx);
                }
                // Make sure no other client sees us sitting in a channel we cannot hear.
                if let Some(gw) = &self.gateway {
                    gw.send(Command::UpdateVoiceState(VoiceStateUpdate {
                        guild_id: guild,
                        channel_id: None,
                        self_mute: self.self_mute,
                        self_deaf: self.self_deaf,
                        self_video: false,
                    }));
                }
                cx.emit(StateEvent::Store(Change::Voice(guild)));
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_settings_follow_the_channel_bitrate() {
        let mut s = Settings::default();
        s.voice.max_bitrate_kbps = 96;
        s.voice.input_profile = InputProfile::Custom;
        assert_eq!(audio_settings(&s, Some(64_000)).bitrate_kbps, 64);
        assert_eq!(audio_settings(&s, Some(384_000)).bitrate_kbps, 96);
        assert_eq!(audio_settings(&s, None).bitrate_kbps, 96);
        s.voice.auto_sensitivity = false;
        s.voice.sensitivity_db = -40.0;
        assert_eq!(
            audio_settings(&s, None).transmit,
            Transmit::VoiceActivity {
                threshold_db: Some(-40.0)
            }
        );
        s.voice.noise_suppression = NoiseSuppression::Off;
        assert_eq!(audio_settings(&s, None).noise_suppression, Denoise::Off);
        s.voice.input_profile = InputProfile::VoiceIsolation;
        let iso = audio_settings(&s, None);
        assert_eq!(iso.noise_suppression, Denoise::Strong);
        assert!(iso.automatic_gain);
        s.voice.input_profile = InputProfile::Studio;
        let studio = audio_settings(&s, None);
        assert_eq!(studio.noise_suppression, Denoise::Off);
        assert!(!studio.automatic_gain);
    }
}
