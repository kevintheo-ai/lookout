//! The click watcher for the peek at the parking lot (plan: "Kevins Klicks im Blick erkennt ein Mausklick-Beobachter,
//! der nur läuft, solange der Blick an ist"): a global left-mouse-down monitor (`NSEvent
//! addGlobalMonitorForEventsMatchingMask`). It sees clicks into OTHER apps' windows (the terminals), never Lookout's own
//! raising, which is not a click.
//!
//! Permission: NSEvent.h (macOS 27 SDK) asks Accessibility only for key-related events, nothing for mouse events. Not
//! verified live. The monitor never sees events sent to Lookout itself (same header).

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask};

use lookout_core::geom::Point;

/// Removes the monitor when dropped.
pub struct ClickMonitor(Option<Retained<AnyObject>>);

/// Starts watching; `on_click` gets the mouse location in Cocoa screen coordinates (`NSEvent.mouseLocation`, origin
/// bottom-left of the primary screen), on the main thread. Main thread.
pub fn watch_clicks(mtm: MainThreadMarker, on_click: Box<dyn Fn(Point)>) -> ClickMonitor {
    // AppKit calls global monitors on the main thread, where `on_click` (not Send) was made.
    let _ = mtm;
    let handler = RcBlock::new(move |_: NonNull<NSEvent>| {
        let at = NSEvent::mouseLocation();
        on_click(Point { x: at.x, y: at.y });
    });
    ClickMonitor(NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        NSEventMask::LeftMouseDown,
        &handler,
    ))
}

impl Drop for ClickMonitor {
    fn drop(&mut self) {
        if let Some(monitor) = &self.0 {
            // SAFETY: the object addGlobalMonitorForEventsMatchingMask returned, as removeMonitor: expects.
            unsafe { NSEvent::removeMonitor(monitor) };
        }
    }
}
