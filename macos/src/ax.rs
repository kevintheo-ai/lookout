//! Accessibility (AX) and the window list: the terminals' windows with frame, title and state, writing frames, raising,
//! the focused window, and which windows lie on the visible space (CGWindowList). Swift: Common.swift (AX access,
//! `setFrame`, `movePosition`, `resizeInPlace`, `runningTerminals`, `accessibilityTrusted`), Mapper.swift
//! (`collectWindows`, `rereadWindow`), Overlay.swift (`onScreenWindows`, `FocusWatcher.focusedWindow/focusedAppPid`),
//! Main.swift `bringToFront` (the AX part).
//!
//! Every function that talks to AX runs ONLY on the AX queue (`crate::ax_queue`, `crate::ax_sync`), each call short
//! (convention 3). `trusted`, `on_screen_windows`, `running_terminals` and the id registry work from any thread.
//!
//! Window identity: core names windows by `TermWindow::id`. The registry gives every AX window an id that stays the
//! same across ticks (Swift compared AX elements with `CFEqual`; a fresh copy of the same window is `CFEqual` to the
//! old one). Ids are never reused.

use std::collections::{BTreeMap, BTreeSet};

use lookout_core::geom::{Point, Rect, Size};
use lookout_core::model::{OnScreenWindow, Pid, TermWindow, Time};
use lookout_core::motion::AxOrigin;

/// A retained AX element (application or window). `Send` and `Sync`: CF reference counting is thread-safe, and every
/// AX call goes through the serial AX queue.
#[derive(Clone)]
pub struct Ax(());

impl Ax {
    /// `AXUIElementCreateApplication`.
    pub fn app(pid: Pid) -> Ax {
        let _ = pid;
        todo!()
    }

    /// `CFEqual`: the same UI element.
    pub fn same(&self, other: &Ax) -> bool {
        let _ = other;
        todo!()
    }
}

/// The registry id of this window, registering it if new (only `collect_windows` registers).
pub fn window_id(win: &Ax) -> u64 {
    let _ = win;
    todo!()
}

/// The registry id of a window seen before; `None` for anything else (a focused window of another kind).
pub fn known_id(win: &Ax) -> Option<u64> {
    let _ = win;
    todo!()
}

/// The element behind an id; `None` once it is gone from the registry.
pub fn window(id: u64) -> Option<Ax> {
    let _ = id;
    todo!()
}

/// Running Warp and Ghostty (`NSRunningApplication`, thread-safe): pid, bundle id, localized name (bundle id if none).
pub fn running_terminals() -> Vec<(Pid, String, String)> {
    todo!()
}

/// All windows of the running terminals (Swift `collectWindows`): id, app, bundle id, owner pid, Cocoa frame (AX frame
/// converted with `ax_to_cocoa` and `screen_h`; zero if unreadable), title ("" if none), first sighting (from
/// `first_seen` by id, else `now`), movable (position AND size settable), minimized, full screen ("AXFullScreen"); no
/// sessions. Registers new windows; forgets registry entries not seen for 10 minutes. AX queue only.
pub fn collect_windows(
    first_seen: &BTreeMap<u64, Time>,
    now: Time,
    screen_h: f64,
) -> Vec<TermWindow> {
    let _ = (first_seen, now, screen_h);
    todo!()
}

/// One window read fresh (Swift `rereadWindow`): frame, minimized, full screen, title; what cannot be read keeps the old
/// value (minimized and full screen: false). AX queue only.
pub fn reread_window(w: &TermWindow, screen_h: f64) -> TermWindow {
    let _ = (w, screen_h);
    todo!()
}

/// Current title of a window (the ping reads only titles, every 10 ms). AX queue only.
pub fn title(id: u64) -> Option<String> {
    let _ = id;
    todo!()
}

/// Sets size and position (Cocoa rectangle) and returns the MEASURED frame (Swift `setFrame`): the writes of core
/// `frame_writes` (unknown current frame: position, then size), read back; more than 2 pt off -> once `rescue_writes`,
/// read back again. AX queue only.
pub fn set_frame(id: u64, cocoa: Rect, origin: AxOrigin, screen_h: f64) -> Option<Rect> {
    let _ = (id, cocoa, origin, screen_h);
    todo!()
}

/// Only the position, no read-back: the frames on the way of a glide. AX queue only.
pub fn move_position(id: u64, cocoa: Rect, origin: AxOrigin, screen_h: f64) {
    let _ = (id, cocoa, origin, screen_h);
    todo!()
}

/// Only the size, top-left stays: shrinking at the old place. AX queue only.
pub fn resize_in_place(id: u64, size: Size) {
    let _ = (id, size);
    todo!()
}

/// Is the window minimized? AX queue only.
pub fn minimized(id: u64) -> Option<bool> {
    let _ = id;
    todo!()
}

/// Back from the Dock (`AXMinimized` = false). AX queue only.
pub fn unminimize(id: u64) {
    let _ = id;
    todo!()
}

/// `AXRaise` only (orders only within its app, R2-6). AX queue only.
pub fn raise(id: u64) {
    let _ = id;
    todo!()
}

/// `AXMain` = true. AX queue only.
pub fn set_main(id: u64) {
    let _ = id;
    todo!()
}

/// The app in front via AX (`AXFrontmost` = true on the application element); false = refused (the caller falls back
/// to `NSRunningApplication.activate`). AX queue only.
pub fn set_app_frontmost(pid: Pid) -> bool {
    let _ = pid;
    todo!()
}

/// The focused window of an app (`AXFocusedWindow`), as a registry id (`known_id`). AX queue only.
pub fn focused_window(pid: Pid) -> Option<u64> {
    let _ = pid;
    todo!()
}

/// The process of the app with the keyboard focus, read now (system-wide `AXFocusedApplication`): a value remembered on
/// main could be stale while the AX queue still works off a move (R10, review quick 2026-10-04, Codex). AX queue only.
pub fn focused_app_pid() -> Option<Pid> {
    todo!()
}

/// Accessibility access; `prompt` shows macOS's dialog (`AXIsProcessTrustedWithOptions` with
/// `kAXTrustedCheckOptionPrompt`).
pub fn trusted(prompt: bool) -> bool {
    let _ = prompt;
    todo!()
}

/// Windows on the VISIBLE space (`CGWindowListCopyWindowInfo(optionOnScreenOnly | excludeDesktopElements)`), layer 0
/// only, front to back, AX coordinates; `pids` = only these owners, `None` = all apps (for `sits_above`). Geometry and
/// number need no screen-recording access, titles are never read. `None` = the query failed; empty = read, none there.
pub fn on_screen_windows(pids: Option<&BTreeSet<Pid>>) -> Option<Vec<OnScreenWindow>> {
    let _ = pids;
    todo!()
}

#[allow(dead_code)]
fn _types_used(_: Point) {}
