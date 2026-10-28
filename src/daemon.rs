//! Background service: watches the data file and sends desktop notifications.
//! Runs as `nudge --daemon`, started by the window and on login.

use crate::store::{Reminder, Store};
use chrono::{Duration, Local};
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
        .hint(Hint::Resident(true))
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
fn icon() -> String {
    dirs::data_dir()
        .map(|d| d.join("nudge/nudge.svg"))
        .filter(|p| p.exists())
        .map_or("appointment-soon".into(), |p| p.display().to_string())
}

fn open_window() -> std::io::Result<()> {
    let mut child = Command::new(std::env::current_exe()?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()?;
    thread::spawn(move || child.wait());
    Ok(())
}
