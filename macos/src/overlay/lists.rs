//! The lists at the foot of a side (Swift `ListView`, `ListOverlay`, plan 1d, GP 6): what does not fit on a side as a
//! window stands there as a row; a click brings the session to the middle. An own panel kind: the label panels are
//! click-through and lie on every desktop.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBezierPath, NSColor, NSEvent, NSEventPhase, NSLineBreakMode, NSMenu, NSMenuItem,
    NSMutableParagraphStyle, NSPanel, NSResponder, NSScrollElasticity, NSScrollView,
    NSTextAlignment, NSView,
};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize, NSString};

use lookout_core::geom::Rect;
use lookout_core::i18n::tr;
use lookout_core::layout::{LIST_HEADER_H, LIST_ROW_H};
use lookout_core::model::{CardButton, LabelTone, OnScreenWindow};
use lookout_core::overlay::{
    FontWeight, INBOX_ARROW_W, INBOX_BACK_W, INBOX_BACK_X, INBOX_BUTTON_GAP, INBOX_BUTTON_W,
    INBOX_MARGIN, INBOX_OPEN_LINE_H, INBOX_PLUS_W, INBOX_PLUS_X, INBOX_TEXT_X, INBOX_TILE_H,
    INBOX_WHAT_TOP, InboxCard, InboxCounts, InboxNav, InboxTab, InboxTile, LABEL_BG_ALPHA,
    ListLine, ListTarget, PlanMark, WeekStrip, button_at, card_at, more_text, nav_at, plus_at,
    row_at, swipe_step, tile_spans, week_arrows, week_day_span,
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
    on_nav: Rc<dyn Fn(InboxNav)>,
    on_plus: Rc<dyn Fn()>,
    /// The card a right-click menu was opened on.
    menu_card: RefCell<Option<InboxCard>>,
    /// The inbox's cards inside their scroll view: no ground of their own, the tabs view behind them draws it.
    bare: bool,
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

fn stroke_rounded(r: Rect, radius: f64, color: &NSColor) {
    color.setStroke();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(ns_rect(r), radius, radius).stroke();
}

/// An open circle (a task started by hand), 1.5 pt.
fn ring(r: Rect, color: &NSColor) {
    let path = NSBezierPath::bezierPathWithOvalInRect(ns_rect(r));
    path.setLineWidth(1.5);
    color.setStroke();
    path.stroke();
}

/// The name of an area in the head and on its tile.
fn area_name(tab: InboxTab) -> &'static str {
    match tab {
        InboxTab::Overview => tr("Übersicht", "Overview"),
        InboxTab::Waiting => tr("Wartet", "Waiting"),
        InboxTab::Working => tr("Läuft", "Running"),
        InboxTab::Parked => tr("Geparkt", "Parked"),
        InboxTab::Planned => tr("Geplant", "Planned"),
    }
}

/// The inbox's head (Kevin 2026-11-10, "H"): on the overview its title; in an area the way back ("‹ Übersicht"), the
/// area's name and count. The + at the right, a thin line below.
fn draw_head(tab: InboxTab, counts: InboxCounts, y: f64, width: f64) {
    let left = paragraph(NSTextAlignment::Left, false);
    let centre = paragraph(NSTextAlignment::Center, false);
    let title_x = if tab == InboxTab::Overview {
        16.0
    } else {
        let back = Rect::new(INBOX_BACK_X, y + 8.0, INBOX_BACK_W - 6.0, 28.0);
        fill_rounded(back, 7.0, &white(0.08));
        let text = format!("‹ {}", area_name(InboxTab::Overview));
        draw_text(
            &text,
            Rect::new(back.min_x(), y + 13.0, back.width(), 19.0),
            &system_font(14.0, FontWeight::Regular),
            &NSColor::whiteColor(),
            &centre,
        );
        INBOX_BACK_X + INBOX_BACK_W + 6.0
    };
    let count = match tab {
        InboxTab::Overview => None,
        InboxTab::Waiting => Some(counts.waiting),
        InboxTab::Working => Some(counts.working),
        InboxTab::Parked => Some(counts.parked),
        InboxTab::Planned => counts.planned,
    };
    let title = match count {
        Some(n) => format!("{}  {n}", area_name(tab)),
        None => area_name(tab).to_owned(),
    };
    let title_w = (INBOX_PLUS_X - 8.0 - title_x).max(0.0);
    draw_text(
        &title,
        Rect::new(title_x, y + 11.0, title_w, 22.0),
        &system_font(16.0, FontWeight::Semibold),
        &NSColor::whiteColor(),
        &left,
    );
    fill_rounded(
        Rect::new(INBOX_PLUS_X, y + 8.0, INBOX_PLUS_W - 4.0, 28.0),
        7.0,
        &white(0.06),
    );
    let rect = Rect::new(INBOX_PLUS_X, y + 9.0, INBOX_PLUS_W - 4.0, 24.0);
    draw_text(
        "+",
        rect,
        &system_font(19.0, FontWeight::Regular),
        &NSColor::whiteColor(),
        &centre,
    );
    fill_rounded(
        Rect::new(0.0, y + 43.0, width, 1.0),
        0.0,
        &white(INBOX_LINE_ALPHA),
    );
}

/// The amber strip in an area while somebody waits: how many, the first of them, "ansehen ›" (a click opens Waiting).
fn draw_wait_bar(n: usize, text: &str, y: f64, width: f64, tints: &Tints) {
    let amber = tints.color(LabelTone::Waiting);
    fill_rounded(
        Rect::new(0.0, y, width, 35.0),
        0.0,
        &amber.colorWithAlphaComponent(0.16),
    );
    let left = paragraph(NSTextAlignment::Left, false);
    let right = paragraph(NSTextAlignment::Right, false);
    let label = if n == 1 {
        tr("? 1 wartet auf dich", "? 1 is waiting for you").to_owned()
    } else {
        format!("? {n} {}", tr("warten auf dich", "are waiting for you"))
    };
    draw_text(
        &label,
        Rect::new(16.0, y + 8.0, 172.0, 19.0),
        &system_font(14.0, FontWeight::Semibold),
        &amber,
        &left,
    );
    draw_text(
        text,
        Rect::new(192.0, y + 9.0, (width - 192.0 - 96.0).max(0.0), 18.0),
        &system_font(13.5, FontWeight::Regular),
        &white(0.85),
        &left,
    );
    draw_text(
        tr("ansehen ›", "view ›"),
        Rect::new(width - 92.0, y + 9.0, 78.0, 18.0),
        &system_font(13.5, FontWeight::Semibold),
        &amber,
        &right,
    );
}

/// One row of the overview's tiles: an area with its count and what is in it; amber while somebody waits.
fn draw_tiles(tiles: &[InboxTile], y: f64, width: f64, tints: &Tints) {
    let left = paragraph(NSTextAlignment::Left, false);
    let right = paragraph(NSTextAlignment::Right, false);
    let amber = tints.color(LabelTone::Waiting);
    for (tile, (x, w)) in tiles.iter().zip(tile_spans(tiles.len(), width)) {
        let r = Rect::new(x, y, w, INBOX_TILE_H);
        if tile.hot {
            fill_rounded(r, 11.0, &amber.colorWithAlphaComponent(0.13));
            stroke_rounded(r, 11.0, &amber.colorWithAlphaComponent(0.5));
        } else {
            fill_rounded(r, 11.0, &white(0.05));
            stroke_rounded(r, 11.0, &white(0.08));
        }
        let title_color = if tile.hot {
            amber.clone()
        } else {
            NSColor::whiteColor()
        };
        draw_text(
            &tile.title,
            Rect::new(x + 12.0, y + 10.0, w - 70.0, 19.0),
            &system_font(14.5, FontWeight::Semibold),
            &title_color,
            &left,
        );
        draw_text(
            &tile.count.to_string(),
            Rect::new(x + w - 62.0, y + 6.0, 50.0, 26.0),
            &system_font(20.0, FontWeight::Bold),
            &title_color,
            &right,
        );
        let line_color = if tile.hot { white(0.88) } else { white(0.62) };
        for (k, line) in tile.lines.iter().take(2).enumerate() {
            let ly = y + 37.0 + k as f64 * 21.0;
            draw_text(
                line,
                Rect::new(x + 12.0, ly, w - 24.0, 18.0),
                &system_font(13.0, FontWeight::Regular),
                &line_color,
                &left,
            );
        }
    }
}

/// The Planned area's week: arrows and title on top, the seven days below, the chosen one highlighted, today in mint,
/// a dot per item (mint start, amber problem or deadline, open ring a task).
fn draw_week(week: &WeekStrip, y: f64, width: f64, tints: &Tints) {
    let centre = paragraph(NSTextAlignment::Center, false);
    let mint = tints.color(LabelTone::Ready);
    let amber = tints.color(LabelTone::Waiting);
    for ((x, w), arrow) in week_arrows(width).into_iter().zip(["‹", "›"]) {
        fill_rounded(Rect::new(x, y + 6.0, w, 24.0), 6.0, &white(0.06));
        draw_text(
            arrow,
            Rect::new(x, y + 8.0, w, 19.0),
            &system_font(15.0, FontWeight::Semibold),
            &NSColor::whiteColor(),
            &centre,
        );
    }
    let title_x = 10.0 + INBOX_ARROW_W + 4.0;
    draw_text(
        &week.title,
        Rect::new(title_x, y + 9.0, (width - 2.0 * title_x).max(0.0), 18.0),
        &system_font(13.0, FontWeight::Semibold),
        &NSColor::whiteColor(),
        &centre,
    );
    for (k, day) in week.days.iter().enumerate() {
        let (x, w) = week_day_span(k, width);
        if day.selected {
            fill_rounded(
                Rect::new(x + 2.0, y + 36.0, w - 4.0, 56.0),
                9.0,
                &white(0.12),
            );
        }
        draw_text(
            &day.name,
            Rect::new(x, y + 40.0, w, 15.0),
            &system_font(11.5, FontWeight::Regular),
            &white(0.6),
            &centre,
        );
        let number_color = if day.today {
            mint.clone()
        } else {
            NSColor::whiteColor()
        };
        draw_text(
            &day.number.to_string(),
            Rect::new(x, y + 54.0, w, 22.0),
            &system_font(17.0, FontWeight::Semibold),
            &number_color,
            &centre,
        );
        let n = day.dots.len() as f64;
        let mut dx = x + w / 2.0 - (n * 7.0 + (n - 1.0).max(0.0) * 3.0) / 2.0;
        for dot in &day.dots {
            let r = Rect::new(dx, y + 79.0, 7.0, 7.0);
            match dot {
                PlanMark::Start => {
                    mint.setFill();
                    NSBezierPath::bezierPathWithOvalInRect(ns_rect(r)).fill();
                }
                PlanMark::Problem | PlanMark::Deadline => {
                    amber.setFill();
                    NSBezierPath::bezierPathWithOvalInRect(ns_rect(r)).fill();
                }
                PlanMark::Task => ring(Rect::new(dx + 0.75, y + 79.75, 5.5, 5.5), &white(0.78)),
            }
            dx += 10.0;
        }
    }
}

/// The pin before the name of a pinned card, as in the preview Kevin chose from (2026-11-09): a push pin, its outline in
/// a 24-point box, drawn `PIN_SIZE` wide.
const PIN_OUTLINE: [(f64, f64); 14] = [
    (14.5, 2.5),
    (21.5, 9.5),
    (18.9, 10.4),
    (15.5, 13.8),
    (15.8, 18.5),
    (13.9, 20.4),
    (9.7, 16.2),
    (4.4, 21.5),
    (3.0, 20.1),
    (8.3, 14.8),
    (4.1, 10.6),
    (6.0, 8.7),
    (10.7, 9.0),
    (14.1, 5.6),
];
const PIN_SIZE: f64 = 13.0;

fn draw_pin(x: f64, y: f64, color: &NSColor) {
    let path = NSBezierPath::bezierPath();
    let scale = PIN_SIZE / 24.0;
    for (k, &(px, py)) in PIN_OUTLINE.iter().enumerate() {
        let p = NSPoint::new(x + px * scale, y + py * scale);
        if k == 0 {
            path.moveToPoint(p);
        } else {
            path.lineToPoint(p);
        }
    }
    path.closePath();
    color.setFill();
    path.fill();
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
    // A planned item shows its mark: a ring for a task started by hand, a filled circle with ▶ or ! otherwise.
    let symbol = match c.mark {
        None => c.tone.symbol(),
        Some(PlanMark::Start) => "▶",
        Some(PlanMark::Problem | PlanMark::Deadline) => "!",
        Some(PlanMark::Task) => "",
    };
    if c.mark == Some(PlanMark::Task) {
        ring(
            Rect::new(17.0, y + 11.0, GLYPH - 2.0, GLYPH - 2.0),
            &tints.color(c.tone),
        );
    } else {
        tints.color(c.tone).setFill();
        NSBezierPath::bezierPathWithOvalInRect(ns_rect(circle)).fill();
    }
    draw_text(
        symbol,
        Rect::new(circle.min_x(), y + 12.5, GLYPH, 18.0),
        &system_font(if c.mark.is_some() { 11.0 } else { 13.0 }, FontWeight::Bold),
        &NSColor::colorWithWhite_alpha(0.1, 1.0),
        &centre,
    );
    let text_w = width - INBOX_TEXT_X - INBOX_MARGIN;
    let pin_w = if c.pinned {
        draw_pin(INBOX_TEXT_X, y + 11.0, &tints.color(LabelTone::Waiting));
        PIN_SIZE + 5.0
    } else {
        0.0
    };
    line(
        &c.name,
        Rect::new(INBOX_TEXT_X + pin_w, y + 8.0, text_w - 106.0 - pin_w, 20.0),
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
        CardButton::Pin => tr("Anheften", "Pin"),
        CardButton::Unpin => tr("Lösen", "Unpin"),
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
                Some(ListLine::InboxHead { .. })
            );
            if inbox {
                srgb(INBOX_BG).setFill();
            } else {
                black(LABEL_BG_ALPHA).setFill();
            }
            if !self.ivars().bare {
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 8.0, 8.0).fill();
            }
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
                    ListLine::InboxHead { tab, counts } => draw_head(*tab, *counts, y, width),
                    ListLine::WaitBar { n, text } => draw_wait_bar(*n, text, y, width, &tints),
                    ListLine::Tiles(tiles) => draw_tiles(tiles, y, width, &tints),
                    ListLine::Week(week) => draw_week(week, y, width, &tints),
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
            let width = core_rect(self.bounds()).width();
            // Ids copied and the lines' borrow ended before the call: the controller reacts right away and may update
            // this list.
            let (plus, nav, button, row) = {
                let lines = self.ivars().lines.borrow();
                (
                    plus_at(&lines, p.x, p.y),
                    nav_at(&lines, width, p.x, p.y),
                    button_at(&lines, p.x, p.y).map(|(b, ids)| (b, ids.to_vec())),
                    // A planned item has no session behind it: nothing to bring forward.
                    row_at(&lines, p.y).filter(|ids| !ids.is_empty()).map(<[String]>::to_vec),
                )
            };
            if plus {
                let on_plus = Rc::clone(&self.ivars().on_plus);
                on_plus();
            } else if let Some(nav) = nav {
                let on_nav = Rc::clone(&self.ivars().on_nav);
                on_nav(nav);
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
        on_nav: Rc<dyn Fn(InboxNav)>,
        on_plus: Rc<dyn Fn()>,
        bare: bool,
    ) -> Retained<ListView> {
        let this = ListView::alloc(mtm).set_ivars(ListIvars {
            lines: RefCell::default(),
            tints: RefCell::new(tints),
            on_click,
            on_button,
            on_nav,
            on_plus,
            menu_card: RefCell::default(),
            bare,
        });
        // SAFETY: NSView's designated initializer; Swift's `ListView()` ends there with a zero frame.
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }

    /// The right-click menu of the card under `event`, remembering the card for `cardMenuItem:`.
    fn card_menu(&self, event: &NSEvent) -> Option<Retained<NSMenu>> {
        let p = self.convertPoint_fromView(event.locationInWindow(), None);
        let card = card_at(&self.ivars().lines.borrow(), p.y)?.clone();
        if card.menu.is_empty() {
            return None;
        }
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
    inbox: RefCell<Option<InboxPanel>>,
    tints: RefCell<Tints>,
    on_click: Rc<dyn Fn(Vec<String>)>,
    on_button: Rc<dyn Fn(CardButton, Vec<String>)>,
    on_nav: Rc<dyn Fn(InboxNav)>,
    on_plus: Rc<dyn Fn()>,
}

impl ListOverlay {
    /// `on_click`, `on_button` (the buttons on the inbox's open card), `on_nav` (the inbox's way between overview, areas,
    /// weeks and days, also the two-finger swipe back) and `on_plus` (the inbox's +) run on the main thread from the click
    /// (outside any other call into the overlay).
    pub fn new(
        mtm: MainThreadMarker,
        on_click: Box<dyn Fn(Vec<String>)>,
        on_button: Box<dyn Fn(CardButton, Vec<String>)>,
        on_nav: Box<dyn Fn(InboxNav)>,
        on_plus: Box<dyn Fn()>,
    ) -> ListOverlay {
        ListOverlay {
            mtm,
            panels: RefCell::default(),
            inbox: RefCell::default(),
            tints: RefCell::new(Tints {
                ready: NSColor::whiteColor(),
                waiting: NSColor::whiteColor(),
            }),
            on_click: Rc::from(on_click),
            on_button: Rc::from(on_button),
            on_nav: Rc::from(on_nav),
            on_plus: Rc::from(on_plus),
        }
    }

    pub fn set_tints(&self, ready: Rgb, waiting: Rgb) {
        *self.tints.borrow_mut() = Tints {
            ready: srgb(ready),
            waiting: srgb(waiting),
        };
    }

    fn view(&self, tints: &Tints, bare: bool) -> Retained<ListView> {
        ListView::new(
            self.mtm,
            tints.clone(),
            Rc::clone(&self.on_click),
            Rc::clone(&self.on_button),
            Rc::clone(&self.on_nav),
            Rc::clone(&self.on_plus),
            bare,
        )
    }

    /// Panels beyond the targets are ordered out; changed lines or tints redraw. The inbox (the target that starts with its
    /// head) has a panel of its own that scrolls.
    pub fn update(&self, targets: &[ListTarget], stack: Option<&[OnScreenWindow]>) {
        let (inbox, lists): (Vec<&ListTarget>, Vec<&ListTarget>) = targets
            .iter()
            .partition(|t| matches!(t.lines.first(), Some(ListLine::InboxHead { .. })));
        self.update_inbox(inbox.first().copied(), stack);
        let mut panels = self.panels.borrow_mut();
        let tints = self.tints.borrow();
        while panels.len() < lists.len() {
            let view = self.view(&tints, false);
            let panel = make_list_panel(self.mtm);
            panel.setContentView(Some(&view));
            panels.push((panel, view));
        }
        for (t, (panel, view)) in lists.iter().zip(panels.iter()) {
            view.show(&t.lines, &tints);
            place(panel, t.rect, Some(t.window_number), stack);
        }
        for (panel, _) in &panels[lists.len()..] {
            if panel.isVisible() {
                panel.orderOut(None);
            }
        }
    }

    /// The inbox (Kevin 2026-11-10: "das soll scrollbar sein", no "+N more"): the head view is the whole panel and draws
    /// the ground; below the head (and the amber strip, while somebody waits) lies a scroll view whose document view holds
    /// the rest. The head stays, the rest scrolls. Another area starts at its top; in an area a two-finger swipe sideways
    /// leads back to the overview (Kevin 2026-11-10: "mit zwei Fingern zurückwischen").
    fn update_inbox(&self, target: Option<&ListTarget>, stack: Option<&[OnScreenWindow]>) {
        let mut slot = self.inbox.borrow_mut();
        let Some(t) = target else {
            if let Some(p) = slot.as_ref()
                && p.panel.isVisible()
            {
                p.panel.orderOut(None);
            }
            return;
        };
        let tints = self.tints.borrow();
        let p = slot.get_or_insert_with(|| {
            let (tabs, cards) = (self.view(&tints, false), self.view(&tints, true));
            let scroll = InboxScroll::new(self.mtm, Rc::clone(&self.on_nav));
            scroll.setDrawsBackground(false);
            scroll.setHasVerticalScroller(true);
            scroll.setAutohidesScrollers(true);
            scroll.setHorizontalScrollElasticity(NSScrollElasticity::None);
            scroll.setDocumentView(Some(&cards));
            tabs.addSubview(&scroll);
            let panel = make_list_panel(self.mtm);
            panel.setContentView(Some(&tabs));
            InboxPanel {
                panel,
                tabs,
                scroll,
                cards,
                tab: Cell::new(None),
            }
        });
        let fixed = if matches!(t.lines.get(1), Some(ListLine::WaitBar { .. })) {
            2
        } else {
            1
        };
        let (head, body) = t.lines.split_at(fixed.min(t.lines.len()));
        p.tabs.show(head, &tints);
        p.cards.show(body, &tints);
        let top: f64 = head.iter().map(ListLine::height).sum();
        let shown = NSSize::new(t.rect.width(), (t.rect.height() - top).max(0.0));
        p.scroll
            .setFrame(NSRect::new(NSPoint::new(0.0, top), shown));
        let tall: f64 = body.iter().map(ListLine::height).sum();
        // As wide as what the scroller leaves: with "Show scroll bars: Always" it takes room (review quick 2026-11-10).
        let room = p.scroll.contentSize();
        let size = NSSize::new(room.width, tall.max(room.height));
        let now = p.cards.frame().size;
        if now.width != size.width || now.height != size.height {
            p.cards.setFrameSize(size);
            p.cards.setNeedsDisplay(true);
        }
        let tab = match head.first() {
            Some(ListLine::InboxHead { tab, .. }) => Some(*tab),
            _ => None,
        };
        p.scroll
            .ivars()
            .in_area
            .set(tab.is_some_and(|t| t != InboxTab::Overview));
        if p.tab.replace(tab) != tab {
            let clip = p.scroll.contentView();
            clip.scrollToPoint(NSPoint::new(0.0, 0.0));
            p.scroll.reflectScrolledClipView(&clip);
        }
        place(&p.panel, t.rect, Some(t.window_number), stack);
    }

    pub fn hide_all(&self) {
        self.update(&[], None);
    }
}

/// The inbox's panel (`ListOverlay::update_inbox`): the head view as content, the scroll view under the head, the rest
/// as its document view.
struct InboxPanel {
    panel: Retained<NSPanel>,
    tabs: Retained<ListView>,
    scroll: Retained<InboxScroll>,
    cards: Retained<ListView>,
    /// The area shown last.
    tab: Cell<Option<InboxTab>>,
}

struct ScrollIvars {
    on_nav: Rc<dyn Fn(InboxNav)>,
    /// An area is open: a swipe sideways leads back to the overview.
    in_area: Cell<bool>,
    /// The gesture so far: sideways, up or down, and whether it already led back.
    swipe: Cell<(f64, f64, bool)>,
}

define_class!(
    /// The inbox's scroll view: scrolls up and down as ever, and a two-finger swipe sideways in an area leads back to
    /// the overview, like "back" in a browser (Kevin 2026-11-10).
    // SAFETY: NSScrollView has no subclassing requirements; no `Drop`.
    #[unsafe(super(NSScrollView, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ScrollIvars]
    struct InboxScroll;

    impl InboxScroll {
        // SAFETY: the signature of -[NSResponder scrollWheel:].
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            if !self.swiped_back(event) {
                // SAFETY: NSScrollView's own scrolling, same selector and argument.
                let _: () = unsafe { msg_send![super(self), scrollWheel: event] };
            }
        }
    }
);

impl InboxScroll {
    fn new(mtm: MainThreadMarker, on_nav: Rc<dyn Fn(InboxNav)>) -> Retained<InboxScroll> {
        let this = InboxScroll::alloc(mtm).set_ivars(ScrollIvars {
            on_nav,
            in_area: Cell::new(false),
            swipe: Cell::new((0.0, 0.0, false)),
        });
        // SAFETY: NSView's designated initializer, as `NSScrollView::initWithFrame`.
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }

    /// True = this scroll event belongs to a swipe back (now or earlier in the same gesture, its glide too) and is not
    /// scrolled. Only fingers on the trackpad count: a mouse wheel has no phase, the glide after lifting them only a
    /// momentum phase.
    fn swiped_back(&self, event: &NSEvent) -> bool {
        let s = self.ivars();
        let phase = event.phase();
        let began = phase.contains(NSEventPhase::Began) || phase.contains(NSEventPhase::MayBegin);
        let trackpad = !phase.is_empty() || !event.momentumPhase().is_empty();
        let gesture = s.in_area.get() && !phase.is_empty();
        let (dx, dy) = (event.scrollingDeltaX(), event.scrollingDeltaY());
        let (state, mine, back) = swipe_step(s.swipe.get(), began, trackpad, gesture, dx, dy);
        s.swipe.set(state);
        if back {
            let on_nav = Rc::clone(&s.on_nav);
            on_nav(InboxNav::Open(InboxTab::Overview));
        }
        mine
    }
}

impl Drop for ListOverlay {
    /// AppKit keeps a visible panel alive and on screen after its last owner is gone.
    fn drop(&mut self) {
        self.hide_all();
    }
}
