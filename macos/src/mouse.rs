//! The click watcher for the peek at the parking lot (plan: "Kevins Klicks im Blick erkennt ein Mausklick-Beobachter,
//! der nur läuft, solange der Blick an ist"): a global left-mouse-down monitor (`NSEvent
//! addGlobalMonitorForEventsMatchingMask`). It sees clicks into OTHER apps' windows (the terminals), never Lookout's own
//! raising, which is not a click.

use objc2::MainThreadMarker;

use lookout_core::geom::Point;

/// Removes the monitor when dropped.
pub struct ClickMonitor(());

/// Starts watching; `on_click` gets the mouse location in Cocoa screen coordinates (`NSEvent.mouseLocation`, origin
/// bottom-left of the primary screen), on the main thread. Main thread.
pub fn watch_clicks(mtm: MainThreadMarker, on_click: Box<dyn Fn(Point)>) -> ClickMonitor {
    let _ = (mtm, on_click);
    todo!()
}
