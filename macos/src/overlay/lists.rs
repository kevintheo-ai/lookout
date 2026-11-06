//! The lists at the foot of a side (Swift `ListView`, `ListOverlay`, plan 1d, GP 6): what does not fit on a side as a
//! window stands there as a row; a click brings the session to the middle. An own panel kind: the label panels are
//! click-through and lie on every desktop.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSBezierPath, NSColor, NSEvent, NSPanel, NSResponder, NSView};
use objc2_foundation::{NSObject, NSRect};

use lookout_core::geom::Rect;
use lookout_core::layout::{LIST_HEADER_H, LIST_ROW_H};
use lookout_core::model::{LabelTone, OnScreenWindow};
use lookout_core::overlay::{FontWeight, LABEL_BG_ALPHA, ListLine, ListTarget, more_text, row_at};

use super::{
    Rgb, black, core_rect, draw_text, make_list_panel, place, srgb, system_font, tail_truncating,
    white,
};

/// The list's font stays 14 pt: its rows have fixed heights (`LIST_ROW_H`) that the layout counts with.
const LIST_FONT_SIZE: f64 = 14.0;
const LIST_LINE_HEIGHT: f64 = 19.0;

/// Row title colours. Done and asking are white until `set_tints` (Swift's `tints[tone] ?? .white`).
#[derive(Clone, PartialEq)]
struct Tints {
    ready: Retained<NSColor>,
    waiting: Retained<NSColor>,
}

impl Tints {
    fn color(&self, tone: LabelTone) -> Retained<NSColor> {
        match tone {
            LabelTone::Ready => self.ready.clone(),
            LabelTone::Waiting => self.waiting.clone(),
            LabelTone::Running | LabelTone::Watching => NSColor::colorWithWhite_alpha(0.78, 1.0),
        }
    }
}

struct ListIvars {
    lines: RefCell<Vec<ListLine>>,
    tints: RefCell<Tints>,
    on_click: Rc<dyn Fn(Vec<String>)>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements; no `Drop`.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ListIvars]
    struct ListView;

    impl ListView {
        /// Top to bottom, as the lines stand.
        // SAFETY: the signature of -[NSView isFlipped].
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// A click acts at once: Lookout, a menu bar app, is not the active app when a row is clicked.
        // SAFETY: the signature of -[NSView acceptsFirstMouse:].
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        // SAFETY: the signature of -[NSView drawRect:].
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            // Readable like the label, whatever lies behind.
            black(LABEL_BG_ALPHA).setFill();
            let bounds = self.bounds();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 8.0, 8.0).fill();
            let width = core_rect(bounds).width();
            let para = tail_truncating();
            let text = |s: &str, y: f64, size: f64, weight: FontWeight, color: &NSColor| {
                let rect = Rect::new(10.0, y, width - 20.0, LIST_LINE_HEIGHT);
                draw_text(s, rect, &system_font(size, weight), color, &para);
            };
            let tints = self.ivars().tints.borrow();
            let mut y = 0.0;
            for line in self.ivars().lines.borrow().iter() {
                match line {
                    ListLine::Header(t) => text(
                        t,
                        y + (LIST_HEADER_H - LIST_LINE_HEIGHT) / 2.0 + 2.0,
                        12.0,
                        FontWeight::Semibold,
                        &white(0.6),
                    ),
                    ListLine::Row {
                        title,
                        detail,
                        tone,
                        ..
                    } => {
                        let title_color = tints.color(*tone);
                        text(title, y + 4.0, LIST_FONT_SIZE, FontWeight::Semibold, &title_color);
                        if let Some(detail) = detail {
                            let y = y + 4.0 + LIST_LINE_HEIGHT;
                            text(detail, y, LIST_FONT_SIZE, FontWeight::Regular, &white(0.78));
                        }
                    }
                    ListLine::More(n) => text(
                        &more_text(*n),
                        y + (LIST_ROW_H - LIST_LINE_HEIGHT) / 2.0,
                        LIST_FONT_SIZE,
                        FontWeight::Regular,
                        &white(0.6),
                    ),
                }
                y += line.height();
            }
        }

        // SAFETY: the signature of -[NSResponder mouseDown:].
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            // Ids copied and the lines' borrow ended before the call: the controller reacts right away and may update
            // this list.
            let ids = row_at(&self.ivars().lines.borrow(), p.y).map(<[String]>::to_vec);
            if let Some(ids) = ids {
                let on_click = Rc::clone(&self.ivars().on_click);
                on_click(ids);
            }
        }
    }
);

impl ListView {
    fn new(
        mtm: MainThreadMarker,
        tints: Tints,
        on_click: Rc<dyn Fn(Vec<String>)>,
    ) -> Retained<ListView> {
        let this = ListView::alloc(mtm).set_ivars(ListIvars {
            lines: RefCell::default(),
            tints: RefCell::new(tints),
            on_click,
        });
        // SAFETY: NSView's designated initializer; Swift's `ListView()` ends there with a zero frame.
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }

    /// Redraws only for new lines or new tints.
    fn show(&self, lines: &[ListLine], tints: &Tints) {
        let s = self.ivars();
        if *s.lines.borrow() != lines || *s.tints.borrow() != *tints {
            *s.lines.borrow_mut() = lines.to_vec();
            *s.tints.borrow_mut() = tints.clone();
            self.setNeedsDisplay(true);
        }
    }
}

/// The lists at the foot of a side (Swift `ListOverlay`, plan 1d): at most one panel per list (`make_list_panel`), view
/// flipped (top to bottom), accepts the first mouse, black background at `LABEL_BG_ALPHA`, radius 8; header 12 pt
/// semibold white 0.6; row title `LIST_FONT_SIZE` semibold in its tone's colour (ready/waiting tint, running and
/// watching white 0.78), detail below in white 0.78; "+N" row via core `more_text` in white 0.6; line height 19 pt,
/// x inset 10; a click on a row calls `on_click` with its session ids (core `row_at`).
pub struct ListOverlay {
    mtm: MainThreadMarker,
    panels: RefCell<Vec<(Retained<NSPanel>, Retained<ListView>)>>,
    tints: RefCell<Tints>,
    on_click: Rc<dyn Fn(Vec<String>)>,
}

impl ListOverlay {
    /// `on_click` runs on the main thread from the click (outside any other call into the overlay).
    pub fn new(mtm: MainThreadMarker, on_click: Box<dyn Fn(Vec<String>)>) -> ListOverlay {
        ListOverlay {
            mtm,
            panels: RefCell::default(),
            tints: RefCell::new(Tints {
                ready: NSColor::whiteColor(),
                waiting: NSColor::whiteColor(),
            }),
            on_click: Rc::from(on_click),
        }
    }

    pub fn set_tints(&self, ready: Rgb, waiting: Rgb) {
        *self.tints.borrow_mut() = Tints {
            ready: srgb(ready),
            waiting: srgb(waiting),
        };
    }

    /// Panels beyond the targets are ordered out; changed lines or tints redraw.
    pub fn update(&self, targets: &[ListTarget], stack: Option<&[OnScreenWindow]>) {
        let mut panels = self.panels.borrow_mut();
        let tints = self.tints.borrow();
        while panels.len() < targets.len() {
            let view = ListView::new(self.mtm, tints.clone(), Rc::clone(&self.on_click));
            let panel = make_list_panel(self.mtm);
            panel.setContentView(Some(&view));
            panels.push((panel, view));
        }
        for (t, (panel, view)) in targets.iter().zip(panels.iter()) {
            view.show(&t.lines, &tints);
            place(panel, t.rect, Some(t.window_number), stack);
        }
        for (panel, _) in &panels[targets.len()..] {
            if panel.isVisible() {
                panel.orderOut(None);
            }
        }
    }

    pub fn hide_all(&self) {
        self.update(&[], None);
    }
}

impl Drop for ListOverlay {
    /// AppKit keeps a visible panel alive and on screen after its last owner is gone.
    fn drop(&mut self) {
        self.hide_all();
    }
}
