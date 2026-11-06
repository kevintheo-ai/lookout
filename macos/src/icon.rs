//! The menu bar symbol (Main.swift `statusTileImage`, plan 1b).

use objc2::rc::Retained;
use objc2_app_kit::NSImage;

/// The mini app icon as a template image (adapts to light and dark), 18×14 pt: a big tile left (rounded rect 1.5,1.5
/// 8×11, radius 2), two small ones right (11.5,7.5 and 11.5,1.5, 5×5, radius 1.5), strokes 1.2 pt; `filled` fills the
/// top right one, where the icon shows the done session.
pub fn status_tile_image(filled: bool) -> Retained<NSImage> {
    let _ = filled;
    todo!()
}
