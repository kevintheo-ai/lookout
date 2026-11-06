//! Global shortcuts Ctrl+Opt+<digit> (Main.swift `installHotkeys`). Carbon `RegisterEventHotKey` instead of an NSEvent
//! monitor: only Carbon CONSUMES the combination; a global monitor would also pass it on to the terminal, where it lands
//! as a control sequence in the running AI session. A few lines of own FFI to Carbon.framework (no crate).

/// Registers Ctrl+Opt+`digits[i]` (e.g. `[1, 2, 3]`, with the parking lot `[1, 2, 3, 4]`); `on_press(i)` runs on the
/// main queue with the index into `digits`. Register only keys that have a task: a registration swallows the combination
/// system-wide. Logs like Swift: per failure "Hotkey Ctrl+Opt+N nicht registrierbar (Status S) — vermutlich belegt",
/// then "Hotkeys registriert: n/m". Main thread.
pub fn install(digits: &[u8], on_press: Box<dyn Fn(usize)>) {
    let _ = (digits, on_press);
    todo!()
}

/// Unregisters all (quit). Main thread.
pub fn remove() {
    todo!()
}
