#!/usr/bin/env bash
# Build Clips in release mode and install it for the current user:
#   ~/.local/bin/linux-clipboard-manager, a menu entry and icons.
# Then restart it, which also repoints the autostart entry and the GNOME
# shortcut at the installed binary.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_ID="io.github.yannkost.ClipboardManager"
BIN_DIR="$HOME/.local/bin"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
BIN="$BIN_DIR/linux-clipboard-manager"

if [[ "${1:-}" != "--no-build" ]]; then
  (cd "$ROOT/src-tauri" && cargo build --release)
fi

install -Dm755 "$ROOT/src-tauri/target/release/linux-clipboard-manager" "$BIN"
install -Dm644 "$ROOT/src-tauri/icons/32x32.png" "$DATA_DIR/icons/hicolor/32x32/apps/$APP_ID.png"
install -Dm644 "$ROOT/src-tauri/icons/128x128.png" "$DATA_DIR/icons/hicolor/128x128/apps/$APP_ID.png"
install -Dm644 "$ROOT/src-tauri/icons/icon.png" "$DATA_DIR/icons/hicolor/512x512/apps/$APP_ID.png"
install -Dm644 "$ROOT/src-tauri/icons/clips.svg" "$DATA_DIR/icons/hicolor/scalable/apps/$APP_ID.svg"
install -Dm644 "$ROOT/src-tauri/icons/clips-symbolic.svg" "$DATA_DIR/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"

mkdir -p "$DATA_DIR/applications"
cat > "$DATA_DIR/applications/$APP_ID.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Clips
GenericName=Clipboard history
Comment=Search and paste from your clipboard history
Exec=$BIN --toggle
Icon=$APP_ID
Terminal=false
Categories=Utility;
Keywords=clipboard;copy;paste;history;
StartupWMClass=linux-clipboard-manager
Actions=settings;

[Desktop Action settings]
Name=Settings
Exec=$BIN --settings
DESKTOP

gtk-update-icon-cache -q -t "$DATA_DIR/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$DATA_DIR/applications" 2>/dev/null || true

# Restart whichever Clips is running (dev or installed) as the installed one.
# The kernel truncates process names to 15 characters.
pkill -x linux-clipboard 2>/dev/null || true
sleep 1
setsid "$BIN" --hidden >/dev/null 2>&1 < /dev/null &
echo "Installed $BIN and started it."
