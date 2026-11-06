//! The attached screens (NSScreen, main thread) and the cached height of the primary screen, which AX and CGWindowList
//! coordinates refer to (Common.swift `refreshScreenMetrics`, `primaryScreenHeight`; Main.swift `screenInfos`,
//! `isBuiltin`, `screenID`). NSScreen belongs to the main thread, but the AX and work queues convert coordinates: they
//! read only the cache.

use objc2::MainThreadMarker;

use lookout_core::geom::Rect;
use lookout_core::model::ScreenInfo;

/// Every screen in `NSScreen.screens` order: visible frame (without menu bar and Dock, Cocoa), built-in
/// (`CGDisplayIsBuiltin` of `NSScreenNumber`), id = display UUID string (`CGDisplayCreateUUIDFromDisplayID`, survives
/// unplugging), else "display-<number>" (review quick 2026-10-01, GLM), else "" without a number; name =
/// `localizedName`; laptop = internal battery (IOKit `IOPSCopyPowerSourcesInfo` / `IOPSCopyPowerSourcesList` /
/// `IOPSGetPowerSourceDescription`, type "InternalBattery"; read once; unknown = true, the built-in screen is then called
/// MacBook as before, else iMac). Raw: the caller applies core `unique_screen_ids`.
pub fn infos(mtm: MainThreadMarker) -> Vec<ScreenInfo> {
    let _ = mtm;
    todo!()
}

/// Visible frames of all screens, `NSScreen.screens` order.
pub fn visible_frames(mtm: MainThreadMarker) -> Vec<Rect> {
    let _ = mtm;
    todo!()
}

/// Refreshes the cache: height of the primary screen (the one whose frame starts at 0,0; else the first). Main thread
/// only (start, every tick, on screen changes).
pub fn refresh_metrics(mtm: MainThreadMarker) {
    let _ = mtm;
    todo!()
}

/// Cached height of the primary screen: reference point of AX/CG coordinates (top-left). Any thread.
pub fn primary_height() -> f64 {
    todo!()
}
