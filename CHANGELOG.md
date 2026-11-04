# Changelog

Packages for every version are attached to its
[release](https://github.com/khaodius/rcord/releases), with SHA-256 checksums.

## Unreleased

- **Screen share (Go Live)**: the screen button in the voice bar shares your screen into the
  voice channel. Pick a screen and a quality (720p or 1080p, 30 or 60 FPS); people in the
  channel see a LIVE badge next to your name and can watch. On Wayland, rcord uses your
  desktop's own sharing dialog, so you can pick a screen or a single window, and captures
  natively through PipeWire rather than XWayland. Video is H.264 and end-to-end encrypted like
  voice.
- **Watch streams**: click the LIVE badge next to someone in a voice channel to watch their
  stream above the chat, with their sound mixed into the call. Lost packets are recovered by
  asking the streamer for a fresh keyframe.

## 0.1.6 - 2026-11-04

- **Voice channels show who is already in them** when rcord starts, instead of only people who
  joined afterwards. Participants rcord has no profile for yet are looked up.
- **Leaving a call** takes you out of the channel list right away.
- **Voice channels no longer show unread dots or mention counts**; their text chat cannot be
  opened in rcord yet, so the markers could never be cleared.
- **Settings > Voice** is laid out like Discord's: input and output device pickers that list
  your devices, a mic test that plays your voice back exactly as a call sends it with a live
  level meter, and input profiles. Voice Isolation (strong noise suppression, automatic gain
  and sensitivity), Studio (no processing) and Custom, which shows every control: input mode,
  sensitivity with a threshold bar, noise suppression (None, Standard, Strong) and automatic
  gain control. Bitrate and other rarely changed settings are under Show Advanced Voice
  Settings.
- **Automatic gain control** now works, and **Strong** noise suppression also cuts keyboard
  clicks and noises between words.

## 0.1.5 - 2026-11-04

- **Videos play in rcord**: clicking an MP4 or MOV video in chat opens a player over the window
  instead of the browser. H.264 video (what phones, screen recorders and editors write) is
  decoded with Cisco's OpenH264 and AAC sound with Symphonia, timed to the sound card so
  picture and sound stay together. Play and pause, a seek bar you can click or drag, volume and
  mute, and Fill the window. Keys: Space or K plays and pauses, Left and Right jump 5 seconds,
  M mutes, F fills the window, Esc closes. Other formats (WebM, HEVC) say so and offer the
  browser.
- **Fixed a crash when scrolling** quickly through long channels. Some messages with emoji or
  accented letters right after a formatting marker crashed the markup parser. If rcord ever
  crashes now, the reason is written to `crash.log` in the config folder.
- **Pop out by dragging**: drag a panel past the edge of the window and it leaves in its own
  window right away, with the same animation as dropping it on the target.
- **rcord title bar**: a slim bar with minimize, maximize and close, for desktops that draw no
  title bar (on by default on Linux). Drag it to move the window, double-click to maximize,
  and drag the window edges to resize. Toggle it in Settings > Appearance > Window; it works on
  Windows too. Popped-out panels get the same edges and a draggable header.
- **Close to tray works**: with it on, closing the window leaves rcord in the system tray
  (StatusNotifier on Linux, the notification area on Windows). Click the icon or Open rcord to
  bring it back, or Quit rcord from its menu.
- **Server list**: drag servers to put them in your own order, which is saved. Server icons are
  round.
- **Home button**: choose what the top-left button shows in Settings > Appearance > Panels: the
  rcord logo, the friends icon, your avatar or a picture of your own. Clicking it while on Home
  opens Friends.

## 0.1.4 - 2026-11-04

- **Voice channels and calls**: join a voice channel and talk. Audio is Opus at the channel's
  bitrate with forward error correction, end-to-end encrypted with Discord's DAVE protocol (the
  voice bar shows the lock and, on hover, the call's privacy code and ping). Noise suppression,
  voice activity that adapts to the room or uses your fixed threshold, input and output volume
  and the devices chosen in Settings > Voice all apply live. People speaking get a green ring in
  the channel list. A dropped voice connection resumes without leaving the call.

## 0.1.3 - 2026-11-04

- **Friends page**: Home shows your friends instead of "Pick a conversation". Tabs for Online,
  All, Pending (with a count of requests to answer) and Blocked, a search, Message and profile
  buttons on each friend, accept and ignore on requests, and Add Friend by username. The
  Active Now column shows what friends are playing or listening to and which voice channel
  they are in. The Friends button in the DM list now opens it.
- **Pop-out panels**: drop the server list, channel list or member list on the target at the
  top of the window, or outside the window, and it opens in its own window. Closing that window
  or pressing Dock puts it back, sliding open with a glow around where it landed.
- **Custom themes**: create a theme from the current one and change any color, the corner radii
  and the fonts in Settings > Appearance. Themes export to and import from TOML files, and
  files in the themes folder are listed for one-click switching. docs/THEMES.md describes the
  format.
- **Appearance**: a live message preview at the top of Appearance follows every change to
  scaling, message display and avatars in compact mode. The interface and code fonts are picked
  from the fonts installed on your computer. The theme picker shows a single sphere per theme.
- **Animated emoji, GIFs, avatars and server icons** play again. They showed only their first
  frame.
- **Member list** no longer goes empty at random. Discord keeps one list per set of channel
  permissions, and rcord could show a different list than the one it had asked for.
- **Loading older messages** no longer puts the new page below the loading placeholders when
  you keep scrolling up while it loads; the view stays on the message you were reading.
- **Moving panels**: the outline on a lifted panel's own spot only shows while the pointer is
  back over it.
- **Memory**: decoded images default to 96 MB instead of 256 MB (settings still on the old
  default move to the new one), only the last eight channels opened keep their history in
  memory, and on Linux freed memory is handed back to the system after sign-in and once a
  minute.
- New app icon, and a gear for Settings.
- The up arrow, sign-out and shield icons, which were blank, now draw.

## 0.1.2 - 2026-11-04

- **Sign-in fixed**: after a successful sign-in rcord failed with "duplicate field
  `primary_guild`", because Discord sends the server tag under its old and new names at once.
  Both are read now, and an object in a shape rcord does not expect only skips that object
  instead of stopping the connection.
- **Updates**: at each start rcord checks for a newer release and asks to install it. Accepting
  downloads it, checks it against the release checksums, installs it without any installer
  screens and restarts rcord. On Linux, .deb and .rpm installs ask for your password the way
  system updates do. Turn the check off in Settings > Advanced.

## 0.1.1 - 2026-11-04

- **Email or phone and password sign-in**, laid out like Discord's login screen, with
  two-factor codes from an authenticator app, by SMS or from a backup code, and password
  reset links.
- **Captchas**: when Discord asks for one, during a password or QR code sign-in, it opens in a
  small Discord window and the sign-in carries on once it is solved. QR code sign-ins that
  failed with "400 Bad Request" were Discord asking for a captcha rcord could not show.
- **Sign in on Discord's page**, in that same window, for anything else Discord asks during a
  sign-in. Token sign-in moved to a link below the form.
- Discord's error messages are shown as Discord words them, such as "Login or password is
  invalid.", instead of the HTTP status.
- rcord now identifies itself to Discord the way Discord's web client does, matching the build
  number it reads from the web client.

## 0.1.0 - 2026-11-03

First release.

- **Servers, channels and DMs** in Discord's layout, with categories that collapse, unread
  markers and mention counts synced with your other devices, and a quick switcher (Ctrl + K).
- **Messages** with Discord's full markdown, replies, attachments, link embeds, GIFs, stickers,
  custom and animated emoji, and reactions you can add and remove. Long channels scroll
  smoothly: only the rows on screen are laid out, and old pages reload as you scroll back.
- **Expression picker** for emoji, stickers and GIF search.
- **Profiles** with banners, theme colors, avatar decorations, badges, server tags, pronouns,
  bio, roles, connections and mutual servers.
- **Member list** grouped by role and status, loaded as you scroll.
- **Sign in** with a QR code from the Discord mobile app or with a token, stored in the system
  keychain.
- **Movable panels**: hold the right mouse button on a panel to move it, and drag a side
  panel's edge to resize it.
- **Settings** across 13 pages, including themes, accent color, zoom, fonts, message display,
  motion, accessibility, streamer mode, privacy, keybinds and caches.
- **Current client build**: the build number Discord's web app ships is read at each start
  and used from the next one, so requests keep matching the official client.
- **Sample data mode** (`RCORD_DEMO=1`) to try the interface without an account.
