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
- **rcord-voice** has the voice gateway protocol (v8, including the DAVE opcodes), encryption
  mode selection, IP discovery and RTP headers. It is not wired to audio yet.
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
- **Pop-out windows** take the panel's view entity, unchanged, into a second GPUI window
  (`ui/panel_window.rs`); the panel views do not hold on to a window, so they draw in either.
  That window shares the main window's image cache through a global. Docking removes the
  window and the workspace animates the panel's width and outline back in.
- **Memory**: only the last eight channels opened keep their history; older ones load again
  when reopened. On Linux with glibc the allocator is held to two arenas and trimmed after
  READY and once a minute (`memory.rs`), since freed heap otherwise stays resident.
- **Settings** apply through one `update_settings` call that sanitizes, rebuilds the theme
  global, refreshes the window and schedules a save 400 ms later, so dragging a slider writes
  the file once.

## Voice, video and screen share

Calls are the largest piece still to build. The plan, in order:

1. Voice gateway session and UDP transport with `aead_aes256_gcm_rtpsize`, Opus encode and
   decode, a jitter buffer per speaker, and audio devices through `cpal`.
2. DAVE end-to-end encryption (MLS group state, per-sender frame keys), which Discord requires
   for calls.
3. Voice processing: voice activity detection, push to talk, noise suppression and echo
   cancellation, honoring the settings that already exist.
4. Video receive (H.264, VP8, AV1) decoded into GPUI textures, then camera and screen capture
   (PipeWire portals on Linux, Windows Graphics Capture on Windows) with hardware encoders
   where available.

Until step 1 works end to end, selecting a voice channel only explains that calls are not
available; rcord never sends a voice state update that would put the account in a channel it
cannot hear.
