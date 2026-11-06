//! The overlays (Overlay.swift, AppKit part): the calm glow frame around done windows, the labels over AI windows, the
//! clickable lists at the foot of a side, and the preview outlines while the menu is open. What they show is decided in
//! `lookout_core::overlay`; this draws it. Main thread only (`MainThreadMarker`). Colours are sRGB components 0…1
//! (core `color_from_hex`).
//!
//! Design rule (Kevin 2026-09-18): "smoothe Prozesse, die den Geist calm halten, aber informieren": mint instead of red,
//! a thin line with a soft halo, slow breathing (4.5 s) just below notice, every frame out of step with the others,
//! soft fades, nothing jumps.

use std::collections::{BTreeMap, BTreeSet};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSPanel, NSWindow};

use lookout_core::geom::Rect;
use lookout_core::model::OnScreenWindow;
use lookout_core::overlay::{GlowTarget, LabelTarget, ListTarget, PreviewBox};

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

/// Click-through, non-activating panel for glow, labels and preview (Swift `makeOverlayPanel`): borderless +
/// non-activating, clear, no shadow, ignores mouse events, not hidden on deactivation, on all spaces + stationary +
/// ignores cycle + full-screen auxiliary, `animationBehavior = .none` (else macOS animates every show and hide and old
/// and new outlines overlap, 2026-09-18), and a subclass whose `constrainFrameRect:toScreen:` returns the frame
/// unchanged (else AppKit pushed it under the menu bar or back onto its old screen, Kevin 2026-09-25).
pub fn make_overlay_panel(mtm: MainThreadMarker, level: Level) -> Retained<NSPanel> {
    let _ = (mtm, level);
    todo!()
}

/// The list panel (Swift `makeListPanel`): takes clicks without activating Lookout (borderless + non-activating,
/// `becomesKeyOnlyIfNeeded`), normal level, NOT on all spaces (`moveToActiveSpace | ignoresCycle`), no animation, the
/// same frame-keeping subclass.
pub fn make_list_panel(mtm: MainThreadMarker) -> Retained<NSPanel> {
    let _ = mtm;
    todo!()
}

/// Puts a panel on `frame` (Cocoa) right above window `above` (CGWindowID), changing only what differs (Swift `place`):
/// `setFrame` only for a new frame; `orderWindow(.above, relativeTo:)` only if core `sits_above` says another app's
/// window lies between (every order call on another app's window costs WindowServer a rights check with an error line,
/// 284/min until 27.09.). No number: `orderFrontRegardless`.
pub fn place(panel: &NSWindow, frame: Rect, above: Option<i64>, stack: Option<&[OnScreenWindow]>) {
    let _ = (panel, frame, above, stack);
    todo!()
}

/// The glow around done and asking windows (Swift `GlowOverlay`): one panel per target id on `Level::Normal`, frame =
/// target rect grown by 7 pt on every side, a rounded line (inset 7, radius 11, width 2.5) with a soft outer halo (shadow
/// blur 13, alpha 0.75; line alpha 0.9, then a second stroke without shadow at 0.85). Alpha per core `glow_alpha` with
/// a phase offset that grows by `GLOW_PHASE_STEP` per new panel; animated by a 30 Hz timer that runs only while panels
/// exist; a faded-out panel is ordered out and dropped.
pub struct GlowOverlay(());

impl GlowOverlay {
    pub fn new(mtm: MainThreadMarker) -> GlowOverlay {
        let _ = mtm;
        todo!()
    }

    /// Colour for done (`ready`) and asking (`waiting`); existing panels redraw.
    pub fn set_tints(&self, ready: Rgb, waiting: Rgb) {
        let _ = (ready, waiting);
        todo!()
    }

    /// New ids fade in, missing ones fade out; a changed reason redraws in its colour. `frozen`: ids of gliding windows,
    /// which follow their window via `follow` and are left alone here (a tick with a stale rect would pull them back).
    pub fn update(
        &self,
        targets: &[GlowTarget],
        frozen: &BTreeSet<String>,
        stack: Option<&[OnScreenWindow]>,
    ) {
        let _ = (targets, frozen, stack);
        todo!()
    }

    pub fn hide_all(&self) {
        todo!()
    }

    /// While gliding: the frame moves with its window (Cocoa rect per id, grown by 7 pt), no display.
    pub fn follow(&self, rects: &BTreeMap<String, Rect>) {
        let _ = rects;
        todo!()
    }
}

/// The labels (Swift `LabelOverlay`): one panel per id on `Level::Normal`, black background at `LABEL_BG_ALPHA`, radius
/// 10, the lines of `LabelContent::lines` from the top (padding `LABEL_PADDING`, x inset 11, style per core
/// `label_line_style`, truncated at the tail); line 0 in the tone's colour (waiting -> waiting tint, ready -> ready tint,
/// else white 0.82), line 1 white, the rest white 0.8. New panels fade in over 0.45 s, gone ones fade out over 0.35 s
/// and are ordered out; content or tint change redraws.
pub struct LabelOverlay(());

impl LabelOverlay {
    pub fn new(mtm: MainThreadMarker) -> LabelOverlay {
        let _ = mtm;
        todo!()
    }

    pub fn set_tints(&self, ready: Rgb, waiting: Rgb) {
        let _ = (ready, waiting);
        todo!()
    }

    pub fn update(&self, targets: &[LabelTarget], stack: Option<&[OnScreenWindow]>) {
        let _ = (targets, stack);
        todo!()
    }

    pub fn hide_all(&self) {
        todo!()
    }
}

/// The lists at the foot of a side (Swift `ListOverlay`, plan 1d): at most one panel per list (`make_list_panel`), view
/// flipped (top to bottom), accepts the first mouse, black background at `LABEL_BG_ALPHA`, radius 8; header 12 pt
/// semibold white 0.6; row title `LIST_FONT_SIZE` semibold in its tone's colour (ready/waiting tint, running and
/// watching white 0.78), detail below in white 0.78; "+N" row via core `more_text` in white 0.6; line height 19 pt,
/// x inset 10; a click on a row calls `on_click` with its session ids (core `row_at`).
pub struct ListOverlay(());

impl ListOverlay {
    /// `on_click` runs on the main thread from the click (outside any other call into the overlay).
    pub fn new(mtm: MainThreadMarker, on_click: Box<dyn Fn(Vec<String>)>) -> ListOverlay {
        let _ = (mtm, on_click);
        todo!()
    }

    pub fn set_tints(&self, ready: Rgb, waiting: Rgb) {
        let _ = (ready, waiting);
        todo!()
    }

    /// Panels beyond the targets are ordered out; changed lines or tints redraw.
    pub fn update(&self, targets: &[ListTarget], stack: Option<&[OnScreenWindow]>) {
        let _ = (targets, stack);
        todo!()
    }

    pub fn hide_all(&self) {
        todo!()
    }
}

/// The preview outlines while the menu is open (Swift `PreviewOverlay`): one `Level::Floating` panel per box, drawn once
/// (menu tracking stops default-mode timers): accent colour fill 0.16, dashed line (12, 7) width 3 at 0.9, radius 12,
/// inset 4; centred title (system font, size `bounds.width / 16` clamped to 14…26, semibold, white) and detail (13
/// medium, white 0.85) on a black 0.5 block.
pub struct PreviewOverlay(());

impl PreviewOverlay {
    pub fn new(mtm: MainThreadMarker) -> PreviewOverlay {
        let _ = mtm;
        todo!()
    }

    /// Replaces all outlines.
    pub fn show(&self, boxes: &[PreviewBox]) {
        let _ = boxes;
        todo!()
    }

    pub fn hide(&self) {
        todo!()
    }
}
