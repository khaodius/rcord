# Working on rcord

Notes for anyone, person or coding agent, changing this repository. They cover how the
code is laid out, what CI checks, how a release goes out, and the rules the owner set.
Read this before your first commit.

## Ground rules from the owner

- **Authorship.** Commits are authored as the owner and carry nothing else:
  `git -c user.name=khaodius -c user.email=72529959+khaodius@users.noreply.github.com commit ...`.
  - No `Co-Authored-By`, `Signed-off-by` or session trailers.
  - No "generated with" lines, tool names or model names. This applies to commit messages,
    PR titles and bodies, tags, release notes, code comments and logs.
  - If a tool adds a footer to a PR body, remove it.
- **Releases come from `main`.** Pull before you push: other work lands on `main` often.
- **One version per change set.** Before you release, check that nobody has already taken
  the next version number (see [Versions](#versions)).
- **CI stays green on `main`.** Run the same checks locally before pushing (see
  [Local checks](#local-checks)). Never skip, ignore or delete a test to get green.

## Layout

A Cargo workspace. Everything is Rust, and the UI is [GPUI](https://www.gpui.rs) 0.2.2.

| Crate | What it holds |
| --- | --- |
| `crates/rcord` | The app: `main.rs` (windows, tray, title bar), `state.rs` (`AppState`, overlays, settings changes), `ui/*` (views), `voice.rs`, `golive.rs`, `guild_actions.rs` (server and channel actions), `net.rs` (Tokio runtime and REST client), `demo.rs` (sample data) |
| `crates/rcord-model` | Discord's types: messages, channels, guilds, permissions, presence, notification settings, CDN URLs |
| `crates/rcord-store` | The cache that gateway events are applied to. `events.rs` parses dispatches; `lib.rs` applies them and answers queries (unread, permissions, member lists, mutes) |
| `crates/rcord-gateway` | The gateway connection: zlib, heartbeat, resume, opcodes |
| `crates/rcord-rest` | REST client with per-route rate limits. Endpoints live in `routes.rs` |
| `crates/rcord-markup` | Discord markdown parser. It must never panic, and a fuzz test enforces that |
| `crates/rcord-settings` | `Settings` and its TOML file. Serde defaults must keep old files loading |
| `crates/rcord-voice` | Voice: Opus, DAVE end-to-end encryption, RTP, jitter buffer, audio devices |
| `crates/rcord-capture` | Screen capture for Go Live: the desktop portal and PipeWire on Wayland, X11, and Windows |
| `crates/rcord-media` | The in-app video player: MP4 demux, OpenH264 decode, AAC via Symphonia, sound via cpal |
| `crates/rcord-auth` | QR code sign-in and token storage |
| `crates/rcord-webview` | A separate small binary that shows Discord's captcha and web sign-in |
| `third_party/` | Patched xattr, and the PipeWire 1.0 headers that the bindings are generated from |

[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) explains how the pieces fit together.
[docs/THEMES.md](docs/THEMES.md) documents the theme file format.

## Building

Rust 1.88 or newer (`rust-version` in `Cargo.toml`; CI checks it). On Ubuntu 22.04 or 24.04:

```sh
sudo apt-get install -y pkg-config libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev libvulkan-dev \
  libwebkit2gtk-4.1-dev libasound2-dev libpipewire-0.3-dev libgbm-dev libclang-dev
export PKG_CONFIG_PATH=$PWD/third_party/pipewire/pkgconfig
cargo build -p rcord
```

Leave out `PKG_CONFIG_PATH` and the PipeWire bindings fail to build. If you are short on
disk, set `CARGO_INCREMENTAL=0`; the debug target directory grows past 20 GB with
incremental builds.

The Windows build needs the MSVC toolchain. Cross-checking from Linux with
`--target x86_64-pc-windows-msvc` fails in `ring`, so Windows-only code
(`#[cfg(windows)]`) is verified by CI only. Keep it small and simple to review.

## Local checks

These are exactly what CI runs. Run all four before every push:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo +1.88 check --workspace --locked    # the minimum Rust version job
```

- Clippy runs on the newest stable and treats warnings as errors. New lints appear with each
  Rust release (for example `chunks_exact_to_as_chunks`).
- If the 1.88 check fails because a dependency raised its own minimum Rust version, pin
  that dependency to its last compatible version (`openh264 = "=0.9.0"` is pinned for this
  reason) and run `cargo update -p <crate> --precise <version>`.
- Run `cargo fmt --all` after every edit, including small ones. A formatting failure is the
  most common way `main` turns red.

## CI workflows

All are in `.github/workflows/`.

| Workflow | Runs on | Does |
| --- | --- | --- |
| `ci.yml` | push to `main`, every PR, manual | Linux and Windows: fmt, clippy `-D warnings`, tests. A separate job checks Rust 1.88. Then it builds release binaries and uploads them as artifacts |
| `release.yml` | push to `main` that touches `Cargo.toml` or the workflow, a `v*` tag, manual | Publishes a release when the workspace version has no tag yet (details below) |
| `install-scripts.yml` | changes to `scripts/` | ShellCheck and PSScriptAnalyzer, then installs the latest release on Linux (user install, `.deb`, `.rpm` on Fedora) and Windows |

CI cancels an older run on the same branch when a newer push arrives. A cancelled run shows
as failing on the badge until the next run finishes. The README badge only tracks `main`.

To watch a run: `gh run list --limit 5`, then `gh run watch <id> --exit-status`. To read the
failure: `gh run view <id> --log-failed`.

## Releasing

Releases are automatic from `main`:

1. Bump `version` under `[workspace.package]` in the root `Cargo.toml`.
2. Run `cargo check` so `Cargo.lock` picks up the new version. The release job fails if the
   two disagree.
3. Add a `## X.Y.Z - YYYY-MM-DD` section at the top of `CHANGELOG.md`. The release job fails
   without one, and the release notes are copied from it.
4. Commit and push to `main`, or merge a PR into it.

`release.yml` then does the following:

- checks the version, lock file and changelog;
- builds the packages on clean runners:
  - Linux: `.deb`, `.rpm`, and a `.tar.gz` for user installs;
  - Windows: an Inno Setup installer and a portable `.zip`;
- writes `SHA256SUMS`;
- attests build provenance;
- tags the commit `vX.Y.Z` and publishes the GitHub release.

A push whose version is already released does nothing, so pushing fixes after a release is
safe. The in-app updater (`crates/rcord/src/update.rs`) reads the latest release and checks
each package against `SHA256SUMS`, so asset names must stay as they are.

To check that a release went out:
`gh api repos/khaodius/rcord/releases/tags/vX.Y.Z --jq '.tag_name, (.assets | length)'`.
There should be 6 assets. The GraphQL API is not available from every environment, so use
REST.

### Versions

- Several pieces of work often go on at once. Before bumping, run
  `git fetch origin main && git show origin/main:Cargo.toml | grep -m1 ^version` and check
  open PRs for a bump of their own: `gh pr list`, or
  `gh api repos/khaodius/rcord/pulls --jq '.[].head.ref'` where GraphQL is blocked.
- Take the next free number. Don't reuse one that has been released or that an open PR has
  claimed.
- If your branch conflicts on `Cargo.toml`, `Cargo.lock` or `CHANGELOG.md` after a rebase:
  - take `main`'s files;
  - set your version again;
  - run `cargo check`;
  - put your changelog section on top.
- Write each changelog entry as one bold lead phrase and plain sentences that say what
  changed for the user. Follow the style of the existing entries.

## Seeing the app without an account

`RCORD_DEMO=1` starts with sample servers, channels, members and messages and makes no
network calls. Other values open a specific screen:

- `RCORD_DEMO=settings`: Appearance settings
- `RCORD_DEMO=friends`: the Friends page
- `RCORD_DEMO=streams`: streams, on branches that have it

Sample data lives in `crates/rcord/src/demo.rs`.

To screenshot in a headless Linux container, run nested Weston and use its screenshooter. An
X11 root-window grab comes out black.

```sh
Xvfb :98 -screen 0 1600x1000x24 &
export XDG_RUNTIME_DIR=/tmp/xdg; mkdir -p $XDG_RUNTIME_DIR; chmod 700 $XDG_RUNTIME_DIR
DISPLAY=:98 weston --backend=x11 --renderer=pixman --width=1440 --height=900 \
  --shell=kiosk --socket=wl-rcord --debug --idle-time=0 &
sleep 3
env -u DISPLAY WAYLAND_DISPLAY=wl-rcord XDG_CONFIG_HOME=/tmp/cfg \
  VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json RCORD_DEMO=1 target/debug/rcord &
sleep 8; WAYLAND_DISPLAY=wl-rcord weston-screenshooter   # writes wayland-screenshot-*.png
```

- `--debug` is required for the screenshooter.
- `--shell=desktop` draws a desktop around the window instead of filling the screen.
- To change settings for a shot, put a `settings.toml` in `$XDG_CONFIG_HOME/rcord/`, for
  example `[appearance]` with `theme = "midnight"`.
- The README screenshot is `.github/assets/screenshot.png` at 1440×900, in the Midnight
  theme.

## GPUI pitfalls seen in this code

- `on_mouse_move` and `capture_any_mouse_up` fire only while the element is hovered. To
  follow a drag past an element or out of the window, register `window.on_mouse_event`
  from a `canvas` paint callback. `Workspace::pointer_watch` does this.
- A bare `div` is block layout. A `flex_1` child inside it with a fixed-width sibling
  (an image or GIF) can be measured at its narrowest, and its text then wraps one letter
  per line. Give list rows `w_full()`, and make wrappers `flex().flex_col()`.
- `ListState` keeps measured heights. Call `reset` when something that changes row height
  changes, such as a setting, and not on every event.
- Children are not clipped to rounded corners, so round the image itself.
- Windows cannot add or remove the system title bar on an open window. rcord reopens the
  main window when the title bar setting changes there.
- `remove_window` does not call `on_window_should_close`.
- Redraw only what changed. Settings redraws for settings, overlay and a few store events,
  not for every message, typing indicator or presence. Large views must not subscribe to
  `StateEvent::Store(_)` as a whole.

## Discord specifics

- rcord identifies as the desktop client (`ClientProperties`). Gateway capabilities and
  READY fields follow that client, and READY parsing is lenient: an item that fails to parse
  is skipped, not fatal.
- Member lists come from lazy guild subscriptions (op 37) and `GUILD_MEMBER_LIST_UPDATE`.
  Presence changes also arrive as `PRESENCE_UPDATE`.
- Notification settings and mutes arrive in READY as `user_guild_settings` and in
  `USER_GUILD_SETTINGS_UPDATE`. They are changed with
  `PATCH /users/@me/guilds/{id}/settings`.
- REST calls go through `rcord_rest::Client`, which follows rate limits per route. Add new
  endpoints to `routes.rs` and call them through the app's `Net` runtime, never from the UI
  thread.

## Tests

- Unit tests sit next to the code (`#[cfg(test)] mod tests`).
- Gateway and READY parsing have fixtures in `crates/rcord-store/src/tests.rs`.
- When you fix a crash, add the input that caused it as a test.
- `rcord-markup` has a fuzz-style test; keep it passing.
