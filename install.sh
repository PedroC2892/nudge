#!/bin/sh
# Builds Nudge and installs it for the current user (no root needed).
set -eu
cd "$(dirname "$0")"

cargo build --release

BIN="$HOME/.local/bin"
APPS="$HOME/.local/share/applications"
ICON="$HOME/.local/share/nudge/nudge.svg"
mkdir -p "$BIN" "$APPS" "$(dirname "$ICON")"

install -m 755 target/release/nudge "$BIN/nudge"
install -m 644 assets/nudge.svg "$ICON"
cat > "$APPS/nudge.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Nudge
GenericName=Reminders
Comment=Remember what to do and when
Exec=$BIN/nudge
Icon=$ICON
Terminal=false
Categories=Utility;Office;
Keywords=reminder;todo;task;alarm;
StartupWMClass=nudge
DESKTOP

# Notifications on login, even before the window is opened.
mkdir -p "$HOME/.config/autostart"
cat > "$HOME/.config/autostart/nudge-service.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Nudge
Comment=Reminder notifications and tray icon
Exec="$BIN/nudge" --daemon
Icon=$ICON
X-GNOME-Autostart-enabled=true
DESKTOP

update-desktop-database "$APPS" 2>/dev/null || true
command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 >/dev/null 2>&1 || true

# Restart the service so it runs the new binary.
pkill -f "$BIN/nudge --daemon" 2>/dev/null || true
setsid "$BIN/nudge" --daemon >/dev/null 2>&1 &
echo "Installed. Open 'Nudge' from the application menu."
