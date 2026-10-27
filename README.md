# linux-clipboard-manager

A Ditto-style clipboard history manager for Linux: text and image history,
tray icon, autostart, and a searchable popup on <kbd>Ctrl</kbd>+<kbd>I</kbd>.

Built with Tauri 2 (Rust backend in `src-tauri/`, plain HTML/JS popup in `ui/`).

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
- adds an XDG autostart entry (`~/.config/autostart/`) that starts it hidden with `--hidden`.

## Configuration

Open *Settings…* from the tray menu, the ⚙ button in the popup, or <kbd>Ctrl</kbd>+<kbd>,</kbd>.
Settings are saved to `config.toml`:

```toml
hotkey = "<Control>i"          # GTK accelerator syntax
max_items = 500                # unpinned entries kept
max_image_bytes = 20971520     # larger images are skipped
paste_after_select = false     # paste into the previous window (needs the GNOME helper)
autostart = true
poll_fallback = true           # poll the clipboard when the GNOME helper is missing
```

## Opening the popup

Wayland does not let normal apps grab global shortcuts. Until the GNOME Shell
helper extension exists, bind a custom shortcut in
*Settings → Keyboard → Custom Shortcuts* to the binary's full path plus
`--toggle`, for example:

```
/home/you/linux-clipboard-manager/src-tauri/target/release/linux-clipboard-manager --toggle
```

GNOME does not expand `~` in shortcut commands, so use an absolute path.

A second launch toggles the popup of the running instance.

Popup keys: arrows to move, <kbd>Enter</kbd> paste, <kbd>Shift</kbd>+<kbd>Enter</kbd>
paste as plain text, <kbd>Ctrl</kbd>+<kbd>1</kbd>…<kbd>9</kbd> paste that row,
<kbd>Ctrl</kbd>+<kbd>P</kbd> pin, <kbd>Del</kbd> delete, <kbd>Esc</kbd> close.

## D-Bus interface

The app owns `io.github.yannkost.ClipboardManager` on the session bus at
`/io/github/yannkost/ClipboardManager`:

| Member | Signature | |
|---|---|---|
| `Toggle()` | | show/hide the popup |
| `AddText(s)` | | record text |
| `AddImage(ay)` | | record a PNG image |
| `Hotkey` | property `s` | configured shortcut |

It calls the GNOME Shell helper extension at `org.gnome.Shell`
`/io/github/yannkost/ClipboardManager/Shell`, interface
`io.github.yannkost.ClipboardManager.Shell`: `SetText(s)`, `SetImage(ay)`,
`Paste(b plain)`, `Version() → u`. When it is not there, the app polls the
clipboard every 500 ms with arboard instead, and selecting an entry only puts
it on the clipboard (you paste with <kbd>Ctrl</kbd>+<kbd>V</kbd> yourself).
