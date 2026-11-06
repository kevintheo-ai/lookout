//! Fixtures shared by the unit tests of every module (Swift: the helpers in LayoutTests.swift).

use std::collections::BTreeSet;

use crate::geom::Rect;
use crate::layout::PlanInput;
use crate::model::{
    Arrangement, Digest, Engine, LayoutMode, ListEntry, ScreenInfo, Session, StackSide, Target,
    TermWindow, Time,
};
use crate::motion::WARP_BID;

/// Screen of the layout-math tests.
pub const VIS: Rect = Rect::new(0.0, 38.0, 1512.0, 982.0);
/// Kevin's two screens (measured 2026-09-18): LG 45" 21:9 and the MacBook below it.
pub const OLED: Rect = Rect::new(0.0, 0.0, 3008.0, 1239.0);
pub const MACBOOK: Rect = Rect::new(687.0, -1037.0, 1728.0, 1005.0);
pub const BOTH: [Rect; 2] = [OLED, MACBOOK];

pub fn lg_and_macbook() -> Vec<ScreenInfo> {
    vec![ScreenInfo::new(OLED, false), ScreenInfo::new(MACBOOK, true)]
}

/// Control-room target with Swift's labels.
pub fn room(work: usize, control: usize, ready_side: StackSide, flipped: bool) -> Target {
    Target::ControlRoom {
        work,
        control,
        ready_side,
        flipped,
    }
}

/// A plan on Kevin's two screens.
pub fn input(
    mode: LayoutMode,
    target: Target,
    windows: &[TermWindow],
    side: StackSide,
) -> PlanInput<'_> {
    PlanInput::new(mode, target, windows, &BOTH, side)
}

/// Swift `fakeWindow(ready:firstSeen:)`.
pub fn fake_window(ready: bool, first_seen: Time) -> TermWindow {
    fake_window_with(ready, first_seen, None, false, true)
}

/// Swift `fakeWindow(ready:firstSeen:readySince:waiting:hasSession:)`: one Claude session, frame zero.
pub fn fake_window_with(
    ready: bool,
    first_seen: Time,
    ready_since: Option<Time>,
    waiting: bool,
    has_session: bool,
) -> TermWindow {
    let s = Session {
        waiting,
        ..Session::new(
            format!("claude:x{first_seen:?}-{waiting}"),
            Engine::Claude,
            Some(1),
            "x",
            "",
            ready,
            true,
        )
    };
    TermWindow {
        id: 0,
        app: "Warp".into(),
        bundle_id: WARP_BID.into(),
        owner_pid: 0,
        frame: Rect::ZERO,
        title: String::new(),
        sessions: if has_session { vec![s] } else { vec![] },
        first_seen,
        ready_since: if ready {
            Some(ready_since.unwrap_or(first_seen))
        } else {
            None
        },
        movable: true,
        minimized: false,
        fullscreen: false,
    }
}

/// Windows spread over BOTH screens: 0 works (LG), 1 done (MacBook), 2 asks (LG), 3 works (MacBook), 4 is a plain
/// shell without AI session (LG).
pub fn spread_windows() -> Vec<TermWindow> {
    let t0 = 1000.0;
    let mut w = vec![
        fake_window(false, t0),
        fake_window(true, t0 + 1.0),
        fake_window_with(true, t0 + 2.0, None, true, true),
        fake_window(false, t0 + 3.0),
        fake_window_with(false, t0 + 4.0, None, false, false),
    ];
    w[0].frame = Rect::new(100.0, 100.0, 800.0, 600.0);
    w[1].frame = Rect::new(900.0, -900.0, 800.0, 600.0);
    w[2].frame = Rect::new(1500.0, 300.0, 800.0, 600.0);
    w[3].frame = Rect::new(700.0, -700.0, 600.0, 500.0);
    w[4].frame = Rect::new(2000.0, 200.0, 600.0, 400.0);
    w
}

/// Kevin 2026-09-26: with 11 sessions an edge on the MacBook holds at most 5 windows. Active middle (0, works) + done
/// (1) + question (2) + 8 working (3…10), all still on the LG.
pub fn eleven_sessions() -> Vec<TermWindow> {
    let t0 = 1000.0;
    let mut w = vec![
        fake_window(false, t0),
        fake_window_with(true, t0 + 1.0, Some(5000.0), false, true),
        fake_window_with(true, t0 + 2.0, Some(6000.0), true, true),
    ];
    w.extend((3..11).map(|i| fake_window(false, t0 + i as f64)));
    for (i, x) in w.iter_mut().enumerate() {
        x.frame = Rect::new(100.0 + i as f64 * 10.0, 100.0, 800.0, 600.0);
    }
    w
}

/// Swift `tabSession`: a Claude session in a tab, with its topic.
pub fn tab_session(id: &str, ready: bool, waiting: bool, title: Option<&str>) -> Session {
    Session {
        waiting,
        digest: Some(Digest {
            title: title.map(str::to_owned),
            ..Digest::default()
        }),
        ..Session::new(
            format!("claude:{id}"),
            Engine::Claude,
            Some(1),
            id,
            "/Users/test/Projects/Beta",
            ready,
            true,
        )
    }
}

/// Swift `tabWindow`.
pub fn tab_window(sessions: Vec<Session>, title: &str) -> TermWindow {
    TermWindow {
        id: 0,
        app: "Warp".into(),
        bundle_id: WARP_BID.into(),
        owner_pid: 0,
        frame: Rect::ZERO,
        title: title.into(),
        sessions,
        first_seen: 1000.0,
        ready_since: None,
        movable: true,
        minimized: false,
        fullscreen: false,
    }
}

/// Every session has exactly one role (middle, window or parked); nothing pokes over the screen; visible windows
/// overlap neither each other nor a list. List rows: exactly the parked windows outside the parking lot (those have
/// no row).
pub fn assert_roles(a: &Arrangement, count: usize, screen: Rect) {
    assert!(
        a.frames.keys().copied().eq(0..count),
        "every session has a place, got {:?}",
        a.frames.keys()
    );
    for r in a.frames.values() {
        assert!(screen.contains(*r), "{r:?} pokes over the screen");
    }
    let shown: Vec<Rect> = a
        .frames
        .iter()
        .filter(|(k, _)| !a.parked.contains(k))
        .map(|(_, r)| *r)
        .chain(a.lists.iter().map(|l| l.rect))
        .collect();
    for (i, r) in shown.iter().enumerate() {
        for s in &shown[i + 1..] {
            assert!(!r.intersects(*s), "{r:?} overlaps {s:?}");
        }
    }
    let rows: Vec<usize> = a
        .lists
        .iter()
        .flat_map(|l| &l.entries)
        .filter_map(|e| {
            if let ListEntry::Row(i) = e {
                Some(*i)
            } else {
                None
            }
        })
        .collect();
    let listed: BTreeSet<usize> = a.parked.difference(&a.lot).copied().collect();
    assert!(
        rows.iter().copied().collect::<BTreeSet<_>>() == listed && rows.len() == listed.len(),
        "every parked window outside the lot stands exactly once as a row"
    );
}
