//! Focus observation (Overlay.swift `FocusWatcher`): one AXObserver per terminal process on the main run loop
//! (`kAXFocusedWindowChangedNotification`, `kAXApplicationActivatedNotification`). The callback touches no AX
//! (convention 3); it hands `on_change` to the main queue.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use objc2_application_services::{AXError, AXObserver, AXUIElement};
use objc2_core_foundation::{CFRetained, CFRunLoop, CFString, kCFRunLoopDefaultMode};

use lookout_core::model::Pid;

use crate::ax::Ax;

thread_local! {
    /// `on_change` of each watcher on this (the main) thread, by watcher id. It is not `Send`, so it stays here; the AX
    /// callback only sends the id to the main queue.
    static ON_CHANGE: RefCell<BTreeMap<usize, Rc<dyn Fn()>>> = const { RefCell::new(BTreeMap::new()) };
}

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

pub struct FocusWatcher {
    /// Its key in `ON_CHANGE`, and the observers' refcon.
    id: usize,
    observers: BTreeMap<Pid, CFRetained<AXObserver>>,
    watched: BTreeSet<Pid>,
}

impl FocusWatcher {
    /// Main thread. `on_change` runs on the main queue after every reported focus change.
    pub fn new(on_change: Box<dyn Fn()>) -> FocusWatcher {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        ON_CHANGE.with(|m| m.borrow_mut().insert(id, Rc::from(on_change)));
        FocusWatcher {
            id,
            observers: BTreeMap::new(),
            watched: BTreeSet::new(),
        }
    }

    /// Observers for exactly these terminal processes: a restarted terminal (new pid) gets a new one, gone ones are
    /// removed. Nothing to do when the set is unchanged; otherwise logs "FocusWatcher: beobachte N Terminal-Prozess(e)".
    /// Main thread.
    pub fn rebuild(&mut self, pids: &BTreeSet<Pid>) {
        if *pids == self.watched {
            return;
        }
        self.observers.retain(|pid, obs| {
            let keep = pids.contains(pid);
            if !keep {
                on_main_loop(obs, false);
            }
            keep
        });
        for &pid in pids {
            if self.observers.contains_key(&pid) {
                continue;
            }
            // Swift: a pid whose observer cannot be made stays unwatched until the set changes.
            if let Some(obs) = observe(pid, self.id) {
                self.observers.insert(pid, obs);
            }
        }
        self.watched = pids.clone();
        crate::log(&format!(
            "FocusWatcher: beobachte {} Terminal-Prozess(e)",
            self.observers.len()
        ));
    }
}

impl Drop for FocusWatcher {
    /// Sources off the main run loop, `on_change` gone: a late callback finds nothing (Swift's watcher lived as long as
    /// the app).
    fn drop(&mut self) {
        for obs in self.observers.values() {
            on_main_loop(obs, false);
        }
        // `try_with`: a watcher dropped while the thread ends finds its map gone already.
        let _ = ON_CHANGE.try_with(|m| m.borrow_mut().remove(&self.id));
    }
}

/// An observer of `pid` for both notifications, its source on the main run loop (Swift `rebuild`, one pid).
fn observe(pid: Pid, id: usize) -> Option<CFRetained<AXObserver>> {
    let mut raw: *mut AXObserver = ptr::null_mut();
    // SAFETY: `raw` is a valid out pointer; `focus_changed` has the AXObserverCallback signature.
    let err = unsafe { AXObserver::create(pid, Some(focus_changed), NonNull::from(&mut raw)) };
    if err != AXError::Success {
        return None;
    }
    let raw = NonNull::new(raw)?;
    // SAFETY: AXObserverCreate hands over a reference we own (create rule).
    let obs = unsafe { CFRetained::from_raw(raw) };
    let app = Ax::app(pid);
    for name in ["AXFocusedWindowChanged", "AXApplicationActivated"] {
        // SAFETY: the refcon is the watcher's id, only ever read back as a number.
        unsafe {
            obs.add_notification(
                &app.0,
                &CFString::from_static_str(name),
                ptr::without_provenance_mut(id),
            )
        };
    }
    on_main_loop(&obs, true);
    Some(obs)
}

/// Adds or removes the observer's source on the main run loop, default mode (Swift `CFRunLoopGetMain()`,
/// `.defaultMode`). Removing the source ends the observation.
fn on_main_loop(obs: &AXObserver, add: bool) {
    let Some(main) = CFRunLoop::main() else {
        return;
    };
    // SAFETY: the observer is alive; `kCFRunLoopDefaultMode` is an immutable CF constant.
    let (source, mode) = unsafe { (obs.run_loop_source(), kCFRunLoopDefaultMode) };
    if add {
        main.add_source(Some(&*source), mode);
    } else {
        main.remove_source(Some(&*source), mode);
    }
}

/// The AX callback, on the main run loop. No AX here (convention 3): only the watcher's id goes to the main queue (Swift
/// `DispatchQueue.main.async`), so `on_change` never runs inside AX's callout.
unsafe extern "C-unwind" fn focus_changed(
    _: NonNull<AXObserver>,
    _: NonNull<AXUIElement>,
    _: NonNull<CFString>,
    refcon: *mut c_void,
) {
    let id = refcon.addr();
    crate::on_main(move || fire(id));
}

/// Runs watcher `id`'s `on_change` if the watcher still exists (Swift `[weak self]`).
fn fire(id: usize) {
    // Out of the map before the call: `on_change` may create or drop a watcher.
    let on_change = ON_CHANGE.with(|m| m.borrow().get(&id).cloned());
    if let Some(f) = on_change {
        f();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// The way of a reported change once it is on the main queue (`fire`): each watcher hears only itself, a dropped one
    /// nothing. Observers themselves need Accessibility access and a running main run loop: the lead's live test.
    #[test]
    fn a_focus_change_reaches_only_its_own_watcher() {
        let heard = Rc::new(Cell::new([0, 0]));
        let counter = |i: usize| -> Box<dyn Fn()> {
            let heard = heard.clone();
            Box::new(move || {
                let mut h = heard.get();
                h[i] += 1;
                heard.set(h);
            })
        };
        let mut a = FocusWatcher::new(counter(0));
        let b = FocusWatcher::new(counter(1));
        a.rebuild(&BTreeSet::new()); // unchanged: no observer, no log line
        fire(a.id);
        assert_eq!(heard.get(), [1, 0]);
        let gone = a.id;
        drop(a);
        fire(gone);
        fire(b.id);
        assert_eq!(heard.get(), [1, 1], "a dropped watcher hears nothing");
    }
}
