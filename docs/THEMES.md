# Themes

A custom theme starts from one of rcord's built-in themes (Dark, Midnight, Ash or Light). It
can replace any color, both corner radii and both fonts. Anything it leaves out comes from the
built-in theme it starts from. Themes are plain TOML files that you can edit by hand, share and
import.

## Making one in the app

1. Open Settings > Appearance and scroll to **Custom theme**.
2. Choose **Create from current theme**. This copies the selected theme, adds it to the theme
   list above under its own name and switches rcord to it.
3. Change any color. Each field shows the inherited color as grey placeholder text. Type
   `#rrggbb`, or `#rrggbbaa` for a translucent color, and the change shows at once. Clear a
   field, or press its **×**, to go back to the inherited color.
4. Choose **Export…** to save the theme as a file.

**Import…** loads a theme file, copies it into your themes folder and switches to it. Every
file in the themes folder is listed under **In your themes folder**. Clicking one switches to
it.

The themes folder is:

| System  | Folder                                         |
| ------- | ---------------------------------------------- |
| Linux   | `~/.config/rcord/themes/`                      |
| Windows | `%APPDATA%\khaodius\rcord\config\themes\`      |

**Open themes folder** in Settings opens it in your file manager. Any `.toml` file dropped in
there shows up the next time you open Settings.

## The file

```toml
# rcord theme. Every key is described in docs/THEMES.md.
name = "Ocean"
author = "you"
base = "midnight"
radius = 10
radius_small = 6
ui_font = "IBM Plex Sans"
code_font = "IBM Plex Mono"

[colors]
surface = "#0b1622"
sidebar = "#08111b"
accent = "#00a8fc"
mention_bg = "#00a8fc3d"
```

### Top-level keys

| Key            | Type   | Meaning                                                                 |
| -------------- | ------ | ----------------------------------------------------------------------- |
| `name`         | text   | Shown under the theme in Settings, and used for the file name on export. |
| `author`       | text   | Who made it. Optional.                                                  |
| `base`         | text   | The built-in theme to start from: `dark`, `midnight`, `ash` or `light`. |
| `radius`       | number | Corner radius of panels, cards and popups, 0 to 24. Replaces the corner roundness setting. |
| `radius_small` | number | Corner radius of buttons, inputs and chips, 0 to 16. Without it, rcord uses half of `radius`. |
| `ui_font`      | text   | Font family for the interface. Any installed font. Replaces the font setting. |
| `code_font`    | text   | Font family for code blocks and inline code.                            |

Every key is optional.

### Colors

Colors go in a `[colors]` table. Each value is `#rgb`, `#rrggbb` or `#rrggbbaa`, where the
last pair is opacity (`00` is clear, `ff` is solid). The saturation setting still applies on
top of a custom theme.

| Key             | Paints                                                                    |
| --------------- | ------------------------------------------------------------------------- |
| `base`          | The window background behind everything, and the server list.            |
| `sidebar`       | The channel list and the member list.                                     |
| `surface`       | The chat area.                                                            |
| `floating`      | Menus, tooltips, profile cards and pickers.                               |
| `input`         | The message box and other text fields.                                    |
| `hover`         | Rows under the pointer. Usually translucent, such as `#ffffff0b`.         |
| `selected`      | The open channel and selected items. Usually translucent, such as `#ffffff16`. |
| `border`        | Lines between panels and around cards.                                    |
| `text`          | Message text and most labels.                                             |
| `text_muted`    | Channel names, descriptions and other secondary text.                     |
| `text_faint`    | Timestamps, placeholders and hints.                                       |
| `header`        | Names, titles and the open channel's name.                                |
| `accent`        | Buttons, switches, sliders and highlights. Replaces the accent color setting. |
| `accent_text`   | Text on accent-colored buttons. Without it, rcord picks black or white for contrast. |
| `link`          | Links in messages.                                                        |
| `danger`        | Errors, delete buttons and mention badges.                                |
| `online`        | The online status dot.                                                    |
| `idle`          | The idle status dot.                                                      |
| `dnd`           | The do-not-disturb status dot.                                            |
| `offline`       | The offline status dot.                                                   |
| `mention_bg`    | Behind @mentions inside messages.                                         |
| `mention_text`  | The text of @mentions.                                                    |
| `mentioned_row` | Behind whole messages that mention you. Usually translucent.              |
| `code_bg`       | Inline code and code blocks.                                              |
| `scrim`         | The dimming over the app behind dialogs, such as `#000000b3`.             |

rcord skips any key or color it can't read, so a theme made for a newer version still opens
in an older one.

## Where it is stored

The theme in use is kept in `settings.toml` under `[appearance.custom_theme]`, in the same
format, so it survives even when the file it came from is deleted. `appearance.theme =
"custom"` selects it.
