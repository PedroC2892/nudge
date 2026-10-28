mod daemon;
mod ics;
mod store;
mod ui;

use std::fs;
use std::io::Write;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--daemon") {
        daemon::run();
        return Ok(());
    }
    let Some(listener) = claim_window() else {
        return Ok(()); // the open window was asked to come to front
    };
    start_daemon();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Nudge")
            .with_app_id("nudge")
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native("Nudge", options, Box::new(|cc| Ok(Box::new(ui::App::new(cc, listener)))))
}

/// Starts the notification service in its own process group so it outlives
/// the window. A second instance exits immediately, so this is always safe.
fn start_daemon() {
    let Ok(exe) = std::env::current_exe() else { return };
    match Command::new(exe)
        .arg("--daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .process_group(0)
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(e) => eprintln!("nudge: cannot start notification service: {e}"),
    }
}

/// Only one window: a second launch pokes the first one through a socket in
/// the private runtime dir and exits. Returns None in that case.
fn claim_window() -> Option<Option<UnixListener>> {
    let Some(sock) = dirs::runtime_dir().map(|d| d.join("nudge-window.sock")) else {
        return Some(None);
    };
    if let Ok(mut s) = UnixStream::connect(&sock) {
        let _ = s.write_all(b"show");
        return None;
    }
    let _ = fs::remove_file(&sock); // stale file from a crashed window
    Some(UnixListener::bind(&sock).ok())
}
