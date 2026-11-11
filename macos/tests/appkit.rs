//! AppKit tests of the overlays (Swift tests/LayoutTests.swift). AppKit wants the main thread, so this is its own
//! harness (`harness = false`): `main` runs each test inside an autorelease pool, catches its panic, prints one line
//! per test and exits 1 if any failed. Every panel stays invisible and lets clicks through (`invisible`): the tests run
//! while someone works at the same Mac. Every test orders its panels out, the overlays do it when dropped.

mod invisible;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::{Rc, Weak};

use objc2::MainThreadMarker;
use objc2::rc::{Retained, autoreleasepool};
use objc2_app_kit::{
    NSApplication, NSBorderType, NSColor, NSEvent, NSEventModifierFlags, NSEventType,
    NSFloatingWindowLevel, NSNormalWindowLevel, NSScreen, NSScrollView, NSScrollerStyle, NSView,
    NSWindow, NSWindowAnimationBehavior, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSDate, NSPoint, NSRect, NSRunLoop, NSSize};

use lookout_core::geom::Rect;
use lookout_core::model::{CardButton, LabelTone, OnScreenWindow, Pid};
use lookout_core::overlay::{
    GlowTarget, InboxCard, InboxCounts, InboxNav, InboxTab, InboxTile, LabelContent, LabelTarget,
    ListLine, ListTarget, PlanMark, PreviewBox, WeekDay, WeekStrip,
};
use lookout_macos::overlay::{
    GlowOverlay, LabelOverlay, Level, ListOverlay, PreviewOverlay, make_list_panel,
    make_overlay_panel, place,
};

const MINT: (f64, f64, f64) = (0.369, 0.812, 0.627);
const ORANGE: (f64, f64, f64) = (1.0, 0.6, 0.2);

type Test = (&'static str, fn(MainThreadMarker));

fn main() {
    // SAFETY: the first statement, no other thread runs yet. Whatever logs here must not reach Kevin's log.
    unsafe {
        std::env::set_var(
            "LOOKOUT_LOG",
            std::env::temp_dir().join("lookout-appkit-test.log"),
        )
    };
    let mtm = MainThreadMarker::new().expect("tests run on the main thread");
    let _ = NSApplication::sharedApplication(mtm);
    invisible::keep_windows_invisible(mtm);
    let tests: [Test; 10] = [
        (
            "overlay_panels_do_not_animate",
            overlay_panels_do_not_animate,
        ),
        (
            "overlay_panels_stay_where_told",
            overlay_panels_stay_where_told,
        ),
        ("glow_follows_gliding_window", glow_follows_gliding_window),
        ("list_panel_takes_clicks", list_panel_takes_clicks),
        ("list_click_reaches_on_click", list_click_reaches_on_click),
        ("inbox_scrolls_below_its_tabs", inbox_scrolls_below_its_tabs),
        (
            "inbox_tiles_and_strip_lead_on",
            inbox_tiles_and_strip_lead_on,
        ),
        ("every_panel_kind_draws", every_panel_kind_draws),
        ("fades_end_off_screen", fades_end_off_screen),
        (
            "place_orders_only_when_needed",
            place_orders_only_when_needed,
        ),
    ];
    let mut failed = 0;
    for (name, test) in tests {
        let ok = catch_unwind(AssertUnwindSafe(|| autoreleasepool(|_| test(mtm)))).is_ok();
        println!("{} {name}", if ok { "  ok  " } else { "  FAIL" });
        failed += usize::from(!ok);
    }
    println!("{} passed, {failed} failed", tests.len() - failed);
    if failed > 0 {
        std::process::exit(1);
    }
}

fn ns(r: Rect) -> NSRect {
    NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.w, r.h))
}

fn rect(r: NSRect) -> Rect {
    Rect::new(r.origin.x, r.origin.y, r.size.width, r.size.height)
}

/// The glow panel around a window: grown by 7 pt on every side.
fn grown(r: Rect) -> Rect {
    Rect::new(r.x - 7.0, r.y - 7.0, r.w + 14.0, r.h + 14.0)
}

/// The app's window with this frame. The overlays keep their panels to themselves; Swift's test searched the same way.
fn window_at(mtm: MainThreadMarker, frame: Rect) -> Retained<NSWindow> {
    NSApplication::sharedApplication(mtm)
        .windows()
        .iter()
        .find(|w| rect(w.frame()) == frame)
        .unwrap_or_else(|| panic!("no window at {frame:?}"))
}

/// Runs the main run loop (default mode: timers and animations) for `seconds`.
fn run_loop(seconds: f64) {
    NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(seconds));
}

/// Swift's `defer { p.orderOut(nil) }`: AppKit keeps a visible window alive after its last owner is gone.
struct OrderOut<'a>(&'a NSWindow);

impl Drop for OrderOut<'_> {
    fn drop(&mut self) {
        self.0.orderOut(None);
    }
}

/// Swift `testOverlayPanelsDoNotAnimate`: macOS' window animation would let old and new outlines overlap.
fn overlay_panels_do_not_animate(mtm: MainThreadMarker) {
    for (level, number) in [
        (Level::Normal, NSNormalWindowLevel),
        (Level::Floating, NSFloatingWindowLevel),
    ] {
        let p = make_overlay_panel(mtm, level);
        assert_eq!(p.animationBehavior(), NSWindowAnimationBehavior::None);
        assert!(p.ignoresMouseEvents() && p.level() == number);
        // The `Retained` is the panel's only owner: AppKit must not release it on `close` as well.
        assert!(!p.isReleasedWhenClosed());
    }
}

/// Swift `testOverlayPanelsStayWhereTold` (Kevin 2026-09-25): AppKit moved overlay panels under the menu bar (the
/// frame sat 7 pt too low at the top) and, when they changed screens, back onto the MacBook (probe −7,−460 instead of
/// −7,−7).
fn overlay_panels_stay_where_told(mtm: MainThreadMarker) {
    let screens = NSScreen::screens(mtm);
    let Some(first) = screens.firstObject() else {
        return;
    };
    let p = make_overlay_panel(mtm, Level::Normal);
    let _out = OrderOut(&p);
    let place = |r: Rect| {
        p.setFrame_display(ns(r), false);
        p.orderFrontRegardless();
        rect(p.frame())
    };
    // Top edge 7 pt into the menu bar: the glow panel of a window at the top of the screen.
    let vis = rect(first.visibleFrame());
    let top = Rect::new(vis.min_x() + 40.0, vis.max_y() - 293.0, 400.0, 300.0);
    assert_eq!(place(top), top, "moved at the top");
    // Every other screen (the MacBook below the LG): from there onto a frame that touches the first screen's bottom
    // edge and reaches 7 pt into the other one, the case from the screenshot.
    let f = rect(first.frame());
    for other in screens.iter().skip(1) {
        let o = rect(other.frame());
        place(Rect::new(
            o.mid_x() - 200.0,
            o.mid_y() - 150.0,
            400.0,
            300.0,
        ));
        let bottom = Rect::new(
            f.min_x().max(o.min_x()) - 7.0,
            f.min_y() - 7.0,
            818.0,
            427.0,
        );
        assert_eq!(place(bottom), bottom, "moved on the change of screens");
    }
}

/// Swift `testGlowFollowsGlidingWindow` (Kevin 2026-09-25): the frame glides with its window instead of standing at
/// the old place during the move, and a tick with a stale rectangle does not pull it back.
fn glow_follows_gliding_window(mtm: MainThreadMarker) {
    let glow = GlowOverlay::new(mtm);
    let r1 = Rect::new(100.0, 100.0, 400.0, 300.0);
    let r2 = Rect::new(600.0, 100.0, 400.0, 300.0);
    let target = |rect| GlowTarget {
        id: "a".into(),
        rect,
        waiting: false,
        stopped: false,
        window_number: None,
    };
    glow.update(&[target(r1)], &BTreeSet::new(), None);
    let panel = window_at(mtm, grown(r1));
    glow.follow(&BTreeMap::from([("a".to_owned(), r2)]));
    assert_eq!(rect(panel.frame()), grown(r2), "follows the gliding window");
    glow.update(&[target(r1)], &BTreeSet::from(["a".to_owned()]), None);
    assert_eq!(
        rect(panel.frame()),
        grown(r2),
        "a stale tick does not pull it back"
    );
}

/// Swift `testListPanelTakesClicks` (GP 6): the list takes clicks without activating Lookout, lies only on the desktop
/// of the middle and is not moved by AppKit (like the labels).
fn list_panel_takes_clicks(mtm: MainThreadMarker) {
    let p = make_list_panel(mtm);
    let _out = OrderOut(&p);
    assert!(!p.ignoresMouseEvents(), "takes clicks");
    assert!(
        p.styleMask()
            .contains(NSWindowStyleMask::NonactivatingPanel),
        "does not activate"
    );
    let spaces = p.collectionBehavior();
    assert!(
        !spaces.contains(NSWindowCollectionBehavior::CanJoinAllSpaces),
        "only on the middle's desktop"
    );
    assert_eq!(p.animationBehavior(), NSWindowAnimationBehavior::None);
    let screens = NSScreen::screens(mtm);
    let Some(first) = screens.firstObject() else {
        return;
    };
    let vis = rect(first.visibleFrame());
    let top = Rect::new(vis.min_x() + 40.0, vis.max_y() - 100.0, 480.0, 120.0);
    p.setFrame_display(ns(top), false);
    p.orderFrontRegardless();
    assert_eq!(rect(p.frame()), top, "not moved");
}

/// A click on a row reaches `on_click` with exactly that row's session ids; the header and "+N" lines take none. The
/// click goes through `NSWindow.sendEvent` to the view's `mouseDown:`: the flipped view's coordinates, core `row_at`.
/// The callback updates the list from inside the click, as the controller does (it brings the session to the middle
/// and plans anew); that only works if the list holds no borrow while `on_click` runs. A synthetic event reaches the
/// view whatever `acceptsFirstMouse:` says (measured), so that one is asked directly: it lets the first click into the
/// inactive app through.
fn list_click_reaches_on_click(mtm: MainThreadMarker) {
    let lines = vec![
        ListLine::Header("SESSIONS".into()), // 0…28 from the top
        ListLine::Row {
            ids: vec!["a".into(), "b".into()],
            title: "✓ alpha".into(),
            detail: Some("done".into()),
            tone: LabelTone::Ready,
        }, // 28…74
        ListLine::Row {
            ids: vec!["c".into()],
            title: "● gamma".into(),
            detail: None,
            tone: LabelTone::Running,
        }, // 74…120
        ListLine::More(2),                   // 120…166
    ];
    let at = Rect::new(300.0, 300.0, 480.0, 166.0);
    let moved = Rect::new(320.0, 300.0, 480.0, 166.0);
    let clicks: Rc<RefCell<Vec<Vec<String>>>> = Rc::default();
    let lists = Rc::new_cyclic(|me: &Weak<ListOverlay>| {
        let (me, clicks, lines) = (me.clone(), Rc::clone(&clicks), lines.clone());
        ListOverlay::new(
            mtm,
            Box::new(move |ids| {
                clicks.borrow_mut().push(ids);
                if let Some(lists) = me.upgrade() {
                    // New lines in the same places: the view has to take them while it handles the click.
                    let mut lines = lines.clone();
                    lines[3] = ListLine::More(2 + clicks.borrow().len());
                    let target = ListTarget {
                        rect: moved,
                        lines,
                        window_number: 0,
                    };
                    lists.update(&[target], None);
                    lists.set_tints(MINT, ORANGE);
                }
            }),
            Box::new(|_, _| {}),
            Box::new(|_| {}),
            Box::new(|| None),
        )
    });
    let target = ListTarget {
        rect: at,
        lines,
        window_number: 0,
    };
    lists.update(&[target], None);
    let panel = window_at(mtm, at);
    let view = panel.contentView().expect("content view");
    assert!(view.acceptsFirstMouse(None) && view.isFlipped());
    let click = |from_top: f64| {
        // Window coordinates count from the bottom; the panel is all list.
        let location = NSPoint::new(40.0, rect(panel.frame()).height() - from_top);
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            NSEventType::LeftMouseDown,
            location,
            NSEventModifierFlags::empty(),
            0.0,
            panel.windowNumber(),
            None,
            0,
            1,
            1.0,
        )
        .expect("mouse event");
        panel.sendEvent(&event);
    };
    click(10.0);
    click(140.0);
    assert!(clicks.borrow().is_empty(), "header and +N take no click");
    click(94.0);
    assert_eq!(
        *clicks.borrow(),
        [vec!["c".to_owned()]],
        "the second row's ids"
    );
    assert_eq!(
        rect(panel.frame()),
        moved,
        "the update from inside the click went through"
    );
    click(40.0);
    assert_eq!(
        clicks.borrow().last(),
        Some(&vec!["a".to_owned(), "b".to_owned()]),
        "the first row's ids"
    );
}

/// Kevin 2026-11-10: the inbox scrolls below its tabs instead of "+N more". Every card stands in the scroll view's
/// document (it is as tall as they are); scrolled to the end, a click near the bottom reaches the last card and one on
/// the tabs still reaches the tabs; another tab starts at its top.
fn inbox_scrolls_below_its_tabs(mtm: MainThreadMarker) {
    let card = |k: usize| {
        ListLine::Card(InboxCard {
            ids: vec![format!("c{k}")],
            name: format!("card {k}"),
            project: None,
            since: None,
            what: None,
            tone: LabelTone::Ready,
            open: false,
            lines: 1,
            menu: vec![CardButton::Park],
            pinned: false,
            mark: None,
        })
    };
    let at = Rect::new(200.0, 200.0, 430.0, 300.0);
    let target_of = |tab, cards: usize| {
        let mut lines = vec![
            ListLine::InboxHead {
                tab,
                counts: InboxCounts {
                    waiting: cards,
                    ..InboxCounts::default()
                },
                team: None,
            },
            ListLine::Group("Fertig".into()),
        ];
        lines.extend((0..cards).map(card));
        ListTarget {
            rect: at,
            lines,
            window_number: 0,
        }
    };
    let target = |tab| target_of(tab, 10);
    let clicks: Rc<RefCell<Vec<Vec<String>>>> = Rc::default();
    let tabs: Rc<RefCell<Vec<InboxNav>>> = Rc::default();
    let lists = {
        let (clicks, tabs) = (Rc::clone(&clicks), Rc::clone(&tabs));
        ListOverlay::new(
            mtm,
            Box::new(move |ids| clicks.borrow_mut().push(ids)),
            Box::new(|_, _| {}),
            Box::new(move |tab| tabs.borrow_mut().push(tab)),
            Box::new(|| None),
        )
    };
    lists.update(&[target(InboxTab::Waiting)], None);
    let panel = window_at(mtm, at);
    let content = panel.contentView().expect("content view");
    let scroll = content
        .subviews()
        .firstObject()
        .expect("a view under the tabs")
        .downcast::<NSScrollView>()
        .expect("a scroll view");
    let tall = 28.0 + 10.0 * 76.0;
    let document = scroll.documentView().expect("the cards");
    assert_eq!(document.frame().size.height, tall, "every card, no +N");
    assert_eq!(
        rect(scroll.frame()).height(),
        300.0 - 44.0,
        "below the tabs"
    );

    let clip = scroll.contentView();
    clip.scrollToPoint(NSPoint::new(0.0, tall - (300.0 - 44.0)));
    scroll.reflectScrolledClipView(&clip);
    let click = |from_top: f64| {
        let location = NSPoint::new(40.0, rect(panel.frame()).height() - from_top);
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            NSEventType::LeftMouseDown,
            location,
            NSEventModifierFlags::empty(),
            0.0,
            panel.windowNumber(),
            None,
            0,
            1,
            1.0,
        )
        .expect("mouse event");
        panel.sendEvent(&event);
    };
    click(290.0);
    assert_eq!(
        clicks.borrow().last(),
        Some(&vec!["c9".to_owned()]),
        "the last card, scrolled into view"
    );
    click(20.0);
    assert_eq!(
        *tabs.borrow(),
        [InboxNav::Open(InboxTab::Overview)],
        "the head stays where it is: its way back to the overview"
    );
    // No frame and no ground of their own: the inbox's ground shows behind the cards (review quick 2026-11-10).
    assert_eq!(scroll.borderType(), NSBorderType::NoBorder);
    assert!(!scroll.drawsBackground() && !scroll.contentView().drawsBackground());
    // Scrolled to the end and the list shrinks on the same tab (review quick 2026-11-10): what is left stays in view.
    assert!(
        scroll.contentView().bounds().origin.y > 0.0,
        "still scrolled down"
    );
    lists.update(&[target_of(InboxTab::Waiting, 1)], None);
    assert_eq!(
        scroll.contentView().bounds().origin.y,
        0.0,
        "back up to what is left"
    );
    lists.update(&[target(InboxTab::Waiting)], None);

    lists.update(&[target(InboxTab::Working)], None);
    assert_eq!(
        scroll.contentView().bounds().origin.y,
        0.0,
        "another tab starts at its top"
    );

    // With "Show scroll bars: Always" the scroller takes room: the cards are as wide as what is left (review quick
    // 2026-11-10), so nothing is cut off on the right and nothing scrolls sideways.
    scroll.setScrollerStyle(NSScrollerStyle::Legacy);
    lists.update(&[target(InboxTab::Working)], None);
    let left = scroll.contentSize().width;
    assert!(left < 430.0, "the scroller takes room: {left}");
    assert_eq!(document.frame().size.width, left);
}

/// Kevin 2026-11-10 ("H"): a tile on the overview opens its area; in an area the amber strip stays under the head (it does
/// not scroll away) and opens Waiting.
fn inbox_tiles_and_strip_lead_on(mtm: MainThreadMarker) {
    let at = Rect::new(240.0, 220.0, 430.0, 400.0);
    let tile = |tab, title: &str| InboxTile {
        tab,
        team: None,
        title: title.into(),
        count: 1,
        lines: vec![],
        hot: false,
    };
    let overview = ListTarget {
        rect: at,
        lines: vec![
            ListLine::InboxHead {
                tab: InboxTab::Overview,
                counts: InboxCounts::default(),
                team: None,
            },
            ListLine::Tiles(vec![tile(InboxTab::Waiting, "Wartet")]),
            ListLine::Tiles(vec![
                tile(InboxTab::Working, "Läuft"),
                tile(InboxTab::Planned, "Geplant"),
            ]),
        ],
        window_number: 0,
    };
    let navs: Rc<RefCell<Vec<InboxNav>>> = Rc::default();
    let lists = {
        let navs = Rc::clone(&navs);
        ListOverlay::new(
            mtm,
            Box::new(|_| {}),
            Box::new(|_, _| {}),
            Box::new(move |nav| navs.borrow_mut().push(nav)),
            Box::new(|| None),
        )
    };
    lists.update(&[overview], None);
    let panel = window_at(mtm, at);
    let click = |x: f64, from_top: f64| {
        let location = NSPoint::new(x, rect(panel.frame()).height() - from_top);
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            NSEventType::LeftMouseDown,
            location,
            NSEventModifierFlags::empty(),
            0.0,
            panel.windowNumber(),
            None,
            0,
            1,
            1.0,
        )
        .expect("mouse event");
        panel.sendEvent(&event);
    };
    click(320.0, 44.0 + 94.0 + 20.0);
    assert_eq!(
        *navs.borrow(),
        [InboxNav::Open(InboxTab::Planned)],
        "the right tile of the second row"
    );

    let area = ListTarget {
        rect: at,
        lines: vec![
            ListLine::InboxHead {
                tab: InboxTab::Working,
                counts: InboxCounts::default(),
                team: None,
            },
            ListLine::WaitBar {
                n: 1,
                text: "alpha".into(),
            },
            ListLine::Group("Arbeitet".into()),
        ],
        window_number: 0,
    };
    lists.update(&[area], None);
    let content = panel.contentView().expect("content view");
    let scroll = content
        .subviews()
        .firstObject()
        .expect("a view under the head")
        .downcast::<NSScrollView>()
        .expect("a scroll view");
    assert_eq!(
        rect(scroll.frame()).height(),
        400.0 - 44.0 - 36.0,
        "below head and strip"
    );
    click(300.0, 44.0 + 10.0);
    assert_eq!(
        navs.borrow().last(),
        Some(&InboxNav::Open(InboxTab::Waiting)),
        "the strip"
    );
    click(20.0, 20.0);
    assert_eq!(
        navs.borrow().last(),
        Some(&InboxNav::Open(InboxTab::Overview)),
        "the way back"
    );
}

/// What `view` paints at (x, y), its points from the bottom left, drawn into a bitmap at the window's scale.
fn painted(view: &NSView, x: f64, y: f64) -> Retained<NSColor> {
    let bounds = view.bounds();
    let rep = view
        .bitmapImageRepForCachingDisplayInRect(bounds)
        .expect("bitmap");
    view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
    let scale = rep.pixelsWide() as f64 / bounds.size.width;
    // Bitmap rows count from the top.
    let (column, row) = (x * scale, (bounds.size.height - y) * scale);
    rep.colorAtX_y(column as isize, row as isize)
        .expect("pixel")
}

/// Each panel kind draws: `display` runs the on-screen path, and a bitmap of each content view shows paint where it
/// belongs (a `drawRect:` that is not wired up would leave it empty): the glow's line on the left edge around a clear
/// middle, the dark ground of label and list, the preview's tinted box (0.16) with its dark text block in the middle.
fn every_panel_kind_draws(mtm: MainThreadMarker) {
    let glow = GlowOverlay::new(mtm);
    glow.set_tints(MINT, ORANGE);
    let g = Rect::new(120.0, 520.0, 300.0, 200.0);
    let glow_target = GlowTarget {
        id: "g".into(),
        rect: g,
        waiting: true,
        stopped: false,
        window_number: None,
    };
    glow.update(&[glow_target], &BTreeSet::new(), None);

    let labels = LabelOverlay::new(mtm);
    labels.set_tints(MINT, ORANGE);
    let l = Rect::new(460.0, 520.0, 360.0, 80.0);
    let content = LabelContent {
        tone: LabelTone::Waiting,
        duration: Some("for 6 min".into()),
        topic: "Lookout · overlays".into(),
        detail: Some("“Shall I commit?”".into()),
        hint: None,
    };
    let label_target = LabelTarget {
        id: "l".into(),
        rect: l,
        window_number: None,
        content,
    };
    labels.update(&[label_target], None);

    let lists = ListOverlay::new(
        mtm,
        Box::new(|_| {}),
        Box::new(|_, _| {}),
        Box::new(|_| {}),
        Box::new(|| None),
    );
    lists.set_tints(MINT, ORANGE);
    let li = Rect::new(860.0, 520.0, 300.0, 120.0);
    let row = ListLine::Row {
        ids: vec!["a".into()],
        title: "? alpha".into(),
        detail: Some("“Shall I commit?”".into()),
        tone: LabelTone::Waiting,
    };
    let list_target = ListTarget {
        rect: li,
        lines: vec![ListLine::Header("SESSIONS".into()), row, ListLine::More(3)],
        window_number: 0,
    };
    let card = |open, tone| {
        ListLine::Card(InboxCard {
            ids: vec!["a".into()],
            name: "alpha".into(),
            project: Some("beta".into()),
            since: Some("seit 6 Min.".into()),
            what: Some("Shall I commit?".into()),
            tone,
            open,
            lines: 2,
            menu: vec![CardButton::Park],
            // The open card is the pinned one: its pin is probed below.
            pinned: open,
            mark: None,
        })
    };
    let ib = Rect::new(60.0, 200.0, 430.0, 500.0);
    let inbox = ListTarget {
        rect: ib,
        lines: vec![
            ListLine::InboxHead {
                tab: InboxTab::Working,
                counts: InboxCounts {
                    waiting: 1,
                    working: 2,
                    parked: 3,
                    planned: Some(4),
                },
                team: Some(("Acme".into(), 3)),
            },
            ListLine::WaitBar {
                n: 1,
                text: "alpha: Shall I commit?".into(),
            },
            ListLine::Tiles(vec![
                InboxTile {
                    tab: InboxTab::Waiting,
                    team: None,
                    title: "Wartet auf dich".into(),
                    count: 1,
                    lines: vec!["? alpha".into()],
                    hot: true,
                },
                InboxTile {
                    tab: InboxTab::Planned,
                    team: None,
                    title: "Geplant".into(),
                    count: 4,
                    lines: vec!["▶ heute 10:00 Report".into(), "1 heute · 0 morgen".into()],
                    hot: false,
                },
            ]),
            ListLine::Week(WeekStrip {
                title: "KW 46".into(),
                days: (0..7)
                    .map(|k| WeekDay {
                        date: lookout_core::planned::NaiveDate::from_ymd_opt(2026, 11, 9 + k)
                            .unwrap(),
                        name: "Mo".into(),
                        number: 9 + k,
                        today: k == 1,
                        selected: k == 1,
                        dots: vec![PlanMark::Start, PlanMark::Task, PlanMark::Deadline],
                    })
                    .collect(),
            }),
            ListLine::Card(InboxCard {
                ids: vec![],
                name: "Report".into(),
                project: None,
                since: Some("10:00".into()),
                what: Some("Startet von selbst".into()),
                tone: LabelTone::Watching,
                open: false,
                lines: 1,
                menu: vec![],
                pinned: false,
                mark: Some(PlanMark::Task),
            }),
            ListLine::Group("Fragen".into()),
            card(true, LabelTone::Waiting),
            ListLine::Hint("Rechts antworten".into()),
            card(false, LabelTone::Ready),
            ListLine::Buttons {
                ids: vec!["a".into()],
                buttons: vec![CardButton::ReadNext, CardButton::Park],
            },
            ListLine::Empty("Niemand".into()),
            // A team's tile at the end: the probes above keep their places.
            ListLine::Tiles(vec![InboxTile {
                tab: InboxTab::Team,
                team: Some("Acme".into()),
                title: "Acme".into(),
                count: 3,
                lines: vec!["? Key account fragt dich".into(), "1 Arbeiter läuft".into()],
                hot: true,
            }]),
        ],
        window_number: 0,
    };
    lists.update(&[list_target, inbox], None);

    let preview = PreviewOverlay::new(mtm);
    let pr = Rect::new(120.0, 120.0, 480.0, 300.0);
    preview.show(&[PreviewBox {
        rect: pr,
        title: "alpha · beta".into(),
        detail: "claude · done".into(),
    }]);

    // Per panel frame: (x, y, lowest alpha, highest alpha).
    type Probe = (f64, f64, f64, f64);
    let probes: [(Rect, &[Probe]); 5] = [
        (
            grown(g),
            &[(7.0, 107.0, 0.8, 1.0), (157.0, 107.0, 0.0, 0.01)],
        ),
        (l, &[(180.0, 40.0, 0.9, 1.0)]),
        (li, &[(150.0, 60.0, 0.9, 1.0)]),
        (ib, &[(400.0, 250.0, 0.99, 1.0)]),
        (pr, &[(240.0, 150.0, 0.5, 1.0), (30.0, 40.0, 0.1, 0.25)]),
    ];
    for (frame, points) in probes {
        let panel = window_at(mtm, frame);
        panel.display();
        let view = panel.contentView().expect("content view");
        for &(x, y, lo, hi) in points {
            let alpha = painted(&view, x, y).alphaComponent();
            assert!(
                (lo..=hi).contains(&alpha),
                "{frame:?} at ({x}, {y}): alpha {alpha}"
            );
        }
    }
    // The glow asks: its line is orange, not mint (done and asking must not look alike).
    let line = painted(
        &window_at(mtm, grown(g))
            .contentView()
            .expect("content view"),
        7.0,
        107.0,
    );
    let (red, green, blue) = (
        line.redComponent(),
        line.greenComponent(),
        line.blueComponent(),
    );
    assert!(
        red > green && green > blue,
        "waiting tint: {red} {green} {blue}"
    );
    // The pin before the name of the pinned card (tabs 44, group 28, then the card) takes the waiting tint; the same
    // place on the card that is not pinned shows the inbox's ground.
    let inbox_view = window_at(mtm, ib).contentView().expect("content view");
    let amber = |y: f64| {
        let c = painted(&inbox_view, 57.6, y);
        c.redComponent() > c.greenComponent() && c.greenComponent() > c.blueComponent()
    };
    assert!(amber(500.0 - 89.5), "pin on the pinned card");
    assert!(!amber(500.0 - 220.5), "no pin on the other card");
}

/// The fades run: the glow's ticker breathes a new frame in and a dropped one out until it leaves the screen, a label's
/// fade-in reaches full and its fade-out orders the panel out at the end.
fn fades_end_off_screen(mtm: MainThreadMarker) {
    let glow = GlowOverlay::new(mtm);
    let g = Rect::new(200.0, 200.0, 300.0, 200.0);
    let target = GlowTarget {
        id: "f".into(),
        rect: g,
        waiting: false,
        stopped: false,
        window_number: None,
    };
    glow.update(&[target], &BTreeSet::new(), None);
    let labels = LabelOverlay::new(mtm);
    let l = Rect::new(560.0, 200.0, 360.0, 62.0);
    let content = LabelContent {
        tone: LabelTone::Running,
        duration: None,
        topic: "Lookout".into(),
        detail: None,
        hint: None,
    };
    labels.update(
        &[LabelTarget {
            id: "f".into(),
            rect: l,
            window_number: None,
            content,
        }],
        None,
    );
    let (glow_panel, label_panel) = (window_at(mtm, grown(g)), window_at(mtm, l));
    assert!(
        glow_panel.alphaValue() == 0.0 && label_panel.alphaValue() == 0.0,
        "both start invisible"
    );
    run_loop(0.7);
    // Breathing keeps the faded-in glow between 0.62 − 0.13 and 0.62 + 0.13.
    assert!(
        (0.45..=0.8).contains(&glow_panel.alphaValue()),
        "glow faded in: {}",
        glow_panel.alphaValue()
    );
    assert_eq!(label_panel.alphaValue(), 1.0, "label faded in");
    glow.hide_all();
    labels.hide_all();
    run_loop(0.7);
    assert!(!glow_panel.isVisible(), "glow ordered out after its fade");
    assert!(!label_panel.isVisible(), "label ordered out after its fade");
}

/// Swift `place`: a panel is ordered again only when another app's window lies between it and its window; every order
/// call relative to another app's window costs WindowServer a rights check with an error line (284/min until 27.09.).
/// Seen through a hidden panel, which an order call shows. Its window here is an own panel, so no call ever touches
/// another app's window.
fn place_orders_only_when_needed(mtm: MainThreadMarker) {
    let window = make_overlay_panel(mtm, Level::Normal);
    let _out_window = OrderOut(&window);
    window.orderFrontRegardless();
    let p = make_overlay_panel(mtm, Level::Normal);
    let _out = OrderOut(&p);
    let frame = Rect::new(150.0, 150.0, 200.0, 100.0);
    let own = std::process::id() as Pid;
    let entry = |number: isize, pid| OnScreenWindow {
        number: number as i64,
        pid,
        rect: Rect::ZERO,
    };
    let (panel_no, window_no) = (p.windowNumber(), window.windowNumber());
    // Right above its window with only an own panel between: the new frame, no order call.
    let stack = [
        entry(panel_no, own),
        entry(panel_no + 1000, own),
        entry(window_no, 77),
    ];
    place(&p, frame, Some(window_no as i64), Some(&stack));
    assert_eq!(rect(p.frame()), frame);
    assert!(!p.isVisible(), "left alone");
    // Another app's window between, or no stack known: ordered above its window again.
    let stack = [
        entry(panel_no, own),
        entry(panel_no + 1000, 55),
        entry(window_no, 77),
    ];
    place(&p, frame, Some(window_no as i64), Some(&stack));
    assert!(p.isVisible(), "ordered above its window");
    p.orderOut(None);
    place(&p, frame, Some(window_no as i64), None);
    assert!(p.isVisible(), "unknown stack: ordered");
    // No window number: in front.
    p.orderOut(None);
    place(&p, frame, None, None);
    assert!(p.isVisible(), "no number: in front");
}
