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
use std::ptr::{self, NonNull};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use objc2_app_kit::NSWorkspace;
use objc2_application_services::{
    AXError, AXIsProcessTrusted, AXIsProcessTrustedWithOptions, AXUIElement, AXValue, AXValueType,
    kAXTrustedCheckOptionPrompt,
};
use objc2_core_foundation::{
    CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect,
    CGSize,
};
use objc2_core_graphics::{
    CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo, CGWindowListOption,
    kCGNullWindowID, kCGWindowBounds, kCGWindowLayer, kCGWindowNumber, kCGWindowOwnerPID,
};

use lookout_core::geom::{Rect, Size};
use lookout_core::model::{OnScreenWindow, Pid, TermWindow, Time};
use lookout_core::motion::{
    AxOrigin, AxWrite, TERMINAL_BIDS, ax_origin, ax_to_cocoa, cocoa_to_ax, frame_writes,
    frames_differ, rescue_writes,
};

// Attribute and action names: `#define`s in the HIServices headers, so not in the binding.
const WINDOWS: &str = "AXWindows";
const TITLE: &str = "AXTitle";
const POSITION: &str = "AXPosition";
const SIZE: &str = "AXSize";
const MINIMIZED: &str = "AXMinimized";
const FULL_SCREEN: &str = "AXFullScreen";
const MAIN: &str = "AXMain";
const FRONTMOST: &str = "AXFrontmost";
const FOCUSED_WINDOW: &str = "AXFocusedWindow";
const FOCUSED_APPLICATION: &str = "AXFocusedApplication";
const RAISE: &str = "AXRaise";

/// A retained AX element (application or window). `Send` and `Sync`: CF reference counting is thread-safe, and every
/// AX call goes through the serial AX queue.
#[derive(Clone)]
pub struct Ax(pub(crate) CFRetained<AXUIElement>);

// SAFETY: the binding leaves AXUIElement !Send and !Sync only because CF types start out that way. The element is an
// immutable token for a UI element of some process (pid + id): retain, release and `CFEqual` are thread-safe, and every
// AX call on it runs on the serial AX queue (convention 3).
unsafe impl Send for Ax {}
// SAFETY: see `Send`; `&Ax` offers `same` (thread-safe) and AX calls, which convention 3 keeps on the AX queue.
unsafe impl Sync for Ax {}

impl Ax {
    /// `AXUIElementCreateApplication`.
    pub fn app(pid: Pid) -> Ax {
        // SAFETY: only creates the element; for a pid without an app it is one whose calls fail.
        Ax(unsafe { AXUIElement::new_application(pid) })
    }

    /// `CFEqual`: the same UI element.
    pub fn same(&self, other: &Ax) -> bool {
        // CF types compare with `CFEqual`.
        *self.0 == *other.0
    }

    // ---------- Attribute access (Common.swift `axRaw` and friends); AX queue only ----------

    /// `AXUIElementCopyAttributeValue`, `None` on any error.
    fn raw(&self, attr: &'static str) -> Option<CFRetained<CFType>> {
        let mut value: *const CFType = ptr::null();
        // SAFETY: `value` is a valid out pointer for the call.
        let err = unsafe {
            self.0
                .copy_attribute_value(&CFString::from_static_str(attr), NonNull::from(&mut value))
        };
        if err != AXError::Success {
            return None;
        }
        let value = NonNull::new(value.cast_mut())?;
        // SAFETY: on success the copied value is ours to release (create rule).
        Some(unsafe { CFRetained::from_raw(value) })
    }

    fn string(&self, attr: &'static str) -> Option<String> {
        Some(self.raw(attr)?.downcast::<CFString>().ok()?.to_string())
    }

    /// Swift `as? Bool`: a CFBoolean, or a number equal to 0 or 1.
    fn bool(&self, attr: &'static str) -> Option<bool> {
        let value = self.raw(attr)?;
        if let Some(b) = value.downcast_ref::<CFBoolean>() {
            return Some(b.as_bool());
        }
        match value.downcast_ref::<CFNumber>()?.as_i64()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    fn element(&self, attr: &'static str) -> Option<Ax> {
        self.raw(attr)?.downcast::<AXUIElement>().ok().map(Ax)
    }

    /// The AX elements of an array attribute (anything else in it is skipped; Swift's cast could not check); empty if
    /// unreadable.
    fn elements(&self, attr: &'static str) -> Vec<Ax> {
        let Some(array) = self.raw(attr).and_then(|v| v.downcast::<CFArray>().ok()) else {
            return vec![];
        };
        // SAFETY: an AX array holds CF objects, so reading them as CFType is sound; each is type-checked below.
        let array = unsafe { CFRetained::cast_unchecked::<CFArray<CFType>>(array) };
        array
            .iter()
            .filter_map(|v| v.downcast::<AXUIElement>().ok().map(Ax))
            .collect()
    }

    /// Actual frame in AX coordinates (top-left), Swift `axFrame`.
    fn frame(&self) -> Option<Rect> {
        let position = self.raw(POSITION)?.downcast::<AXValue>().ok()?;
        let size = self.raw(SIZE)?.downcast::<AXValue>().ok()?;
        let (mut p, mut s) = (CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0));
        // SAFETY: `p` and `s` are of the types asked for; AX answers false for a value of another type.
        let read = unsafe {
            position.value(AXValueType::CGPoint, NonNull::from(&mut p).cast())
                && size.value(AXValueType::CGSize, NonNull::from(&mut s).cast())
        };
        read.then(|| Rect::new(p.x, p.y, s.width, s.height))
    }

    fn settable(&self, attr: &'static str) -> bool {
        let mut settable = 0u8;
        // SAFETY: `settable` is a valid out pointer (a C `Boolean`).
        let err = unsafe {
            self.0.is_attribute_settable(
                &CFString::from_static_str(attr),
                NonNull::from(&mut settable),
            )
        };
        err == AXError::Success && settable != 0
    }

    fn set(&self, attr: &'static str, value: &CFType) -> bool {
        // SAFETY: callers pass the attribute's own type: a CFBoolean for the flags, an AXValue for position and size.
        let err = unsafe {
            self.0
                .set_attribute_value(&CFString::from_static_str(attr), value)
        };
        err == AXError::Success
    }

    fn set_bool(&self, attr: &'static str, value: bool) -> bool {
        self.set(attr, CFBoolean::new(value))
    }

    /// Swift `writePosition` / `writeSize`: the result is not checked, the read-back is.
    fn write(&self, w: AxWrite) {
        let (attr, value) = match w {
            AxWrite::Position(p) => {
                let p = CGPoint::new(p.x, p.y);
                // SAFETY: points at a CGPoint, the type named; AXValueCreate copies it.
                let v = unsafe { AXValue::new(AXValueType::CGPoint, NonNull::from(&p).cast()) };
                (POSITION, v)
            }
            AxWrite::Size(s) => {
                let s = CGSize::new(s.w, s.h);
                // SAFETY: points at a CGSize, the type named; AXValueCreate copies it.
                let v = unsafe { AXValue::new(AXValueType::CGSize, NonNull::from(&s).cast()) };
                (SIZE, v)
            }
        };
        if let Some(value) = value {
            self.set(attr, &value);
        }
    }

    fn perform(&self, action: &'static str) -> bool {
        // SAFETY: a plain action name; AX answers kAXErrorActionUnsupported for one it does not know.
        let err = unsafe { self.0.perform_action(&CFString::from_static_str(action)) };
        err == AXError::Success
    }

    fn pid(&self) -> Option<Pid> {
        let mut pid = 0;
        // SAFETY: `pid` is a valid out pointer.
        (unsafe { self.0.pid(NonNull::from(&mut pid)) } == AXError::Success).then_some(pid)
    }
}

fn apply(win: &Ax, writes: &[AxWrite]) {
    for &w in writes {
        win.write(w);
    }
}

// ---------- Window identity ----------

/// Swift held a window's AX element as long as the controller did; the registry holds it here, so a window AX misses
/// for a tick keeps its id. Forgotten when unseen this long.
const FORGET_AFTER: Duration = Duration::from_secs(10 * 60);

struct Entry {
    win: Ax,
    id: u64,
    /// Monotonic: with the wall clock, a jump forward would forget a window AX just missed for a tick.
    seen: Instant,
}

/// ponytail: linear `CFEqual` search, fine for the ~40 windows of a desk; key it by `CFHash` if it ever holds hundreds.
struct Registry {
    entries: Vec<Entry>,
    last_id: u64,
}

impl Registry {
    const fn new() -> Registry {
        Registry {
            entries: Vec::new(),
            last_id: 0,
        }
    }

    fn known(&self, win: &Ax) -> Option<u64> {
        self.entries.iter().find(|e| e.win.same(win)).map(|e| e.id)
    }

    /// The id of `win`, a new one if unknown (counting up, never reused); marks it seen at `now`.
    fn id(&mut self, win: &Ax, now: Instant) -> u64 {
        if let Some(e) = self.entries.iter_mut().find(|e| e.win.same(win)) {
            e.seen = now;
            return e.id;
        }
        self.last_id += 1;
        self.entries.push(Entry {
            win: win.clone(),
            id: self.last_id,
            seen: now,
        });
        self.last_id
    }

    fn get(&self, id: u64) -> Option<Ax> {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.win.clone())
    }

    fn forget_unseen(&mut self, now: Instant) {
        self.entries
            .retain(|e| now.duration_since(e.seen) < FORGET_AFTER);
    }
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry::new());

/// Held only for lookups, never across an AX call (those can block for seconds).
fn registry() -> MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(|e| e.into_inner())
}

/// The registry id of this window, registering it if new (only `collect_windows` registers).
pub fn window_id(win: &Ax) -> u64 {
    registry().id(win, Instant::now())
}

/// The registry id of a window seen before; `None` for anything else (a focused window of another kind).
pub fn known_id(win: &Ax) -> Option<u64> {
    registry().known(win)
}

/// The element behind an id; `None` once it is gone from the registry.
pub fn window(id: u64) -> Option<Ax> {
    registry().get(id)
}

// ---------- Terminals and their windows ----------

/// Running Warp and Ghostty in Swift's order (`NSWorkspace.runningApplications`, documented thread-safe; objc2 does not
/// tie it to the main thread): pid, bundle id, localized name (bundle id if none).
pub fn running_terminals() -> Vec<(Pid, String, String)> {
    // An own pool: Objective-C temporaries go now, not whenever the queue's thread drains.
    objc2::rc::autoreleasepool(|_| {
        NSWorkspace::sharedWorkspace()
            .runningApplications()
            .iter()
            .filter_map(|app| {
                let bid = app.bundleIdentifier()?.to_string();
                if !TERMINAL_BIDS.contains(&bid.as_str()) {
                    return None;
                }
                let name = app
                    .localizedName()
                    .map_or_else(|| bid.clone(), |n| n.to_string());
                Some((app.processIdentifier(), bid, name))
            })
            .collect()
    })
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
    let mut out = vec![];
    for (pid, bundle_id, app) in running_terminals() {
        let origin = ax_origin(&bundle_id);
        for win in Ax::app(pid).elements(WINDOWS) {
            let id = window_id(&win);
            out.push(TermWindow {
                id,
                app: app.clone(),
                bundle_id: bundle_id.clone(),
                owner_pid: pid,
                frame: win
                    .frame()
                    .map_or(Rect::ZERO, |r| ax_to_cocoa(r, origin, screen_h)),
                title: win.string(TITLE).unwrap_or_default(),
                sessions: vec![],
                first_seen: first_seen.get(&id).copied().unwrap_or(now),
                ready_since: None,
                movable: win.settable(POSITION) && win.settable(SIZE),
                minimized: win.bool(MINIMIZED).unwrap_or(false),
                fullscreen: win.bool(FULL_SCREEN).unwrap_or(false),
            });
        }
    }
    registry().forget_unseen(Instant::now());
    out
}

/// One window read fresh (Swift `rereadWindow`): frame, minimized, full screen, title; what cannot be read keeps the old
/// value (minimized and full screen: false). AX queue only.
pub fn reread_window(w: &TermWindow, screen_h: f64) -> TermWindow {
    // Gone from the registry: nothing is readable, as if every read failed.
    let win = window(w.id);
    let origin = ax_origin(&w.bundle_id);
    let flag = |attr| win.as_ref().and_then(|x| x.bool(attr)).unwrap_or(false);
    TermWindow {
        frame: win
            .as_ref()
            .and_then(Ax::frame)
            .map_or(w.frame, |r| ax_to_cocoa(r, origin, screen_h)),
        minimized: flag(MINIMIZED),
        fullscreen: flag(FULL_SCREEN),
        title: win
            .as_ref()
            .and_then(|x| x.string(TITLE))
            .unwrap_or_else(|| w.title.clone()),
        ..w.clone()
    }
}

/// Current title of a window (the ping reads only titles, every 10 ms). AX queue only.
pub fn title(id: u64) -> Option<String> {
    window(id)?.string(TITLE)
}

// ---------- Writing ----------

/// Sets size and position (Cocoa rectangle) and returns the MEASURED frame (Swift `setFrame`): the writes of core
/// `frame_writes` (unknown current frame: position, then size), read back; more than 2 pt off -> once `rescue_writes`,
/// read back again. AX queue only.
pub fn set_frame(id: u64, cocoa: Rect, origin: AxOrigin, screen_h: f64) -> Option<Rect> {
    let win = window(id)?;
    let target = cocoa_to_ax(cocoa, origin, screen_h);
    let writes = win.frame().map_or_else(
        || {
            vec![
                AxWrite::Position(target.origin()),
                AxWrite::Size(target.size()),
            ]
        },
        |current| frame_writes(current, target),
    );
    apply(&win, &writes);
    let got = win.frame()?;
    let measured = ax_to_cocoa(got, origin, screen_h);
    if !frames_differ(measured, cocoa, 2.0) {
        return Some(measured);
    }
    apply(&win, &rescue_writes(got, target));
    win.frame().map(|r| ax_to_cocoa(r, origin, screen_h))
}

/// Only the position, no read-back: the frames on the way of a glide. AX queue only.
pub fn move_position(id: u64, cocoa: Rect, origin: AxOrigin, screen_h: f64) {
    if let Some(win) = window(id) {
        win.write(AxWrite::Position(
            cocoa_to_ax(cocoa, origin, screen_h).origin(),
        ));
    }
}

/// Only the size, top-left stays: shrinking at the old place. AX queue only.
pub fn resize_in_place(id: u64, size: Size) {
    if let Some(win) = window(id) {
        win.write(AxWrite::Size(size));
    }
}

/// Is the window minimized? AX queue only.
pub fn minimized(id: u64) -> Option<bool> {
    window(id)?.bool(MINIMIZED)
}

/// Back from the Dock (`AXMinimized` = false). AX queue only.
pub fn unminimize(id: u64) {
    if let Some(win) = window(id) {
        win.set_bool(MINIMIZED, false);
    }
}

/// `AXRaise` only (orders only within its app, R2-6). Swift raised the middle after parking with it alone: `AXMain` there
/// could take the key window from another Warp window Kevin types in (`set_main` is separate). AX queue only.
pub fn raise(id: u64) {
    if let Some(win) = window(id) {
        win.perform(RAISE);
    }
}

/// `AXMain` = true, for bringing a window to front: `minimized` -> `unminimize`, `raise`, `set_main`,
/// `set_app_frontmost` (Swift `bringToFront`). AX queue only.
pub fn set_main(id: u64) {
    if let Some(win) = window(id) {
        win.set_bool(MAIN, true);
    }
}

/// The app in front via AX (`AXFrontmost` = true on the application element); false = refused (the caller falls back
/// to `NSRunningApplication.activate`). AX queue only.
pub fn set_app_frontmost(pid: Pid) -> bool {
    Ax::app(pid).set_bool(FRONTMOST, true)
}

/// Window `id`'s app in front (`set_app_frontmost`), unless it is in front already with `id` focused. `AXFrontmost` layers
/// ALL the app's windows over other apps' windows, Lookout's list among them, also when the app is in front already:
/// measured 2026-11-11, Warp windows Lookout does not arrange covered the list for 0.1 s to 3 s after every click (Kevin:
/// "dann kommen immer die ganzen Fenster kurz nach vorne"). Call after `raise` and `set_main`. AX queue only.
pub fn bring_app_front(id: u64, pid: Pid) -> bool {
    if focused_app_pid() == Some(pid) && focused_window(pid) == Some(id) {
        return true;
    }
    set_app_frontmost(pid)
}

// ---------- Focus ----------

/// The focused window of an app (`AXFocusedWindow`), as a registry id (`known_id`). AX queue only.
pub fn focused_window(pid: Pid) -> Option<u64> {
    known_id(&Ax::app(pid).element(FOCUSED_WINDOW)?)
}

/// The process of the app with the keyboard focus, read now (system-wide `AXFocusedApplication`): a value remembered on
/// main could be stale while the AX queue still works off a move (R10, review quick 2026-10-04, Codex). AX queue only.
pub fn focused_app_pid() -> Option<Pid> {
    // SAFETY: only creates the system-wide element.
    let system = Ax(unsafe { AXUIElement::new_system_wide() });
    system.element(FOCUSED_APPLICATION)?.pid()
}

// ---------- Access and the window list ----------

/// Accessibility access; `prompt` shows macOS's dialog (`AXIsProcessTrustedWithOptions` with
/// `kAXTrustedCheckOptionPrompt`).
pub fn trusted(prompt: bool) -> bool {
    if !prompt {
        // SAFETY: no arguments, reads this process's access.
        return unsafe { AXIsProcessTrusted() };
    }
    // SAFETY: an immutable string constant of HIServices.
    let key = unsafe { kAXTrustedCheckOptionPrompt };
    let options = CFDictionary::from_slices(&[key], &[CFBoolean::new(true)]);
    // SAFETY: the documented option and value type: kAXTrustedCheckOptionPrompt -> CFBoolean.
    unsafe { AXIsProcessTrustedWithOptions(Some(options.as_opaque())) }
}

/// Windows on the VISIBLE space (`CGWindowListCopyWindowInfo(optionOnScreenOnly | excludeDesktopElements)`), layer 0
/// only, front to back, AX coordinates; `pids` = only these owners, `None` = all apps (for `sits_above`). Geometry and
/// number need no screen-recording access, titles are never read. `None` = the query failed; empty = read, none there.
pub fn on_screen_windows(pids: Option<&BTreeSet<Pid>>) -> Option<Vec<OnScreenWindow>> {
    let options =
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID)?;
    // SAFETY: an array of CF objects; each is type-checked below.
    let list = unsafe { CFRetained::cast_unchecked::<CFArray<CFType>>(list) };
    // SAFETY: immutable string constants of CoreGraphics.
    let (owner_key, layer_key, number_key, bounds_key) = unsafe {
        (
            kCGWindowOwnerPID,
            kCGWindowLayer,
            kCGWindowNumber,
            kCGWindowBounds,
        )
    };
    let mut out = vec![];
    for item in list.iter() {
        // Like Swift's `as? [[String: Any]]`: one entry that is no dictionary fails the whole query.
        let dict = item.downcast::<CFDictionary>().ok()?;
        // SAFETY: CGWindowList's dictionaries have CFString keys and CF values; every value is type-checked when read.
        let dict = unsafe { CFRetained::cast_unchecked::<CFDictionary<CFString, CFType>>(dict) };
        // Swift `as? Int`: a number with an exact integer value.
        let int = |key: &CFString| dict.get(key)?.downcast_ref::<CFNumber>()?.as_i64();
        // Swift `pid_t(owner)` trapped on an owner beyond 32 bits; skipped here.
        let Some(pid) = int(owner_key).and_then(|p| Pid::try_from(p).ok()) else {
            continue;
        };
        if pids.is_some_and(|p| !p.contains(&pid)) || int(layer_key) != Some(0) {
            continue;
        }
        let Some(number) = int(number_key) else {
            continue;
        };
        let Some(bounds) = dict
            .get(bounds_key)
            .and_then(|b| b.downcast::<CFDictionary>().ok())
        else {
            continue;
        };
        let mut r = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0));
        // SAFETY: `bounds` is a dictionary, `r` a valid out pointer.
        if !unsafe { CGRectMakeWithDictionaryRepresentation(Some(&*bounds), &mut r) } {
            continue;
        }
        out.push(OnScreenWindow {
            number,
            pid,
            rect: Rect::new(r.origin.x, r.origin.y, r.size.width, r.size.height),
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Identity as Swift had it with `CFEqual`: a fresh copy of an element finds its id, another element gets another.
    #[test]
    fn registry_names_an_element_by_its_identity() {
        let own = std::process::id() as Pid;
        let (a, b) = (Ax::app(own), Ax::app(own));
        assert!(
            a.same(&b),
            "two fresh elements of one app are the same element"
        );
        let id = window_id(&a);
        assert_eq!(window_id(&b), id, "a fresh copy keeps the id");
        assert_eq!(known_id(&b), Some(id));
        assert!(
            window(id).is_some_and(|w| w.same(&a)),
            "the id leads back to the element"
        );
        let launchd = Ax::app(1);
        assert!(!launchd.same(&a));
        assert_ne!(window_id(&launchd), id, "another app, another id");
        assert_eq!(
            known_id(&Ax::app(Pid::MAX)),
            None,
            "never registered: no id"
        );
    }

    /// A window AX misses for a while keeps its id; one unseen for 10 minutes is forgotten, and its id never returns.
    #[test]
    fn registry_forgets_after_ten_minutes_and_never_reuses_an_id() {
        let mut r = Registry::new();
        let t0 = Instant::now();
        let min = |m: u64| t0 + Duration::from_secs(m * 60);
        let (a, b) = (Ax::app(101), Ax::app(102));
        let (ia, ib) = (r.id(&a, t0), r.id(&b, t0));
        assert_ne!(ia, ib);
        r.id(&a, min(9));
        r.forget_unseen(min(11));
        assert_eq!(r.known(&a), Some(ia), "seen 2 minutes ago: keeps its id");
        assert_eq!(r.known(&b), None, "unseen for 11 minutes: forgotten");
        assert!(r.get(ib).is_none());
        assert!(
            r.id(&b, min(12)) > ia.max(ib),
            "back again: a new id, never a reused one"
        );
    }

    /// Read off the main thread, as on the AX queue: only Warp and Ghostty, each with a pid and a name.
    #[test]
    fn running_terminals_lists_only_terminals() {
        for (pid, bid, name) in running_terminals() {
            assert!(
                pid > 0 && TERMINAL_BIDS.contains(&bid.as_str()) && !name.is_empty(),
                "{pid} {bid} {name}"
            );
        }
    }

    #[test]
    fn on_screen_windows_reads_the_window_list() {
        assert!(
            on_screen_windows(None).is_some(),
            "CGWindowList answers without screen-recording access"
        );
        assert_eq!(
            on_screen_windows(Some(&BTreeSet::new())),
            Some(vec![]),
            "no owners asked for: none kept"
        );
    }

    #[test]
    fn trusted_without_prompt_only_asks() {
        // Either answer is fine here; `trusted(true)` would show macOS's dialog, so it stays out of the tests.
        let _ = trusted(false);
    }
}
