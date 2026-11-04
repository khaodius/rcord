# Changelog

Packages for every version are attached to its
[release](https://github.com/khaodius/rcord/releases), with SHA-256 checksums.

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
