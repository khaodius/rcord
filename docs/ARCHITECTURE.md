# Architecture

rcord is one process with two kinds of threads. GPUI owns the main thread and draws every
window. A small Tokio runtime with two worker threads owns every socket: the gateway, the HTTP
API and image downloads. Nothing on the main thread waits on the network; results come back
through channels and are applied between frames.

```
             ┌───────────────────────── network runtime (Tokio) ─────────────────────────┐
             │                                                                           │
 Discord ◀──▶│ rcord-gateway: websocket, zlib-stream, heartbeat, resume                  │
 gateway     │        │ raw dispatches (name + undecoded JSON)                           │
             │        ▼                                                                  │
             │ parse task: rcord-store::parse → typed Update                             │
             │        │ async channel                                                    │
             └────────┼──────────────────────────────────────────────────────────────────┘
                      ▼
             ┌──────────────────────── main thread (GPUI) ───────────────────────────────┐
             │ pump: drains up to 512 updates per wake, applies each to the Store,       │
             │ emits one StateEvent per change                                           │
             │        │                                                                  │
             │        ▼                                                                  │
             │ AppState (entity) ── events ──▶ ServerRail · ChannelSidebar · ChatView ·  │
             │                                 MemberList · OverlayLayer · SettingsView │
             └───────────────────────────────────────────────────────────────────────────┘
```

## Crates

- **rcord-model** holds Discord's objects (users, guilds, channels, messages, emoji, presences)
  as serde types that tolerate fields being added or missing, plus permission math and every
  CDN URL the client builds.
- **rcord-gateway** connects with `compress=zlib-stream` and keeps one inflate context for the
  whole connection, as the protocol requires. It identifies as a user client with the
  capabilities the official client sends, so READY arrives deduplicated (users listed once,
  members by ID) and read states versioned. Heartbeats carry jitter, a missed acknowledgement
  is treated as a dead connection, and a drop resumes the session where Discord allows it,
  otherwise it identifies again with backoff. Close codes that mean the token is bad stop
  reconnecting and send the user back to sign-in.
- **rcord-rest** wraps the HTTP API. Every request carries the same client properties as the
  gateway. Rate limits are tracked per route bucket, keyed the way Discord keys them (the
  channel, guild or webhook ID stays in the key, other IDs do not), so a burst of reactions
  in one channel never slows another. 429 responses wait out `retry_after`, global or not.
- **rcord-store** is the cache. Gateway updates and API pages go in through one `apply`
  function that returns what changed, so views redraw only for changes they show. Each channel
  keeps a bounded window of messages; scrolling up loads older pages and the far end is
  dropped. The member list mirrors the gateway's lazy list (`SYNC`, `INSERT`, `UPDATE`,
  `DELETE`, `INVALIDATE`) and the view asks for more ranges as it scrolls.
- **rcord-markup** parses Discord's markdown into blocks and inlines without regular
  expressions, with the same edge cases as Discord: nested emphasis, escapes, spoilers,
  `<url>` links that do not embed, mentions and `<t:…:R>` timestamps.
- **rcord-auth** implements the remote-auth handshake behind the QR code: a fresh RSA-2048 key,
  a proof of the decrypted nonce, the fingerprint shown as the code, then the encrypted user
  preview and ticket once the phone scans and approves. Tokens are stored with the platform
  keychain and read back at start.
- **rcord-settings** holds every setting with defaults and clamps. Unknown or missing keys
  never fail a load, so a settings file from another version still opens; saves are atomic.
- **rcord-voice** carries calls: the voice gateway (v8), DAVE end-to-end encryption, UDP media
  with transport encryption, and the audio path between the devices and Opus. See below.
- **rcord** is the application.

## The application

`AppState` is one GPUI entity that owns the session, the Store, the selection and the overlay
stack. Views hold an `Entity<AppState>` and subscribe to `StateEvent`, which carries the
store's `Change`: a new message in one channel redraws the chat if that channel is open and
the unread markers in the sidebar, and nothing else.

- **Lists are virtual.** The chat and the member list use GPUI's `list`, which measures and
  paints only the rows in view. The chat's row model (day separators, grouped messages,
  pending sends) is diffed on every change and spliced, so a new message lays out one row.
- **Panels are cached views.** Server, channel and member panels are drawn as cached views, so
  typing in the composer redraws the chat column only.
- **Images** load through an HTTP client that serves Discord's CDN from a content-addressed
  disk cache first; CDN URLs change whenever the image does, so cached files never go stale.
  Decoded images live in an LRU cache with a byte budget instead of GPUI's keep-everything
  default, which matters in a client that scrolls past thousands of avatars.
- **Panel moving** is handled by the workspace in the capture phase: a right press starts a
  timer, and if the button is still down after a moment, or the pointer moved a few pixels,
  the panel lifts. The release drops it at the nearest gap and is consumed, so the panel
  underneath never sees it as a context-menu click. A quick right click passes through
  untouched.
- **Settings** apply through one `update_settings` call that sanitizes, rebuilds the theme
  global, refreshes the window and schedules a save 400 ms later, so dragging a slider writes
  the file once.

## Voice

Joining a voice channel sends a voice state update on the main gateway. Discord answers with
two dispatches: our own `VOICE_STATE_UPDATE`, which carries the voice session ID, and
`VOICE_SERVER_UPDATE`, with the media server's endpoint and a token. `AppState` holds the call
until both have arrived, then `rcord_voice::connect` starts it on the network runtime.

```
 main thread            network runtime (Tokio)                  rcord-audio thread
 ───────────            ───────────────────────                  ──────────────────
 AppState.call ◀─events─ voice gateway (WebSocket, JSON+binary)
                        │ Hello → Identify → Ready
                        │ UDP IP discovery → Select Protocol
                        │ Session Description: key, DAVE version
                        │ DAVE MLS: external sender, key package,
                        │ proposals, commit/welcome, transitions
                        ▼
                        UDP socket ── RTP ──▶ open (AEAD) ──▶ DAVE decrypt ──▶ jitter buffer per SSRC
                                                                              Opus decode / FEC / PLC
                                                                              mix, gain, limiter
                                                                              resample ──▶ speakers
                        UDP socket ◀── seal (AEAD) ◀── DAVE encrypt ◀── Opus encode (20 ms, FEC)
                                                                              voice activity / PTT
                                                                              RNNoise, gain
                                                                     microphone ──▶ resample to 48 kHz
```

- **Voice gateway.** Heartbeats acknowledge the last sequence number (`seq_ack`), as v8
  requires. A dropped socket resumes with that sequence and keeps the UDP socket, the keys and
  the MLS group, so a blip does not interrupt audio. Close codes that end the session (kicked,
  moved, call ended, session expired) end the call with a readable reason, and rcord sends a
  voice state update that leaves the channel, so it never sits in a channel it cannot hear.
- **Transport encryption.** `aead_aes256_gcm_rtpsize` is preferred, then
  `aead_xchacha20_poly1305_rtpsize`. The RTP header (and an extension's four-byte header) is the
  additional authenticated data; a 32-bit counter appended to each packet is the nonce. RTCP on
  the same socket is ignored.
- **DAVE.** The MLS group is managed with `davey`. rcord offers protocol version 1, sends its key
  package when the session starts, commits proposals for members the voice server announced
  (and only those), joins from welcomes, acknowledges every prepared transition, and switches
  keys when the server executes it. A commit or welcome that fails is reported and rcord rejoins
  with a fresh key package. Each Opus frame is encrypted with the sender's ratcheted key before
  transport encryption; Opus silence frames pass through as the protocol specifies. The
  privacy code is shown in the voice bar's tooltip.
- **Audio.** Device callbacks only move samples into and out of two queues. The `rcord-audio`
  thread does everything else every 5 ms. The microphone is downmixed to mono, resampled to
  48 kHz with a cubic resampler when the device runs at another rate, scaled by the input
  volume and cleaned by RNNoise. The gate transmits on voice activity (RNNoise's voice
  probability and a threshold that tracks the noise floor, or the fixed threshold from
  Settings) or push to talk, with a hangover so words are not clipped. Frames are encoded as
  20 ms Opus at the channel's bitrate (capped by Settings) with in-band FEC, and five silence
  frames follow the end of speech. Playback produces a frame whenever the device queue drops
  under 60 ms, so it follows the device's clock. Each speaker has a jitter buffer (three frames
  deep, bounded at 25) and a decoder; a missing frame is rebuilt from the next packet's FEC or
  concealed. Mixed audio passes a soft limiter rather than clipping.
- **Devices** are opened at 48 kHz when they support it. On Linux the PulseAudio backend (also
  served by PipeWire) is used when available, with ALSA as the fallback. A lost device is
  reopened every two seconds until it comes back; changing the device in Settings reopens it.

### Still to build

1. Push to talk with a global key, echo cancellation, per-user volume in the participant menu.
2. Go Live: `STREAM_CREATE`/`STREAM_WATCH` on the main gateway, a second voice connection to
   the stream server with video SSRCs, H.264 RTP packetization, then capture (PipeWire portal
   on Wayland, XComposite or the portal on X11, Windows Graphics Capture) with hardware encoders
   where available, and decoding into GPUI textures for watching.
3. Camera video on the same path.
