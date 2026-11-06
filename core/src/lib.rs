//! Platform-neutral logic of Lookout: everything that decides where a window goes and what state a
//! session is in. No operating system calls live here, so this crate builds and tests on any platform.

pub mod config;
pub mod geom;
pub mod i18n;
pub mod layout;
pub mod model;
pub mod motion;

#[cfg(test)]
mod testutil;

/// Version shown in the log at start.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert!(!super::VERSION.is_empty());
    }
}
