//! Lookout for macOS (Rust rewrite). Stage 0 probe: shows a menu bar item, checks whether this
//! signed bundle holds Accessibility access, appends the answer to a probe log, and quits after 3 s.

#[cfg(target_os = "macos")]
fn main() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSStatusBar, NSVariableStatusItemLength,
    };
    use objc2_foundation::ns_string;
    use std::io::Write;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        // Boolean (unsigned char) in C.
        fn AXIsProcessTrusted() -> u8;
    }

    let mtm = MainThreadMarker::new().expect("Lookout must start on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = item.button(mtm) {
        button.setTitle(ns_string!("L·rs"));
    }

    // SAFETY: plain C function without arguments; it only reads the TCC state of this process.
    let trusted = unsafe { AXIsProcessTrusted() } != 0;
    let home = std::env::var("HOME").unwrap_or_default();
    let line = format!(
        "lookout {} pid {} trusted={}\n",
        lookout_core::VERSION,
        std::process::id(),
        trusted
    );
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("{home}/Library/Logs/Lookout-rust-probe.log"))
    {
        let _ = f.write_all(line.as_bytes());
    }

    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(3));
        std::process::exit(0);
    });
    app.run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Lookout runs on macOS only for now; the core crate is platform-neutral.");
}
