//! Interface language: German or English, chosen once at start (Kevin 2026-09-26: both, automatically). The log stays
//! German.
//!
//! In the app the choice is one global flag. Under `cargo test` it is kept per test thread and starts German, like
//! Swift's test runner set it, so tests that switch to English cannot race the others.

/// Set at start from `prefers_german`.
pub fn set_german(german: bool) {
    flag::set(german);
}

pub fn is_german() -> bool {
    flag::get()
}

/// Interface text in its language.
pub fn tr<'a>(de: &'a str, en: &'a str) -> &'a str {
    if is_german() { de } else { en }
}

/// `language` from the configuration wins over the Mac's language (GP 14); without either, English.
pub fn prefers_german(language: Option<&str>, preferred: &[String]) -> bool {
    language.map_or_else(
        || preferred.first().is_some_and(|p| p.starts_with("de")),
        |l| l == "de",
    )
}

#[cfg(not(test))]
mod flag {
    use std::sync::atomic::{AtomicBool, Ordering};

    static GERMAN: AtomicBool = AtomicBool::new(false);

    pub fn set(german: bool) {
        GERMAN.store(german, Ordering::Relaxed);
    }

    pub fn get() -> bool {
        GERMAN.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod flag {
    use std::cell::Cell;

    thread_local! {
        static GERMAN: Cell<bool> = const { Cell::new(true) };
    }

    pub fn set(german: bool) {
        GERMAN.with(|g| g.set(german));
    }

    pub fn get() -> bool {
        GERMAN.with(Cell::get)
    }
}
