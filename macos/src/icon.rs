//! The menu bar symbol (Main.swift `statusTileImage`, plan 1b).

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_app_kit::{NSBezierPath, NSColor, NSImage};
use objc2_foundation::{NSPoint, NSRect, NSSize};

/// The mini app icon as a template image (adapts to light and dark), 18×14 pt: a big tile left (rounded rect 1.5,1.5
/// 8×11, radius 2), two small ones right (11.5,7.5 and 11.5,1.5, 5×5, radius 1.5), strokes 1.2 pt; `filled` fills the
/// top right one, where the icon shows the done session.
pub fn status_tile_image(filled: bool) -> Retained<NSImage> {
    let draw = RcBlock::new(move |_: NSRect| -> Bool {
        let tile = |x, y, w, h, radius| {
            let r = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(r, radius, radius)
        };
        let left = tile(1.5, 1.5, 8.0, 11.0, 2.0);
        let top = tile(11.5, 7.5, 5.0, 5.0, 1.5);
        let bottom = tile(11.5, 1.5, 5.0, 5.0, 1.5);
        NSColor::blackColor().set();
        for p in [&left, &top, &bottom] {
            p.setLineWidth(1.2);
            p.stroke();
        }
        if filled {
            top.fill();
        }
        Bool::YES
    });
    let image =
        NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(18.0, 14.0), false, &draw);
    image.setTemplate(true);
    image
}
