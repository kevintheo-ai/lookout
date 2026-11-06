//! Focus observation (Overlay.swift `FocusWatcher`): one AXObserver per terminal process on the main run loop
//! (`kAXFocusedWindowChangedNotification`, `kAXApplicationActivatedNotification`). The callback touches no AX
//! (convention 3); it hands `on_change` to the main queue.

use std::collections::BTreeSet;

use lookout_core::model::Pid;

pub struct FocusWatcher(());

impl FocusWatcher {
    /// Main thread. `on_change` runs on the main queue after every reported focus change.
    pub fn new(on_change: Box<dyn Fn()>) -> FocusWatcher {
        let _ = on_change;
        todo!()
    }

    /// Observers for exactly these terminal processes: a restarted terminal (new pid) gets a new one, gone ones are
    /// removed. Nothing to do when the set is unchanged; otherwise logs "FocusWatcher: beobachte N Terminal-Prozess(e)".
    /// Main thread.
    pub fn rebuild(&mut self, pids: &BTreeSet<Pid>) {
        let _ = pids;
        todo!()
    }
}
