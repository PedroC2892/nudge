# Progress

## M0 — Core app: DONE (2026-10-28)

Deliverable: a desktop reminder app in Rust, opened from the application menu,
that reminds the user what to do and when.

### Done
- egui/eframe GUI: sidebar with smart views (All, Today, Upcoming, Overdue,
  Important, Completed) and custom lists, reminder cards, editor dialog.
- Reminders with title, notes, due time, importance, completion and list.
- Repeat rules: every N minutes, hours, days, weeks, months or years, with an
  optional end date. Missed occurrences are skipped, not fired in bulk.
- Search (Ctrl+F), sorting (time, title, newest), "show completed" toggle,
  Ctrl+N for a new reminder.
- Background service (`nudge --daemon`) sends desktop notifications with
  done, snooze and open actions and an optional sound.
- Shared JSON store at `~/.local/share/nudge/data.json`: locked
  read-modify-write and atomic rename, so the window and the service never
  overwrite each other. A corrupt file is moved aside, not lost.
- iCalendar (`.ics`) import and export (VTODO; VEVENT on import).
- Settings: 12/24-hour time, sound, snooze length, start on login.
- `install.sh` / `uninstall.sh`: per-user install, menu entry, autostart.

### Decisions
- egui instead of GTK4/libadwaita: no system dev libraries needed on KDE.
- Notifications live in a separate process so they keep working after the
  window is closed.

## M1 — Startup time: DONE (2026-10-28)

Problem: the window took ~13 s to open.
Cause: eframe's default wgpu renderer spent the whole time in Vulkan setup,
measured with a timer before the first frame.
Fix: switch to the glow (OpenGL) renderer. Startup is now ~0.13 s and the
binary went from 32 MB to 24 MB.

## M2 — Security hardening: DONE (2026-10-28)

- Single-instance lock moved from an abstract Unix socket (shared by all local
  users, could be squatted) to `flock` on a file in `XDG_RUNTIME_DIR`.
- Notification title and body are HTML-escaped; imported `.ics` text cannot
  inject markup.
- Repeat arithmetic is checked; huge intervals end the series instead of
  panicking. Missed fixed-length periods are skipped in one step. Imported
  `INTERVAL` is clamped to 1..=999.

## M3 — Desktop integration: DONE (2026-10-28)

- System tray icon (StatusNotifierItem via `ksni`) owned by the service, so it
  stays after the window is closed. Click opens the window; menu has
  "Open Nudge" and "Quit".
- Single window: a second launch asks the open window to come to front and
  exits.
- Notifications are not resident, so they close after an action button.
- Autostart entry visible in KDE System Settings > Autostart.
- Menu entry in a single category (Utility).

## Tests

`cargo test`: repeat rules and end dates, missed-occurrence handling, fast
skipping of old minutely reminders, `.ics` round trip.
`cargo clippy` clean.

## Open items / ideas

- Sync with Microsoft To Do and CalDAV (the reference app has it, optional).
- Repeat on specific weekdays (for example Monday and Wednesday).
- Done/snooze buttons for repeating reminders (today only one-off reminders
  have them, because repeating ones already moved to the next occurrence).
- `.ics` import ignores `TZID`; times are read as local.
- Bringing the open window to front may only flash the taskbar on Wayland
  (no activation token is passed to an existing window).
- Undo for deletions instead of the confirmation dialog.
