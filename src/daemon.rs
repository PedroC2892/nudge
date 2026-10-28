//! Background service: watches the data file and sends desktop notifications.
//! Runs as `nudge --daemon`, started by the window and on login.

use crate::store::{Reminder, Store};
use chrono::{Duration, Local};
use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;
use notify_rust::{Hint, Notification, Timeout, Urgency};
use std::fs::File;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;

const TICK: std::time::Duration = std::time::Duration::from_secs(5);

pub fn run() {
    // flock is released by the kernel when the process dies, so it is a
    // stale-proof single-instance lock. XDG_RUNTIME_DIR is private to the user.
    let Some(dir) = dirs::runtime_dir().or_else(dirs::data_dir) else { return };
    let Ok(lock) = File::create(dir.join("nudge-daemon.lock")) else { return };
    if lock.try_lock().is_err() {
        return; // already running
    }
    let store = Arc::new(Store::open().expect("cannot open data directory"));
    // Keep the handle alive: dropping it removes the icon.
    let _tray = Tray
        .spawn()
        .inspect_err(|e| eprintln!("nudge: no system tray: {e}"))
        .ok();

    loop {
        match store.update(|d| (d.take_due(Local::now()), d.settings.clone())) {
            Ok((_, (fired, settings))) => {
                for r in fired {
                    notify(&store, r, settings.sound, settings.snooze_minutes);
                }
            }
            Err(e) => eprintln!("nudge: cannot update data: {e}"),
        }
        thread::sleep(TICK);
    }
}

fn notify(store: &Arc<Store>, r: Reminder, sound: bool, snooze_minutes: u32) {
    let mut n = Notification::new();
    n.appname("Nudge")
        .summary(&escape(&r.title))
        .body(&escape(&r.notes))
        .icon(&icon())
        .urgency(if r.important { Urgency::Critical } else { Urgency::Normal })
        .timeout(Timeout::Never)
        .hint(Hint::Category("reminder".into()))
        .action("default", "Open");
    // ponytail: done/snooze only for one-off reminders; repeating ones already moved to the next occurrence
    if !r.repeat.is_on() {
        n.action("done", "Mark as done");
        n.action("snooze", &format!("Snooze {snooze_minutes} min"));
    }
    if sound {
        n.hint(Hint::SoundName("alarm-clock-elapsed".into()));
    }
    let handle = match n.show() {
        Ok(h) => h,
        Err(e) => return eprintln!("nudge: notification failed: {e}"),
    };

    let store = Arc::clone(store);
    thread::spawn(move || {
        handle.wait_for_action(|action| {
            let res = match action {
                "done" => store.update(|d| d.get_mut(r.id).map(|x| x.done = true)).map(drop),
                "snooze" => store
                    .update(|d| {
                        d.get_mut(r.id).map(|x| {
                            x.due = Some(Local::now() + Duration::minutes(snooze_minutes.into()));
                            x.notified = false;
                        })
                    })
                    .map(drop),
                "default" => open_window(),
                _ => Ok(()),
            };
            if let Err(e) = res {
                eprintln!("nudge: action {action} failed: {e}");
            }
        });
    });
}

/// Notification servers parse a subset of HTML; titles and notes are plain text.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// install.sh puts the icon next to the data, since ~/.local/share/icons may not be writable.
fn icon_dir() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|d| d.join("nudge")).filter(|d| d.join("nudge.svg").exists())
}

fn icon() -> String {
    icon_dir().map_or("appointment-soon".into(), |d| d.join("nudge.svg").display().to_string())
}

/// System tray icon that stays while the window is closed.
struct Tray;

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "nudge".into()
    }

    fn title(&self) -> String {
        "Nudge".into()
    }

    fn icon_theme_path(&self) -> String {
        icon_dir().map(|d| d.display().to_string()).unwrap_or_default()
    }

    fn icon_name(&self) -> String {
        if icon_dir().is_some() { "nudge" } else { "appointment-soon" }.into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: "Nudge".into(), description: "Click to open your reminders".into(), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        if let Err(e) = open_window() {
            eprintln!("nudge: cannot open window: {e}");
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Open Nudge".into(),
                activate: Box::new(|t: &mut Self| t.activate(0, 0)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit (stop notifications)".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_: &mut Self| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Opens the window, or brings the open one to front (see `main::claim_window`).
fn open_window() -> std::io::Result<()> {
    let mut cmd = Command::new(std::env::current_exe()?);
    // Lets Wayland compositors give focus to the new window.
    if let Some(token) = ksni::current_xdg_activation_token() {
        cmd.env("XDG_ACTIVATION_TOKEN", token);
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()?;
    thread::spawn(move || child.wait());
    Ok(())
}
