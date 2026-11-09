//! The lists at the foot of a side (Swift `ListView`, `ListOverlay`, plan 1d, GP 6): what does not fit on a side as a
//! window stands there as a row; a click brings the session to the middle. An own panel kind: the label panels are
//! click-through and lie on every desktop.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBezierPath, NSColor, NSEvent, NSLineBreakMode, NSMenu, NSMenuItem, NSMutableParagraphStyle,
    NSPanel, NSResponder, NSTextAlignment, NSView,
};
use objc2_foundation::{NSObject, NSRect, NSString};

use lookout_core::geom::Rect;
use lookout_core::i18n::tr;
use lookout_core::layout::{LIST_HEADER_H, LIST_ROW_H};
use lookout_core::model::{CardButton, LabelTone, OnScreenWindow};
use lookout_core::overlay::{
    FontWeight, INBOX_BUTTON_GAP, INBOX_BUTTON_W, INBOX_MARGIN, INBOX_OPEN_LINE_H, INBOX_PLUS_W,
    INBOX_PLUS_X, INBOX_TAB_W, INBOX_TAB_X, INBOX_TEXT_X, INBOX_WHAT_TOP, InboxCard, InboxTab,
    LABEL_BG_ALPHA, ListLine, ListTarget, button_at, card_at, more_text, plus_at, row_at, tab_at,
};

use super::{
    Rgb, black, core_rect, draw_text, make_list_panel, ns_rect, place, srgb, system_font,
    tail_truncating, white,
};

/// The list's font stays 14 pt: its rows have fixed heights (`LIST_ROW_H`) that the layout counts with.
const LIST_FONT_SIZE: f64 = 14.0;
const LIST_LINE_HEIGHT: f64 = 19.0;
/// The inbox window: a solid surface, it stands beside the terminal, not over it.
const INBOX_BG: Rgb = (0.149, 0.169, 0.180);
const INBOX_LINE_ALPHA: f64 = 0.09;
const GLYPH: f64 = 22.0;

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
    on_button: Rc<dyn Fn(CardButton, Vec<String>)>,
    on_tab: Rc<dyn Fn(InboxTab)>,
    on_plus: Rc<dyn Fn()>,
    /// The card a right-click menu was opened on.
    menu_card: RefCell<Option<InboxCard>>,
}

fn paragraph(alignment: NSTextAlignment, wrap: bool) -> Retained<NSMutableParagraphStyle> {
    let para = tail_truncating();
    para.setAlignment(alignment);
    if wrap {
        para.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    }
    para
}

fn fill_rounded(r: Rect, radius: f64, color: &NSColor) {
    color.setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(ns_rect(r), radius, radius).fill();
}

/// One card of the inbox, `block` high with the line that belongs to it. Top-down, like the view.
fn draw_card(c: &InboxCard, y: f64, width: f64, block: f64, tints: &Tints) {
    let left = paragraph(NSTextAlignment::Left, false);
    let right = paragraph(NSTextAlignment::Right, false);
    let centre = paragraph(NSTextAlignment::Center, false);
    let line = |s: &str, r: Rect, size: f64, weight: FontWeight, color: &NSColor| {
        draw_text(s, r, &system_font(size, weight), color, &left);
    };
    if c.open {
        fill_rounded(Rect::new(8.0, y, width - 16.0, block), 9.0, &white(0.075));
    }
    let circle = Rect::new(16.0, y + 10.0, GLYPH, GLYPH);
    tints.color(c.tone).setFill();
    NSBezierPath::bezierPathWithOvalInRect(ns_rect(circle)).fill();
    draw_text(
        c.tone.symbol(),
        Rect::new(circle.min_x(), y + 12.5, GLYPH, 18.0),
        &system_font(13.0, FontWeight::Bold),
        &NSColor::colorWithWhite_alpha(0.1, 1.0),
        &centre,
    );
    let text_w = width - INBOX_TEXT_X - INBOX_MARGIN;
    line(
        &c.name,
        Rect::new(INBOX_TEXT_X, y + 8.0, text_w - 106.0, 20.0),
        15.0,
        FontWeight::Semibold,
        &NSColor::whiteColor(),
    );
    if let Some(since) = &c.since {
        draw_text(
            since,
            Rect::new(width - INBOX_MARGIN - 104.0, y + 10.0, 104.0, 18.0),
            &system_font(13.0, FontWeight::Regular),
            &white(0.6),
            &right,
        );
    }
    if let Some(project) = &c.project {
        line(
            project,
            Rect::new(INBOX_TEXT_X, y + 28.0, text_w, 17.0),
            13.0,
            FontWeight::Regular,
            &white(0.6),
        );
    }
    let Some(what) = &c.what else {
        return;
    };
    let top = y + INBOX_WHAT_TOP;
    if c.open {
        let rect = Rect::new(
            INBOX_TEXT_X,
            top,
            text_w,
            c.lines as f64 * INBOX_OPEN_LINE_H,
        );
        let wrap = paragraph(NSTextAlignment::Left, true);
        let font = system_font(16.5, FontWeight::Regular);
        draw_text(what, rect, &font, &NSColor::whiteColor(), &wrap);
    } else {
        let rect = Rect::new(INBOX_TEXT_X, top, text_w, 20.0);
        line(what, rect, 14.5, FontWeight::Regular, &white(0.82));
    }
}

/// The words of a card's command, on a button and in the right-click menu.
fn button_title(button: CardButton) -> &'static str {
    match button {
        CardButton::ReadNext => tr(
            "Gelesen: ausblenden bis zur nächsten Antwort",
            "Read: hide until the next answer",
        ),
        CardButton::Park => tr("Parken", "Park"),
        CardButton::Unpark => tr("Zurückholen", "Bring back"),
    }
}

/// The buttons under the open card, side by side where `button_at` looks for them.
fn draw_buttons(buttons: &[CardButton], y: f64) {
    for (k, b) in buttons.iter().enumerate() {
        let x = INBOX_TEXT_X + k as f64 * (INBOX_BUTTON_W + INBOX_BUTTON_GAP);
        let button = Rect::new(x, y + 3.0, INBOX_BUTTON_W, 28.0);
        let path =
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(ns_rect(button), 7.0, 7.0);
        white(0.06).setFill();
        path.fill();
        white(0.25).setStroke();
        path.stroke();
        draw_text(
            button_title(*b),
            Rect::new(button.min_x(), y + 8.0, button.width(), 19.0),
            &system_font(14.0, FontWeight::Regular),
            &NSColor::whiteColor(),
            &paragraph(NSTextAlignment::Center, false),
        );
    }
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
            // Readable like the label, whatever lies behind; the inbox is a solid surface of its own.
            let bounds = self.bounds();
            let inbox = matches!(
                self.ivars().lines.borrow().first(),
                Some(ListLine::InboxTabs { .. })
            );
            if inbox {
                srgb(INBOX_BG).setFill();
            } else {
                black(LABEL_BG_ALPHA).setFill();
            }
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 8.0, 8.0).fill();
            let width = core_rect(bounds).width();
            let para = tail_truncating();
            let text = |s: &str, y: f64, size: f64, weight: FontWeight, color: &NSColor| {
                let rect = Rect::new(10.0, y, width - 20.0, LIST_LINE_HEIGHT);
                draw_text(s, rect, &system_font(size, weight), color, &para);
            };
            let tints = self.ivars().tints.borrow();
            let mut y = 0.0;
            let lines = self.ivars().lines.borrow();
            for (k, line) in lines.iter().enumerate() {
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
                    ListLine::InboxTabs { tab, waiting, working, parked } => {
                        let tabs = [
                            (InboxTab::Waiting, tr("Wartet", "Waiting"), *waiting),
                            (InboxTab::Working, tr("Läuft", "Running"), *working),
                            (InboxTab::Parked, tr("Geparkt", "Parked"), *parked),
                        ];
                        for (k, (which, label, n)) in tabs.into_iter().enumerate() {
                            let x = INBOX_TAB_X + k as f64 * INBOX_TAB_W;
                            let shown = which == *tab;
                            if shown {
                                fill_rounded(Rect::new(x, y + 8.0, INBOX_TAB_W - 4.0, 28.0), 7.0, &white(0.12));
                            }
                            // On the other tab new questions must not go unnoticed: "Wartet" takes the waiting tint.
                            let color = if which == InboxTab::Waiting && !shown && n > 0 {
                                tints.color(LabelTone::Waiting)
                            } else if shown {
                                NSColor::whiteColor()
                            } else {
                                white(0.6)
                            };
                            let centre = paragraph(NSTextAlignment::Center, false);
                            let rect = Rect::new(x, y + 13.0, INBOX_TAB_W - 4.0, 19.0);
                            draw_text(&format!("{label} {n}"), rect, &system_font(14.0, FontWeight::Semibold), &color, &centre);
                        }
                        fill_rounded(Rect::new(INBOX_PLUS_X, y + 8.0, INBOX_PLUS_W - 4.0, 28.0), 7.0, &white(0.06));
                        let centre = paragraph(NSTextAlignment::Center, false);
                        let rect = Rect::new(INBOX_PLUS_X, y + 9.0, INBOX_PLUS_W - 4.0, 24.0);
                        draw_text("+", rect, &system_font(19.0, FontWeight::Regular), &NSColor::whiteColor(), &centre);
                        fill_rounded(Rect::new(0.0, y + 43.0, width, 1.0), 0.0, &white(INBOX_LINE_ALPHA));
                    }
                    ListLine::Group(t) => {
                        let font = system_font(13.0, FontWeight::Semibold);
                        draw_text(t, Rect::new(16.0, y + 8.0, width - 32.0, 18.0), &font, &white(0.6), &para);
                    }
                    ListLine::Card(c) => {
                        // The highlight takes in the card's own note or button, never the next card.
                        let extra = lines
                            .get(k + 1)
                            .filter(|n| c.open && matches!(n, ListLine::Hint(_) | ListLine::Buttons { .. }))
                            .map_or(0.0, ListLine::height);
                        draw_card(c, y, width, line.height() + extra, &tints);
                    }
                    ListLine::Buttons { buttons, .. } => draw_buttons(buttons, y),
                    ListLine::Hint(t) => {
                        let font = system_font(13.5, FontWeight::Regular);
                        let rect = Rect::new(INBOX_TEXT_X, y + 2.0, width - INBOX_TEXT_X - INBOX_MARGIN, 20.0);
                        draw_text(t, rect, &font, &white(0.6), &para);
                    }
                    ListLine::Empty(t) => {
                        let font = system_font(17.0, FontWeight::Regular);
                        let centre = paragraph(NSTextAlignment::Center, false);
                        draw_text(t, Rect::new(16.0, y + 60.0, width - 32.0, 26.0), &font, &NSColor::whiteColor(), &centre);
                    }
                }
                y += line.height();
            }
        }

        /// A right-click on a card offers what can be done with it (Kevin 2026-11-09).
        // SAFETY: the signature of -[NSView menuForEvent:]; the method returns a retained object as its name's family asks.
        #[unsafe(method_id(menuForEvent:))]
        fn menu_for_event(&self, event: &NSEvent) -> Option<Retained<NSMenu>> {
            self.card_menu(event)
        }

        // SAFETY: an action method: one object argument, no result.
        #[unsafe(method(cardMenuItem:))]
        fn card_menu_item(&self, item: &NSMenuItem) {
            let Some(card) = self.ivars().menu_card.borrow_mut().take() else {
                return;
            };
            let Some(&button) = card.menu.get(item.tag() as usize) else {
                return;
            };
            let on_button = Rc::clone(&self.ivars().on_button);
            on_button(button, card.ids);
        }

        // SAFETY: the signature of -[NSResponder mouseDown:].
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            // Ids copied and the lines' borrow ended before the call: the controller reacts right away and may update
            // this list.
            let (plus, tab, button, row) = {
                let lines = self.ivars().lines.borrow();
                (
                    plus_at(&lines, p.x, p.y),
                    tab_at(&lines, p.x, p.y),
                    button_at(&lines, p.x, p.y).map(|(b, ids)| (b, ids.to_vec())),
                    row_at(&lines, p.y).map(<[String]>::to_vec),
                )
            };
            if plus {
                let on_plus = Rc::clone(&self.ivars().on_plus);
                on_plus();
            } else if let Some(tab) = tab {
                let on_tab = Rc::clone(&self.ivars().on_tab);
                on_tab(tab);
            } else if let Some((button, ids)) = button {
                let on_button = Rc::clone(&self.ivars().on_button);
                on_button(button, ids);
            } else if let Some(ids) = row {
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
        on_button: Rc<dyn Fn(CardButton, Vec<String>)>,
        on_tab: Rc<dyn Fn(InboxTab)>,
        on_plus: Rc<dyn Fn()>,
    ) -> Retained<ListView> {
        let this = ListView::alloc(mtm).set_ivars(ListIvars {
            lines: RefCell::default(),
            tints: RefCell::new(tints),
            on_click,
            on_button,
            on_tab,
            on_plus,
            menu_card: RefCell::default(),
        });
        // SAFETY: NSView's designated initializer; Swift's `ListView()` ends there with a zero frame.
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }

    /// The right-click menu of the card under `event`, remembering the card for `cardMenuItem:`.
    fn card_menu(&self, event: &NSEvent) -> Option<Retained<NSMenu>> {
        let p = self.convertPoint_fromView(event.locationInWindow(), None);
        let card = card_at(&self.ivars().lines.borrow(), p.y)?.clone();
        let mtm = self.mtm();
        let menu = NSMenu::new(mtm);
        for (tag, button) in card.menu.iter().enumerate() {
            // SAFETY: designated initializer on a fresh allocation; the action is a method of this view.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(button_title(*button)),
                    Some(sel!(cardMenuItem:)),
                    &NSString::new(),
                )
            };
            // SAFETY: the target is weak in AppKit; this view outlives the menu it opened.
            unsafe { item.setTarget(Some(self)) };
            item.setTag(tag as isize);
            menu.addItem(&item);
        }
        *self.ivars().menu_card.borrow_mut() = Some(card);
        Some(menu)
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
    on_button: Rc<dyn Fn(CardButton, Vec<String>)>,
    on_tab: Rc<dyn Fn(InboxTab)>,
    on_plus: Rc<dyn Fn()>,
}

impl ListOverlay {
    /// `on_click`, `on_button` (the buttons on the inbox's open card), `on_tab` and `on_plus` (the inbox's +) run on the
    /// main thread from the click (outside any other call into the overlay).
    pub fn new(
        mtm: MainThreadMarker,
        on_click: Box<dyn Fn(Vec<String>)>,
        on_button: Box<dyn Fn(CardButton, Vec<String>)>,
        on_tab: Box<dyn Fn(InboxTab)>,
        on_plus: Box<dyn Fn()>,
    ) -> ListOverlay {
        ListOverlay {
            mtm,
            panels: RefCell::default(),
            tints: RefCell::new(Tints {
                ready: NSColor::whiteColor(),
                waiting: NSColor::whiteColor(),
            }),
            on_click: Rc::from(on_click),
            on_button: Rc::from(on_button),
            on_tab: Rc::from(on_tab),
            on_plus: Rc::from(on_plus),
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
            let view = ListView::new(
                self.mtm,
                tints.clone(),
                Rc::clone(&self.on_click),
                Rc::clone(&self.on_button),
                Rc::clone(&self.on_tab),
                Rc::clone(&self.on_plus),
            );
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
