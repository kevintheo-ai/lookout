//! Lookout for macOS: the menu bar app.

#[cfg(target_os = "macos")]
fn main() {
    lookout_macos::controller::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Lookout runs on macOS only for now; the core crate is platform-neutral.");
}
