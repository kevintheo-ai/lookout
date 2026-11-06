//! The labels over the AI windows (Swift `LabelView`, `LabelPanel`, `LabelOverlay`): click-through, soft fades, no
//! breathing.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ptr::NonNull;

use block2::{RcBlock, StackBlock};
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSBezierPath, NSColor, NSPanel, NSResponder,
    NSView,
};
use objc2_foundation::{NSObject, NSRect};

use lookout_core::geom::Rect;
use lookout_core::model::{LabelTone, OnScreenWindow};
use lookout_core::overlay::{
    LABEL_BG_ALPHA, LABEL_PADDING, LabelContent, LabelTarget, label_line_style,
};

use super::{
    Level, Rgb, black, core_rect, draw_text, make_overlay_panel, place, srgb, system_font,
    tail_truncating, white,
};

struct LabelIvars {
    content: RefCell<Option<LabelContent>>,
    tint: RefCell<Retained<NSColor>>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements; no `Drop`.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = LabelIvars]
    struct LabelView;

    impl LabelView {
        // SAFETY: the signature of -[NSView drawRect:].
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let content = self.ivars().content.borrow();
            let Some(content) = content.as_ref() else {
                return;
            };
            let tint = self.ivars().tint.borrow();
            black(LABEL_BG_ALPHA).setFill();
            let bounds = self.bounds();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 10.0, 10.0).fill();
            let bounds = core_rect(bounds);
            let para = tail_truncating();
            let mut y = bounds.max_y() - LABEL_PADDING;
            for (i, line) in content.lines().iter().enumerate() {
                let style = label_line_style(i);
                y -= style.height;
                let color = match i {
                    0 => tint.clone(),
                    1 => white(1.0),
                    _ => white(0.8),
                };
                draw_text(
                    line,
                    Rect::new(11.0, y, bounds.width() - 22.0, style.height),
                    &system_font(style.size, style.weight),
                    &color,
                    &para,
                );
            }
        }
    }
);

impl LabelView {
    fn new(mtm: MainThreadMarker) -> Retained<LabelView> {
        let this = LabelView::alloc(mtm).set_ivars(LabelIvars {
            content: RefCell::new(None),
            tint: RefCell::new(NSColor::whiteColor()),
        });
        // SAFETY: NSView's designated initializer; Swift's `LabelView()` ends there with a zero frame.
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }

    /// Redraws only for new content or a new tint.
    fn show(&self, content: &LabelContent, tint: Retained<NSColor>) {
        let s = self.ivars();
        if s.content.borrow().as_ref() != Some(content) || *s.tint.borrow() != tint {
            *s.content.borrow_mut() = Some(content.clone());
            *s.tint.borrow_mut() = tint;
            self.setNeedsDisplay(true);
        }
    }
}

struct LabelPanel {
    panel: Retained<NSPanel>,
    view: Retained<LabelView>,
}

impl LabelPanel {
    fn new(mtm: MainThreadMarker) -> LabelPanel {
        // The windows' level, like the glow.
        let panel = make_overlay_panel(mtm, Level::Normal);
        let view = LabelView::new(mtm);
        panel.setContentView(Some(&view));
        panel.setAlphaValue(0.0);
        LabelPanel { panel, view }
    }
}

/// The labels (Swift `LabelOverlay`): one panel per id on `Level::Normal`, black background at `LABEL_BG_ALPHA`, radius
/// 10, the lines of `LabelContent::lines` from the top (padding `LABEL_PADDING`, x inset 11, style per core
/// `label_line_style`, truncated at the tail); line 0 in the tone's colour (waiting -> waiting tint, ready -> ready tint,
/// else white 0.82), line 1 white, the rest white 0.8. New panels fade in over 0.45 s, gone ones fade out over 0.35 s
/// and are ordered out; content or tint change redraws.
pub struct LabelOverlay {
    mtm: MainThreadMarker,
    panels: RefCell<BTreeMap<String, LabelPanel>>,
    ready: RefCell<Retained<NSColor>>,
    waiting: RefCell<Retained<NSColor>>,
    running: Retained<NSColor>,
}

impl LabelOverlay {
    pub fn new(mtm: MainThreadMarker) -> LabelOverlay {
        LabelOverlay {
            mtm,
            panels: RefCell::default(),
            ready: RefCell::new(NSColor::systemGreenColor()),
            waiting: RefCell::new(NSColor::systemOrangeColor()),
            running: NSColor::colorWithWhite_alpha(0.82, 1.0),
        }
    }

    pub fn set_tints(&self, ready: Rgb, waiting: Rgb) {
        *self.ready.borrow_mut() = srgb(ready);
        *self.waiting.borrow_mut() = srgb(waiting);
    }

    pub fn update(&self, targets: &[LabelTarget], stack: Option<&[OnScreenWindow]>) {
        let mut panels = self.panels.borrow_mut();
        for t in targets {
            let is_new = !panels.contains_key(&t.id);
            let p = panels
                .entry(t.id.clone())
                .or_insert_with(|| LabelPanel::new(self.mtm));
            let tint = match t.content.tone {
                LabelTone::Waiting => self.waiting.borrow().clone(),
                LabelTone::Ready => self.ready.borrow().clone(),
                LabelTone::Running | LabelTone::Watching => self.running.clone(),
            };
            p.view.show(&t.content, tint);
            place(&p.panel, t.rect, t.window_number, stack); // like the glow
            if is_new {
                fade(&p.panel, 1.0, false);
            }
        }
        let wanted: BTreeSet<&str> = targets.iter().map(|t| t.id.as_str()).collect();
        panels.retain(|id, p| {
            let keep = wanted.contains(id.as_str());
            if !keep {
                fade(&p.panel, 0.0, true);
            }
            keep
        });
    }

    pub fn hide_all(&self) {
        self.update(&[], None);
    }
}

impl Drop for LabelOverlay {
    /// AppKit keeps a visible panel alive and on screen after its last owner is gone.
    fn drop(&mut self) {
        for p in self.panels.borrow().values() {
            p.panel.orderOut(None);
        }
    }
}

/// A soft fade to `alpha` (Swift `fade`): in over 0.45 s, out over 0.35 s; `order_out` orders the panel out at the end.
fn fade(panel: &Retained<NSPanel>, alpha: f64, order_out: bool) {
    let changes = StackBlock::new(|ctx: NonNull<NSAnimationContext>| {
        // SAFETY: AppKit hands in the group's context, valid during the call.
        unsafe { ctx.as_ref() }.setDuration(if alpha > 0.0 { 0.45 } else { 0.35 });
        panel.animator().setAlphaValue(alpha);
    });
    let done = order_out.then(|| {
        let panel = panel.clone();
        RcBlock::new(move || panel.orderOut(None))
    });
    NSAnimationContext::runAnimationGroup_completionHandler(&changes, done.as_deref());
}
