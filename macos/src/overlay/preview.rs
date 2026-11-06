//! The preview outlines while hovering in the menu (Swift `PreviewView`, `PreviewOverlay`, Kevin 2026-09-18): where
//! the windows WOULD land before Kevin switches. While the menu is open the run loop is in tracking mode and
//! default-mode timers do not fire, so nothing here animates: the outlines are drawn once and stand until the menu
//! highlights another item or closes.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBezierPath, NSColor, NSFont, NSFontWeightMedium, NSLineBreakMode, NSMutableParagraphStyle,
    NSPanel, NSResponder, NSStringDrawing, NSTextAlignment, NSView,
};
use objc2_core_graphics::CGRectInset;
use objc2_foundation::{NSObject, NSRect, NSString};

use lookout_core::geom::Rect;
use lookout_core::overlay::{FontWeight, PreviewBox};

use super::{
    Level, black, core_rect, make_overlay_panel, ns_rect, system_font, text_attributes, white,
};

struct PreviewIvars {
    title: String,
    detail: String,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements; no `Drop`.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = PreviewIvars]
    struct PreviewView;

    impl PreviewView {
        // SAFETY: the signature of -[NSView drawRect:].
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let accent = NSColor::controlAccentColor();
            let outline = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                CGRectInset(self.bounds(), 4.0, 4.0),
                12.0,
                12.0,
            );
            accent.colorWithAlphaComponent(0.16).setFill();
            outline.fill();
            outline.setLineWidth(3.0);
            let dash = [12.0, 7.0];
            // SAFETY: `dash` holds the 2 values the count names; AppKit copies them.
            unsafe { outline.setLineDash_count_phase(dash.as_ptr(), 2, 0.0) };
            accent.colorWithAlphaComponent(0.9).setStroke();
            outline.stroke();

            let bounds = core_rect(self.bounds());
            let para = NSMutableParagraphStyle::new();
            para.setAlignment(NSTextAlignment::Center);
            para.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            let title_size = (bounds.width() / 16.0).clamp(14.0, 26.0);
            let title_font = system_font(title_size, FontWeight::Semibold);
            // SAFETY: an AppKit weight constant, a plain number set before any code runs.
            let detail_font = NSFont::systemFontOfSize_weight(13.0, unsafe { NSFontWeightMedium });
            let title_attrs = text_attributes(&title_font, &NSColor::whiteColor(), &para);
            let detail_attrs = text_attributes(&detail_font, &white(0.85), &para);
            let title = NSString::from_str(&self.ivars().title);
            let detail = NSString::from_str(&self.ivars().detail);
            // SAFETY: both dictionaries hold the value types AppKit expects for their keys (`text_attributes`).
            let th = unsafe { title.sizeWithAttributes(Some(&title_attrs)) }.height;
            // SAFETY: as above.
            let dh = unsafe { detail.sizeWithAttributes(Some(&detail_attrs)) }.height;
            let text_w = (bounds.width() - 32.0).min(520.0);
            let block = Rect::new(
                bounds.mid_x() - text_w / 2.0 - 12.0,
                bounds.mid_y() - (th + dh + 4.0) / 2.0 - 8.0,
                text_w + 24.0,
                th + dh + 4.0 + 16.0,
            );
            // A dark ground behind the text: readable over any window content.
            black(0.5).setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(ns_rect(block), 10.0, 10.0)
                .fill();
            let (x, y) = (block.min_x() + 12.0, block.min_y() + 8.0);
            let detail_rect = Rect::new(x, y, text_w, dh);
            let title_rect = Rect::new(x, y + dh + 4.0, text_w, th);
            // SAFETY: as above.
            unsafe {
                detail.drawInRect_withAttributes(ns_rect(detail_rect), Some(&detail_attrs));
                title.drawInRect_withAttributes(ns_rect(title_rect), Some(&title_attrs));
            }
        }
    }
);

impl PreviewView {
    fn new(mtm: MainThreadMarker, b: &PreviewBox) -> Retained<PreviewView> {
        let this = PreviewView::alloc(mtm).set_ivars(PreviewIvars {
            title: b.title.clone(),
            detail: b.detail.clone(),
        });
        // SAFETY: NSView's designated initializer; Swift's `PreviewView()` ends there with a zero frame.
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }
}

/// The preview outlines while the menu is open (Swift `PreviewOverlay`): one `Level::Floating` panel per box, drawn once
/// (menu tracking stops default-mode timers): accent colour fill 0.16, dashed line (12, 7) width 3 at 0.9, radius 12,
/// inset 4; centred title (system font, size `bounds.width / 16` clamped to 14…26, semibold, white) and detail (13
/// medium, white 0.85) on a black 0.5 block.
pub struct PreviewOverlay {
    mtm: MainThreadMarker,
    panels: RefCell<Vec<Retained<NSPanel>>>,
}

impl PreviewOverlay {
    pub fn new(mtm: MainThreadMarker) -> PreviewOverlay {
        PreviewOverlay {
            mtm,
            panels: RefCell::default(),
        }
    }

    /// Replaces all outlines.
    pub fn show(&self, boxes: &[PreviewBox]) {
        self.hide();
        let mut panels = self.panels.borrow_mut();
        for b in boxes {
            let view = PreviewView::new(self.mtm, b);
            // The preview lies above everything on purpose while the menu is open.
            let panel = make_overlay_panel(self.mtm, Level::Floating);
            panel.setContentView(Some(&view));
            panel.setFrame_display(ns_rect(b.rect), true);
            panel.orderFrontRegardless();
            panels.push(panel);
        }
    }

    pub fn hide(&self) {
        for panel in self.panels.take() {
            panel.orderOut(None);
        }
    }
}

impl Drop for PreviewOverlay {
    /// AppKit keeps a visible panel alive and on screen after its last owner is gone.
    fn drop(&mut self) {
        self.hide();
    }
}
