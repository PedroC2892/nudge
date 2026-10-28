# Nudge

A desktop reminder app for Linux, written in Rust. It keeps track of what you
have to do and when, and sends a desktop notification at the right time — even
when the window is closed.

## Features

- Reminders with title, notes, date and time
- Repeat every N minutes, hours, days, weeks, months or years, with an optional end date
- Mark reminders as important or completed
- Group reminders into lists (create, rename, delete)
- Smart views: All, Today, Upcoming, Overdue, Important, Completed
- Search (Ctrl+F) and sort by time, title or creation date
- Desktop notifications with **Mark as done**, **Snooze** and **Open** actions, optional sound
- Background notification service that starts on login
- Import and export iCalendar (`.ics`) files
- 12/24-hour time format

## Requirements

- Rust (stable, edition 2024)
- A Linux desktop with a notification server (KDE Plasma, GNOME, …) and OpenGL

## Install

```sh
./install.sh
```

This builds a release binary and installs it for the current user (no root):

- `~/.local/bin/nudge` — the app
- `~/.local/share/applications/nudge.desktop` — entry in the application menu
- `~/.config/autostart/nudge-service.desktop` — notification service on login

To remove it, run `./uninstall.sh`. Your reminders are kept.

## How it works

The same binary runs in two modes:

- `nudge` opens the window and makes sure the background service is running.
- `nudge --daemon` is the background service. It checks for due reminders every
  few seconds and sends the notifications. Only one instance runs per user.

Both processes share `~/.local/share/nudge/data.json`. Every write is a locked
read-modify-write, so changes made from a notification (done, snooze) and from
the window never overwrite each other.

## Development

```sh
cargo run          # open the window
cargo run -- --daemon
cargo test
```
