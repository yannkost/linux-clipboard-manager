# Clips (linux-clipboard-manager)

A Ditto-style clipboard history manager for Linux (GNOME, Wayland): text and
image history, a searchable popup on <kbd>Ctrl</kbd>+<kbd>I</kbd>, pinned items,
a preview panel, tray menu, autostart, and a libadwaita-style settings window.

Built with Tauri 2 (Rust backend in `src-tauri/`, plain HTML/JS in `ui/`, no
build step). The UI follows the "Clips Desktop" design; its colour tokens live
in `ui/tokens.css`.

## Build

```bash
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev build-essential pkg-config libxdo-dev
cd src-tauri
cargo build --release
cargo test
```

Run `src-tauri/target/release/linux-clipboard-manager`. On first start it:

- writes `~/.config/linux-clipboard-manager/config.toml` with defaults,
- stores history in `~/.local/share/linux-clipboard-manager/` (`history.db` plus `images/*.png`),
- adds an XDG autostart entry (`~/.config/autostart/`) that starts it hidden with `--hidden`,
- creates (or adopts) a GNOME custom keyboard shortcut that runs `linux-clipboard-manager --toggle`,
- shows a one-time welcome dialog.

Command-line flags for a second launch: `--toggle` shows/hides the popup,
`--settings` opens the settings window.

## Configuration

Open *Settings* from the tray menu or with <kbd>Ctrl</kbd>+<kbd>,</kbd> in the
popup. Changes apply immediately and are saved to `config.toml`:

| Key | Default | |
|---|---|---|
| `hotkey` | `"<Control>i"` | GTK accelerator; empty disables it. Kept in sync with the GNOME custom shortcut |
| `max_items` | `500` | unpinned entries kept |
| `max_image_bytes` | `20971520` | larger images are skipped |
| `retention_days` | `0` | delete unpinned entries unused for this long (0 = forever, or 1/7/30) |
| `paste_after_select` | `false` | paste into the previous window (needs the GNOME helper) |
| `move_to_top` | `true` | a pasted entry moves to the top of Recent |
| `strip_formatting` | `false` | swap Enter and Shift+Enter |
| `autostart` | `true` | |
| `show_tray` | `true` | |
| `theme` | `"system"` | `system`, `light` or `dark` |
| `ignore_passwords` | `true` | needs the GNOME helper |
| `excluded_apps` | `[]` | needs the GNOME helper |
| `poll_fallback` | `true` | poll the clipboard when the GNOME helper is missing |

## The popup

Wayland does not let normal apps grab global shortcuts, so the app registers a
GNOME custom shortcut (*Settings → Keyboard → Custom Shortcuts*) that runs the
binary with `--toggle`. Change it from Clips' own Settings → Shortcut page,
which also warns when GNOME already uses the combination.

| Key | Action |
|---|---|
| type | filter |
| <kbd>↑</kbd> <kbd>↓</kbd>, <kbd>PgUp</kbd> <kbd>PgDn</kbd> | move |
| <kbd>Enter</kbd> / click | paste (<kbd>Shift</kbd>+click: as plain text) |
| <kbd>Shift</kbd>+<kbd>Enter</kbd> | paste as plain text |
| <kbd>Ctrl</kbd>+<kbd>1</kbd>…<kbd>9</kbd> | paste that row of the open tab |
| <kbd>Tab</kbd> | switch Recent / Pinned |
| <kbd>Ctrl</kbd>+<kbd>P</kbd> | pin / unpin |
| <kbd>Del</kbd> | remove |
| <kbd>Space</kbd> or <kbd>→</kbd> / <kbd>←</kbd> | open / close the preview |
| <kbd>Ctrl</kbd>+click, <kbd>Shift</kbd>+<kbd>↑</kbd><kbd>↓</kbd> | multi-select (Enter pastes all, Del removes all) |
| <kbd>Ctrl</kbd>+<kbd>,</kbd> | settings |
| <kbd>Esc</kbd> | close preview, clear selection, clear search, then close |

Entries are shown as links, file paths, colours, code, text or images,
detected from their content.

## D-Bus interface

The app owns `io.github.yannkost.ClipboardManager` on the session bus at
`/io/github/yannkost/ClipboardManager`:

| Member | Signature | |
|---|---|---|
| `Toggle()` | | show/hide the popup |
| `AddText(s)` | | record text |
| `AddImage(ay)` | | record a PNG image |
| `Hotkey` | property `s` | configured shortcut |

The GNOME helper extension also calls `AddTextFrom(s text, s app_name, s app_id, b password)`
and `AddImageFrom(ay image, s app_name, s app_id)`, so Clips can skip
passwords and excluded apps.

## GNOME helper extension

`gnome-extension/clips-helper@yannkost.github.io` runs inside GNOME Shell,
where Wayland allows what a normal app cannot:

- reports every clipboard change to Clips (with the source app and the
  `x-kde-passwordManagerHint` password marker), replacing the 500 ms polling;
- exports `io.github.yannkost.ClipboardManager.Shell` on GNOME Shell's bus
  connection (`/io/github/yannkost/ClipboardManager/Shell`): `SetText(s)`,
  `SetImage(ay)`, `Paste(b plain)`, `Version() → u`. `Paste` refocuses the
  window that was active before the popup and sends <kbd>Ctrl</kbd>+<kbd>V</kbd>
  (<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> in terminals) through a virtual keyboard.

The extension is bundled in the Clips binary. *Install* in Settings → Shortcut
(or *Enable extension* in the welcome dialog) copies it to
`~/.local/share/gnome-shell/extensions/` and adds it to GNOME's enabled
extensions; on Wayland it starts after you log out and back in. Clips keeps
the installed copy up to date on startup. To install by hand:

```bash
cp -r gnome-extension/clips-helper@yannkost.github.io ~/.local/share/gnome-shell/extensions/
gnome-extensions enable clips-helper@yannkost.github.io   # after logging out and in
```

Without the extension Clips falls back to polling the clipboard with arboard,
and selecting an entry only puts it on the clipboard (you paste with
<kbd>Ctrl</kbd>+<kbd>V</kbd> yourself).

## Credits

Bundled fonts: [Inter](https://rsms.me/inter/) and
[JetBrains Mono](https://www.jetbrains.com/lp/mono/) (SIL Open Font License),
and a subset of [Material Symbols Rounded](https://fonts.google.com/icons)
(Apache License 2.0).
