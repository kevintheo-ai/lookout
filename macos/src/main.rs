//! Lookout for macOS (Rust rewrite). Without arguments the menu bar app. `--probe` (build-rust.sh --probe): checks
//! whether this signed bundle holds Accessibility access, appends the answer to
//! `~/Library/Logs/Lookout-rust-probe.log` and quits.

#[cfg(target_os = "macos")]
fn main() {
    if std::env::args().any(|a| a == "--probe") {
        probe();
    } else {
        lookout_macos::controller::run();
    }
}

#[cfg(target_os = "macos")]
fn probe() {
    use std::io::Write;
    let line = format!(
        "lookout {} pid {} trusted={}\n",
        lookout_core::VERSION,
        std::process::id(),
        lookout_macos::ax::trusted(false)
    );
    let path = format!(
        "{}/Library/Logs/Lookout-rust-probe.log",
        lookout_macos::home()
    );
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Lookout runs on macOS only for now; the core crate is platform-neutral.");
}
