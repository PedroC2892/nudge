#!/bin/sh
# Removes Nudge. Your reminders in ~/.local/share/nudge are kept.
pkill -f "nudge --daemon" 2>/dev/null || true
rm -f "$HOME/.local/bin/nudge" \
      "$HOME/.local/share/applications/nudge.desktop" \
      "$HOME/.config/autostart/nudge-service.desktop"
echo "Uninstalled. Data left in ~/.local/share/nudge"
