//! The glow frame around done and asking windows (Swift `GlowView`, `GlowPanel`, `GlowOverlay`).

use std::cell::{Cell, RefCell};
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBezierPath, NSColor, NSGraphicsContext, NSPanel, NSResponder, NSShadow, NSView,
};
use objc2_core_graphics::CGRectInset;
use objc2_foundation::{NSObject, NSRect, NSSize, NSTimer};

use lookout_core::geom::Rect;
use lookout_core::model::{OnScreenWindow, Time};
use lookout_core::overlay::{GLOW_PHASE_STEP, GlowTarget, glow_alpha};

use super::{Level, Rgb, core_rect, make_overlay_panel, ns_rect, place, srgb};

/// The panel around a window: its rectangle grown by 7 pt on every side (Swift `insetBy(dx: -7, dy: -7)`), room for the
/// halo.
fn panel_frame(window: Rect) -> Rect {
    core_rect(CGRectInset(ns_rect(window), -7.0, -7.0))
}

struct GlowIvars {
    tint: RefCell<Retained<NSColor>>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements; no `Drop`.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = GlowIvars]
    struct GlowView;

    impl GlowView {
        // SAFETY: the signature of -[NSView drawRect:].
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            // The glow lies OUTSIDE: the line sits on the window's edge, the shadow frays outwards. A hard frame
            // reads as a warning, a halo does not.
            let tint = self.ivars().tint.borrow().clone();
            let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                CGRectInset(self.bounds(), 7.0, 7.0),
                11.0,
                11.0,
            );
            path.setLineWidth(2.5);

            NSGraphicsContext::saveGraphicsState_class();
            let halo = NSShadow::new();
            halo.setShadowColor(Some(&tint.colorWithAlphaComponent(0.75)));
            halo.setShadowBlurRadius(13.0);
            halo.setShadowOffset(NSSize::ZERO);
            halo.set();
            tint.colorWithAlphaComponent(0.9).setStroke();
            path.stroke();
            NSGraphicsContext::restoreGraphicsState_class();
            // A second stroke without the shadow keeps the line sharp while the glow stays soft.
            tint.colorWithAlphaComponent(0.85).setStroke();
            path.stroke();
        }
    }
);

impl GlowView {
    fn new(mtm: MainThreadMarker, tint: Retained<NSColor>) -> Retained<GlowView> {
        let this = GlowView::alloc(mtm).set_ivars(GlowIvars {
            tint: RefCell::new(tint),
        });
        // SAFETY: NSView's designated initializer; Swift's `GlowView()` ends there with a zero frame.
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }

    fn set_tint(&self, tint: Retained<NSColor>) {
        *self.ivars().tint.borrow_mut() = tint;
        self.setNeedsDisplay(true);
    }
}

struct GlowPanel {
    panel: Retained<NSPanel>,
    view: Retained<GlowView>,
    born_at: Time,
    dying_at: Option<Time>,
    /// Why it glows: (asks, stopped); the colour follows.
    cause: (bool, bool),
    /// Offset in the breathing rhythm: frames that breathe in unison look like a warning light.
    phase: f64,
}

impl GlowPanel {
    fn new(
        mtm: MainThreadMarker,
        tint: Retained<NSColor>,
        cause: (bool, bool),
        phase: f64,
    ) -> GlowPanel {
        let view = GlowView::new(mtm, tint);
        // The windows' level; `update` orders it.
        let panel = make_overlay_panel(mtm, Level::Normal);
        panel.setContentView(Some(&view));
        panel.setAlphaValue(0.0);
        GlowPanel {
            panel,
            view,
            born_at: crate::now(),
            dying_at: None,
            cause,
            phase,
        }
    }
}

/// The glow around done and asking windows (Swift `GlowOverlay`): one panel per target id on `Level::Normal`, frame =
/// target rect grown by 7 pt on every side, a rounded line (inset 7, radius 11, width 2.5) with a soft outer halo (shadow
/// blur 13, alpha 0.75; line alpha 0.9, then a second stroke without shadow at 0.85). Alpha per core `glow_alpha` with
/// a phase offset that grows by `GLOW_PHASE_STEP` per new panel; animated by a 30 Hz timer that runs only while panels
/// exist; a faded-out panel is ordered out and dropped.
pub struct GlowOverlay(Rc<Glow>);

/// Shared with the timer, which holds it weakly (Swift `[weak self]`).
struct Glow {
    mtm: MainThreadMarker,
    panels: RefCell<BTreeMap<String, GlowPanel>>,
    ticker: RefCell<Option<Retained<NSTimer>>>,
    ready: RefCell<Retained<NSColor>>,
    waiting: RefCell<Retained<NSColor>>,
    next_phase: Cell<f64>,
}

impl GlowOverlay {
    pub fn new(mtm: MainThreadMarker) -> GlowOverlay {
        GlowOverlay(Rc::new(Glow {
            mtm,
            panels: RefCell::default(),
            ticker: RefCell::default(),
            ready: RefCell::new(NSColor::systemGreenColor()),
            waiting: RefCell::new(NSColor::systemOrangeColor()),
            next_phase: Cell::new(0.0),
        }))
    }

    /// Colour for done (`ready`) and asking (`waiting`); existing panels redraw.
    pub fn set_tints(&self, ready: Rgb, waiting: Rgb) {
        let g = &self.0;
        *g.ready.borrow_mut() = srgb(ready);
        *g.waiting.borrow_mut() = srgb(waiting);
        for p in g.panels.borrow().values() {
            p.view.set_tint(g.tint(p.cause));
        }
    }

    /// New ids fade in, missing ones fade out; a changed reason redraws in its colour. `frozen`: ids of gliding windows,
    /// which follow their window via `follow` and are left alone here (a tick with a stale rect would pull them back).
    pub fn update(
        &self,
        targets: &[GlowTarget],
        frozen: &BTreeSet<String>,
        stack: Option<&[OnScreenWindow]>,
    ) {
        let g = &self.0;
        let mut panels = g.panels.borrow_mut();
        for t in targets.iter().filter(|t| !frozen.contains(&t.id)) {
            let p = match panels.entry(t.id.clone()) {
                Entry::Occupied(e) => {
                    let p = e.into_mut();
                    if p.cause != (t.waiting, t.stopped) {
                        // The cause changed: the colour follows.
                        p.cause = (t.waiting, t.stopped);
                        p.view.set_tint(g.tint(p.cause));
                    }
                    p
                }
                Entry::Vacant(e) => {
                    g.next_phase.set(g.next_phase.get() + GLOW_PHASE_STEP);
                    e.insert(GlowPanel::new(
                        g.mtm,
                        g.tint((t.waiting, t.stopped)),
                        (t.waiting, t.stopped),
                        g.next_phase.get(),
                    ))
                }
            };
            p.dying_at = None;
            // When Kevin activates the terminal, macOS lifts its window above the panel; the next call puts the frame
            // back on top (`place` does that only then, not on every call).
            place(&p.panel, panel_frame(t.rect), t.window_number, stack);
        }
        let wanted: BTreeSet<&str> = targets
            .iter()
            .map(|t| t.id.as_str())
            .chain(frozen.iter().map(String::as_str))
            .collect();
        let now = crate::now();
        for (id, p) in panels.iter_mut() {
            if p.dying_at.is_none() && !wanted.contains(id.as_str()) {
                p.dying_at = Some(now);
            }
        }
        let empty = panels.is_empty();
        drop(panels);
        if empty { g.stop() } else { g.start() }
    }

    pub fn hide_all(&self) {
        self.update(&[], &BTreeSet::new(), None);
    }

    /// While gliding: the frame moves with its window (Cocoa rect per id, grown by 7 pt), no display.
    pub fn follow(&self, rects: &BTreeMap<String, Rect>) {
        let panels = self.0.panels.borrow();
        for (id, &r) in rects {
            if let Some(p) = panels.get(id) {
                p.panel.setFrame_display(ns_rect(panel_frame(r)), false);
            }
        }
    }
}

impl Drop for GlowOverlay {
    /// Else the timer ticks on for nothing, and AppKit keeps the visible panels alive and on screen.
    fn drop(&mut self) {
        self.0.stop();
        for p in self.0.panels.borrow().values() {
            p.panel.orderOut(None);
        }
    }
}

impl Glow {
    /// Asking in its colour, stopped in the stopped blue-grey (Kevin 2026-11-10, "2a"), done in its colour.
    fn tint(&self, (waiting, stopped): (bool, bool)) -> Retained<NSColor> {
        if waiting {
            self.waiting.borrow().clone()
        } else if stopped {
            srgb(super::STOPPED)
        } else {
            self.ready.borrow().clone()
        }
    }

    /// The 30 Hz ticker, in the run loop's default mode like Swift's `scheduledTimer`: while a menu is open the frames
    /// stand still.
    fn start(self: &Rc<Self>) {
        if self.ticker.borrow().is_some() {
            return;
        }
        let glow = Rc::downgrade(self);
        let block = RcBlock::new(move |_: NonNull<NSTimer>| {
            if let Some(glow) = glow.upgrade() {
                glow.animate();
            }
        });
        // SAFETY: scheduled on the run loop of this thread, the main thread (`mtm`): the block only ever runs there.
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(1.0 / 30.0, true, &block)
        };
        *self.ticker.borrow_mut() = Some(timer);
    }

    fn stop(&self) {
        if let Some(timer) = self.ticker.take() {
            timer.invalidate();
        }
    }

    fn animate(&self) {
        let now = crate::now();
        let mut panels = self.panels.borrow_mut();
        panels.retain(
            |_, p| match glow_alpha(now, p.born_at, p.dying_at, p.phase) {
                Some(alpha) => {
                    p.panel.setAlphaValue(alpha.max(0.0));
                    true
                }
                None => {
                    p.panel.orderOut(None);
                    false
                }
            },
        );
        let empty = panels.is_empty();
        drop(panels);
        if empty {
            self.stop();
        }
    }
}
