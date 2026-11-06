//! The overlays (Overlay.swift, AppKit part): the calm glow frame around done windows, the labels over AI windows, the
//! clickable lists at the foot of a side, and the preview outlines while the menu is open. What they show is decided in
//! `lookout_core::overlay`; this draws it. Main thread only (`MainThreadMarker`). Colours are sRGB components 0…1
//! (core `color_from_hex`).
//!
//! Design rule (Kevin 2026-09-18): "smoothe Prozesse, die den Geist calm halten, aber informieren": mint instead of red,
//! a thin line with a soft halo, slow breathing (4.5 s) just below notice, every frame out of step with the others,
//! soft fades, nothing jumps.
//!
//! State lives in `RefCell`s. AppKit calls back only into the views (drawing, the list's click), and a view borrows
//! only its own state; the list's click runs `on_click` with no borrow held, so the controller may update the list
//! from inside it.

mod glow;
mod labels;
mod lists;
mod preview;

pub use glow::GlowOverlay;
pub use labels::LabelOverlay;
pub use lists::ListOverlay;
pub use preview::PreviewOverlay;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSFloatingWindowLevel, NSFont, NSFontAttributeName,
    NSFontWeightBold, NSFontWeightRegular, NSFontWeightSemibold, NSForegroundColorAttributeName,
    NSLineBreakMode, NSMutableParagraphStyle, NSNormalWindowLevel, NSPanel, NSParagraphStyle,
    NSParagraphStyleAttributeName, NSResponder, NSScreen, NSStringDrawing, NSWindow,
    NSWindowAnimationBehavior, NSWindowCollectionBehavior, NSWindowOrderingMode, NSWindowStyleMask,
};
use objc2_foundation::{
    NSAttributedStringKey, NSDictionary, NSObject, NSPoint, NSRect, NSSize, NSString,
};

use lookout_core::geom::Rect;
use lookout_core::model::{OnScreenWindow, Pid};
use lookout_core::overlay::{FontWeight, sits_above};

/// sRGB components 0…1.
pub type Rgb = (f64, f64, f64);

/// Window level of an overlay panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// The level of the windows: placed right above its own window, other apps in front cover it (Kevin 2026-09-18:
    /// "wenn Chrome davor ist, soll der Rand auch in den Hintergrund").
    Normal,
    /// Above everything (the preview, while the menu is open).
    Floating,
}

define_class!(
    // SAFETY: NSPanel has no subclassing requirements; no ivars, no `Drop`.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    struct OverlayPanel;

    impl OverlayPanel {
        /// The frame as given. AppKit moves every window it sets: under the menu bar, and back onto its old screen
        /// when it changes screens. An overlay must lie exactly over its window, else the frame sat 7 pt too low at
        /// the top and stuck top left on the MacBook after a move to the LG's bottom edge (Kevin 2026-09-25, probe
        /// −7,−460 instead of −7,−7).
        // SAFETY: the signature of -[NSWindow constrainFrameRect:toScreen:].
        #[unsafe(method(constrainFrameRect:toScreen:))]
        fn constrain_frame_rect(&self, frame: NSRect, _screen: Option<&NSScreen>) -> NSRect {
            frame
        }
    }
);

/// What both panel kinds share: an `OverlayPanel`, borderless and non-activating, clear, without shadow, not hidden on
/// deactivation, no window animation.
fn base_panel(mtm: MainThreadMarker, level: Level) -> Retained<NSPanel> {
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    // SAFETY: NSPanel's initializer with its argument types. A panel is not released on close (NSPanel's default, the
    // AppKit tests check it), so the `Retained` stays its only owner.
    let panel: Retained<OverlayPanel> = unsafe {
        msg_send![
            OverlayPanel::alloc(mtm),
            initWithContentRect: NSRect::ZERO,
            styleMask: style,
            backing: NSBackingStoreType::Buffered,
            defer: false
        ]
    };
    let panel = Retained::into_super(panel);
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setLevel(match level {
        Level::Normal => NSNormalWindowLevel,
        Level::Floating => NSFloatingWindowLevel,
    });
    panel.setHidesOnDeactivate(false);
    panel.setAnimationBehavior(NSWindowAnimationBehavior::None);
    panel
}

/// Click-through, non-activating panel for glow, labels and preview (Swift `makeOverlayPanel`): borderless +
/// non-activating, clear, no shadow, ignores mouse events, not hidden on deactivation, on all spaces + stationary +
/// ignores cycle + full-screen auxiliary, `animationBehavior = .none` (else macOS animates every show and hide and old
/// and new outlines overlap, 2026-09-18), and a subclass whose `constrainFrameRect:toScreen:` returns the frame
/// unchanged (else AppKit pushed it under the menu bar or back onto its old screen, Kevin 2026-09-25).
pub fn make_overlay_panel(mtm: MainThreadMarker, level: Level) -> Retained<NSPanel> {
    let panel = base_panel(mtm, level);
    panel.setIgnoresMouseEvents(true);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    panel
}

/// The list panel (Swift `makeListPanel`): takes clicks without activating Lookout (borderless + non-activating,
/// `becomesKeyOnlyIfNeeded`), normal level, NOT on all spaces (`moveToActiveSpace | ignoresCycle`), no animation, the
/// same frame-keeping subclass.
pub fn make_list_panel(mtm: MainThreadMarker) -> Retained<NSPanel> {
    let panel = base_panel(mtm, Level::Normal);
    panel.setBecomesKeyOnlyIfNeeded(true);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::MoveToActiveSpace | NSWindowCollectionBehavior::IgnoresCycle,
    );
    panel
}

/// Puts a panel on `frame` (Cocoa) right above window `above` (CGWindowID), changing only what differs (Swift `place`):
/// `setFrame` only for a new frame; `orderWindow(.above, relativeTo:)` only if core `sits_above` says another app's
/// window lies between (every order call on another app's window costs WindowServer a rights check with an error line,
/// 284/min until 27.09.). No number: `orderFrontRegardless`.
pub fn place(panel: &NSWindow, frame: Rect, above: Option<i64>, stack: Option<&[OnScreenWindow]>) {
    if core_rect(panel.frame()) != frame {
        panel.setFrame_display(ns_rect(frame), false);
    }
    let Some(number) = above else {
        panel.orderFrontRegardless();
        return;
    };
    let own = std::process::id() as Pid;
    if !sits_above(panel.windowNumber() as i64, number, stack, own) {
        panel.orderWindow_relativeTo(NSWindowOrderingMode::Above, number as isize);
    }
}

fn ns_rect(r: Rect) -> NSRect {
    NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.w, r.h))
}

fn core_rect(r: NSRect) -> Rect {
    Rect::new(r.origin.x, r.origin.y, r.size.width, r.size.height)
}

/// A tint as Swift's `colorFromHex` made it: `NSColor(srgbRed:green:blue:alpha: 1)`.
pub(crate) fn srgb(c: Rgb) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(c.0, c.1, c.2, 1.0)
}

fn white(alpha: f64) -> Retained<NSColor> {
    NSColor::whiteColor().colorWithAlphaComponent(alpha)
}

fn black(alpha: f64) -> Retained<NSColor> {
    NSColor::blackColor().colorWithAlphaComponent(alpha)
}

fn system_font(size: f64, weight: FontWeight) -> Retained<NSFont> {
    // SAFETY: AppKit's weight constants, plain numbers set before any code runs.
    let weight = unsafe {
        match weight {
            FontWeight::Regular => NSFontWeightRegular,
            FontWeight::Semibold => NSFontWeightSemibold,
            FontWeight::Bold => NSFontWeightBold,
        }
    };
    NSFont::systemFontOfSize_weight(size, weight)
}

/// Truncated at the tail, like every line the overlays draw.
fn tail_truncating() -> Retained<NSMutableParagraphStyle> {
    let para = NSMutableParagraphStyle::new();
    para.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    para
}

fn text_attributes(
    font: &NSFont,
    color: &NSColor,
    para: &NSParagraphStyle,
) -> Retained<NSDictionary<NSAttributedStringKey, AnyObject>> {
    // SAFETY: AppKit's attribute names, set before any code runs.
    let keys = unsafe {
        [
            NSFontAttributeName,
            NSForegroundColorAttributeName,
            NSParagraphStyleAttributeName,
        ]
    };
    let values: [&AnyObject; 3] = [font, color, para];
    NSDictionary::from_slices(&keys, &values)
}

/// `NSString.draw(in:withAttributes:)` with font, colour and paragraph style, in the drawing view's coordinates.
fn draw_text(text: &str, rect: Rect, font: &NSFont, color: &NSColor, para: &NSParagraphStyle) {
    let attrs = text_attributes(font, color, para);
    // SAFETY: the dictionary holds the value types AppKit expects for these keys (`text_attributes`).
    unsafe { NSString::from_str(text).drawInRect_withAttributes(ns_rect(rect), Some(&attrs)) };
}
