//! Lookout for macOS: the platform layer around `lookout-core` (plan stage 2). The core decides; this crate reads the
//! system (processes, windows, screens), moves windows, draws the overlays and runs the menu bar item.
//!
//! Threads (plan, convention 3, as in Swift): the main thread owns all state and every AppKit object; process, log and
//! ping work runs on the work queue; every AX call runs on the AX queue, each one short. Results come back with
//! `on_main`.

#![cfg(target_os = "macos")]

pub mod animator;
pub mod ax;
pub mod controller;
pub mod focus;
pub mod hotkeys;
pub mod icon;
pub mod menu;
pub mod mouse;
pub mod overlay;
pub mod prefs;
pub mod restore;
pub mod screens;
pub mod sys;

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dispatch2::{DispatchQueue, DispatchRetained, DispatchTime};
use lookout_core::model::Time;

static AX_QUEUE: OnceLock<DispatchRetained<DispatchQueue>> = OnceLock::new();
static WORK_QUEUE: OnceLock<DispatchRetained<DispatchQueue>> = OnceLock::new();

/// ONLY AX calls, each one short. AXObserver callbacks never call AX themselves.
pub fn ax_queue() -> &'static DispatchQueue {
    AX_QUEUE.get_or_init(|| DispatchQueue::new("lookout.ax", None))
}

/// Process work (claude agents, nettop, lsof), the title ping and log I/O.
pub fn work_queue() -> &'static DispatchQueue {
    WORK_QUEUE.get_or_init(|| DispatchQueue::new("lookout.work", None))
}

/// Runs `f` on the AX queue and waits for its result. Never call it from the AX queue itself (deadlock).
pub fn ax_sync<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    let mut out = None;
    ax_queue().exec_sync(|| out = Some(f()));
    out.expect("exec_sync runs the closure before it returns")
}

/// Hands `f` to the main thread (Swift `DispatchQueue.main.async`).
pub fn on_main(f: impl FnOnce() + Send + 'static) {
    DispatchQueue::main().exec_async(f);
}

/// `f` on the main thread after `seconds` (Swift `DispatchQueue.main.asyncAfter`).
pub fn on_main_after(seconds: f64, f: impl FnOnce() + Send + 'static) {
    after(DispatchQueue::main(), seconds, f);
}

/// `f` on `queue` after `seconds`.
pub fn after(queue: &DispatchQueue, seconds: f64, f: impl FnOnce() + Send + 'static) {
    let delay = Duration::try_from_secs_f64(seconds.max(0.0)).unwrap_or_default();
    let when = DispatchTime::try_from(delay).unwrap_or(DispatchTime::NOW);
    let _ = queue.after(when, f);
}

/// Seconds since the UNIX epoch (core `Time`).
pub fn now() -> Time {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

/// The user's home folder.
pub fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// The configuration file: `LOOKOUT_CONFIG` (tests and the live test beside the Swift app), else
/// `~/.config/lookout.json`.
pub fn config_path() -> String {
    std::env::var("LOOKOUT_CONFIG").unwrap_or_else(|_| format!("{}/.config/lookout.json", home()))
}

/// The log: `LOOKOUT_LOG` (tests, the shadow run), else `~/Library/Logs/Lookout.log`, the Swift app's file and format.
pub fn log_path() -> &'static str {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        std::env::var("LOOKOUT_LOG")
            .unwrap_or_else(|_| format!("{}/Library/Logs/Lookout.log", home()))
    })
}

/// One log line (German, Swift's texts), from any thread.
pub fn log(message: &str) {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    lookout_core::log::append_log(message, log_path(), lookout_core::log::LOG_ROTATE_BYTES);
}
