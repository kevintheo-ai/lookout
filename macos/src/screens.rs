//! The attached screens (NSScreen, main thread) and the cached height of the primary screen, which AX and CGWindowList
//! coordinates refer to (Common.swift `refreshScreenMetrics`, `primaryScreenHeight`; Main.swift `screenInfos`,
//! `isBuiltin`, `screenID`). NSScreen belongs to the main thread, but the AX and work queues convert coordinates: they
//! read only the cache.

use std::ptr::NonNull;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;
use objc2_core_foundation::{CFArray, CFDictionary, CFRetained, CFString, CFType, CFUUID, CGRect};
use objc2_core_graphics::CGDisplayIsBuiltin;
use objc2_foundation::{NSNumber, ns_string};

use lookout_core::geom::Rect;
use lookout_core::model::ScreenInfo;

#[link(name = "ColorSync", kind = "framework")]
unsafe extern "C" {
    /// ColorSyncDevice.h. Create rule; NULL when macOS has no UUID for the display.
    fn CGDisplayCreateUUIDFromDisplayID(display: u32) -> Option<NonNull<CFUUID>>;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    // IOPowerSources.h: the blob and the list follow the Create rule, a description belongs to the blob.
    fn IOPSCopyPowerSourcesInfo() -> Option<NonNull<CFType>>;
    fn IOPSCopyPowerSourcesList(blob: &CFType) -> Option<NonNull<CFArray<CFType>>>;
    fn IOPSGetPowerSourceDescription(
        blob: &CFType,
        source: &CFType,
    ) -> Option<NonNull<CFDictionary<CFString, CFType>>>;
}

/// f64 bits of the primary screen's height; 0 bits = 0.0 until the first refresh (Swift `cachedPrimaryHeight = 0`).
static PRIMARY_HEIGHT: AtomicU64 = AtomicU64::new(0);

/// Every screen in `NSScreen.screens` order: visible frame (without menu bar and Dock, Cocoa), built-in
/// (`CGDisplayIsBuiltin` of `NSScreenNumber`), id = display UUID string (`CGDisplayCreateUUIDFromDisplayID`, survives
/// unplugging), else "display-<number>" (review quick 2026-10-01, GLM), else "" without a number; name =
/// `localizedName`; laptop = internal battery (IOKit `IOPSCopyPowerSourcesInfo` / `IOPSCopyPowerSourcesList` /
/// `IOPSGetPowerSourceDescription`, type "InternalBattery"; read once; unknown = true, the built-in screen is then called
/// MacBook as before, else iMac). Raw: the caller applies core `unique_screen_ids`.
pub fn infos(mtm: MainThreadMarker) -> Vec<ScreenInfo> {
    let laptop = is_laptop();
    NSScreen::screens(mtm)
        .iter()
        .map(|s| {
            let number = screen_number(&s);
            ScreenInfo {
                visible: rect(s.visibleFrame()),
                builtin: number.is_some_and(|n| CGDisplayIsBuiltin(n)),
                id: number.map_or_else(String::new, display_id),
                name: s.localizedName().to_string(),
                laptop,
            }
        })
        .collect()
}

/// Visible frames of all screens, `NSScreen.screens` order.
pub fn visible_frames(mtm: MainThreadMarker) -> Vec<Rect> {
    NSScreen::screens(mtm)
        .iter()
        .map(|s| rect(s.visibleFrame()))
        .collect()
}

/// Refreshes the cache: height of the primary screen (the one whose frame starts at 0,0; else the first). Main thread
/// only (start, every tick, on screen changes).
pub fn refresh_metrics(mtm: MainThreadMarker) {
    let screens = NSScreen::screens(mtm);
    let primary = screens
        .iter()
        .find(|s| {
            let o = s.frame().origin;
            o.x == 0.0 && o.y == 0.0
        })
        .or_else(|| screens.firstObject());
    let height = primary.map_or(0.0, |s| rect(s.frame()).height());
    PRIMARY_HEIGHT.store(height.to_bits(), Ordering::Relaxed);
}

/// Cached height of the primary screen: reference point of AX/CG coordinates (top-left). Any thread.
pub fn primary_height() -> f64 {
    f64::from_bits(PRIMARY_HEIGHT.load(Ordering::Relaxed))
}

fn rect(r: CGRect) -> Rect {
    Rect::new(r.origin.x, r.origin.y, r.size.width, r.size.height)
}

/// `deviceDescription["NSScreenNumber"]`, the CGDirectDisplayID.
fn screen_number(s: &NSScreen) -> Option<u32> {
    let n = s
        .deviceDescription()
        .objectForKey(ns_string!("NSScreenNumber"))?;
    n.downcast::<NSNumber>().ok().map(|n| n.as_u32())
}

/// Swift `screenID`: the display UUID (from vendor, model and serial number), else the display number, which holds
/// only until unplugging but keeps the screen selectable.
fn display_id(number: u32) -> String {
    // SAFETY: any number is a valid argument; NULL comes back as None.
    let Some(uuid) = (unsafe { CGDisplayCreateUUIDFromDisplayID(number) }) else {
        return format!("display-{number}");
    };
    // SAFETY: Create rule: this reference is ours, CFRetained releases it.
    let uuid = unsafe { CFRetained::from_raw(uuid) };
    CFUUID::new_string(None, Some(&uuid))
        .map_or_else(|| format!("display-{number}"), |s| s.to_string())
}

/// Swift `isLaptop`: an internal battery among the power sources; unknown counts as laptop. Read once.
fn is_laptop() -> bool {
    static LAPTOP: OnceLock<bool> = OnceLock::new();
    *LAPTOP.get_or_init(|| {
        // SAFETY: no arguments; NULL comes back as None.
        let Some(info) = (unsafe { IOPSCopyPowerSourcesInfo() }) else {
            return true;
        };
        // SAFETY: Create rule: this reference is ours.
        let info = unsafe { CFRetained::from_raw(info) };
        // SAFETY: `info` is the blob from IOPSCopyPowerSourcesInfo.
        let Some(list) = (unsafe { IOPSCopyPowerSourcesList(&info) }) else {
            return true;
        };
        // SAFETY: Create rule: this reference is ours.
        let list = unsafe { CFRetained::from_raw(list) };
        // kIOPSTypeKey and kIOPSInternalBatteryType (IOPSKeys.h).
        let type_key = CFString::from_static_str("Type");
        list.iter().any(|source| {
            // SAFETY: the blob and one of its own sources.
            let Some(desc) = (unsafe { IOPSGetPowerSourceDescription(&info, &source) }) else {
                return false;
            };
            // SAFETY: Get rule: the dictionary lives as long as `info`, which outlives this closure.
            let desc = unsafe { desc.as_ref() };
            desc.get(&type_key)
                .and_then(|t| t.downcast::<CFString>().ok())
                .is_some_and(|t| t.to_string() == "InternalBattery")
        })
    })
}
