//! Layout tests, ported assert for assert from LayoutTests.swift, plus the parking lot (Kevin 2026-11-06).

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::i18n::set_german;
use crate::model::LayoutMode::{Focus2, Grid, Reading};
use crate::model::ListEntry::{Header, More, Row};
use crate::model::StackSide::{Left, Right};
use crate::model::Target::Single;
use crate::model::TargetChoice::{Both, Both2, Both3, Builtin, External, Screen};
use crate::model::{Digest, Engine, Session};
use crate::testutil::*;

fn keys(m: &BTreeMap<usize, Rect>) -> Vec<usize> {
    m.keys().copied().collect()
}

fn entries(a: &Arrangement) -> Vec<Vec<ListEntry>> {
    a.lists.iter().map(|l| l.entries.clone()).collect()
}

fn header(s: &str) -> ListEntry {
    Header(s.into())
}

fn sides(vis: Rect) -> (Rect, Rect, Rect) {
    let (main, left, right) = reading_layout(vis, Right);
    (main, left.expect("left strip"), right.expect("right strip"))
}

// ---------- Task 6/7: Layout-Mathematik (all Cocoa, slot 0 top-left) ----------

#[test]
fn test_grid22() {
    let f = grid_layout(VIS, 2, 2, Right, &["a", "b", "c", "d"]);
    assert!(f.frames.len() == 4 && f.overflow.is_empty());
    assert_eq!(f.frames["a"], Rect::new(0.0, 529.0, 756.0, 491.0));
    assert_eq!(f.frames["a"].max_y(), 1020.0, "slot 0 is on top");
    assert_eq!(
        f.frames["d"].min_y(),
        38.0,
        "last slot closes flush at the bottom"
    );
}

#[test]
fn test_grid22_overflow_reserves_rail() {
    let f = grid_layout(VIS, 2, 2, Right, &["a", "b", "c", "d", "e"]);
    assert_eq!(f.overflow, ["e"]);
    assert_eq!(
        f.frames["a"].width(),
        (VIS.width() - mini_rail_width(VIS)) / 2.0,
        "rail first, then divide by columns"
    );
    assert_eq!(
        mini_stack(VIS, Right, &f.overflow)["e"].width(),
        mini_rail_width(VIS)
    );
}

#[test]
fn test_focus_fills() {
    let (m, s) = focus_layout(VIS, Right);
    assert_eq!(m.max_x(), s.min_x(), "no gap between main window and strip");
    assert_eq!(m.min_x(), VIS.min_x());
    assert_eq!(m.height(), VIS.height());
}

#[test]
fn test_rail_overflow_cap() {
    let many: Vec<String> = (0..20).map(|i| format!("w{i}")).collect();
    let m = mini_stack(VIS, Right, &many);
    assert!(
        m.len() < 20,
        "extra ones stay untouched instead of stacked invisibly"
    );
    for r in m.values() {
        assert!(r.height() >= MIN_CARD_H, "no card below the minimum height");
    }
}

/// Kevin 2026-09-18: whoever gets done LATER queues behind, not in front of those already waiting.
#[test]
fn test_attention_order_ready_queue() {
    let t0 = 1000.0;
    let frueh = fake_window_with(true, t0, Some(5000.0), false, true);
    let spaet = fake_window_with(true, t0, Some(9000.0), false, true);
    let arbeitet = fake_window(false, t0);
    assert_eq!(
        attention_order(&[spaet, frueh, arbeitet]),
        [1, 0, 2],
        "first done in front, later one next, working last"
    );
}

#[test]
fn test_focus_left_side() {
    let (m, s) = focus_layout(VIS, Left);
    assert!(
        s.min_x() == VIS.min_x() && s.max_x() == m.min_x(),
        "strip left, main window right beside it"
    );
}

#[test]
fn test_attention_order_ready_first() {
    let (old, new) = (1000.0, 2000.0);
    assert_eq!(
        attention_order(&[fake_window(false, old), fake_window(true, new)]),
        [1, 0],
        "done window moves up"
    );
    let both = [fake_window(true, new), fake_window(true, old)];
    assert_eq!(
        attention_order(&both),
        [1, 0],
        "on a tie first sighting decides"
    );
}

/// Kevin 2026-09-18: "wenn eine frage gestellt wird an mich ... das hat prio 1".
#[test]
fn test_waiting_beats_everything() {
    let t0 = 1000.0;
    let laengst_fertig = fake_window_with(true, t0, Some(2000.0), false, true);
    let fragt = fake_window_with(true, t0, Some(9000.0), true, true);
    let arbeitet = fake_window(false, t0);
    assert_eq!(
        attention_order(&[&laengst_fertig, &fragt, &arbeitet]),
        [1, 0, 2],
        "the question stands in front although the other session is done much longer"
    );
    assert!(fragt.is_waiting() && !laengst_fertig.is_waiting());
}

#[test]
fn test_grid_shrinks_to_window_count() {
    // One window gets the whole screen, not a quarter.
    let one = grid_layout(VIS, 2, 2, Right, &["a"]);
    assert_eq!(one.frames["a"], VIS);
    // Two windows share side by side, full height.
    let two = grid_layout(VIS, 2, 2, Right, &["a", "b"]);
    assert!(two.frames["a"].height() == VIS.height() && two.frames["b"].height() == VIS.height());
    assert_eq!(
        two.frames["a"].max_x(),
        two.frames["b"].min_x(),
        "no gap between"
    );
    // Three in 3x2: one row, full height.
    let three = grid_layout(VIS, 3, 2, Right, &["a", "b", "c"]);
    assert_eq!(three.frames["c"].height(), VIS.height());
    assert!(three.overflow.is_empty());
}

// ---------- Bildschirm-Zuordnung (review finding GLM) ----------

#[test]
fn test_screen_index_by_overlap() {
    let haupt = Rect::new(0.0, 0.0, 3008.0, 1153.0);
    let zweit = Rect::new(687.0, -1117.0, 1728.0, 1085.0);
    let screens = [haupt, zweit];
    assert_eq!(
        screen_index(Rect::new(100.0, 100.0, 800.0, 600.0), &screens),
        0
    );
    assert_eq!(
        screen_index(Rect::new(800.0, -900.0, 800.0, 600.0), &screens),
        1
    );
    // Centre in the Dock strip (not part of visibleFrame): must NOT fall back to screen 0, which would pull the
    // window over from the second monitor.
    let im_dock = Rect::new(800.0, -1150.0, 800.0, 100.0);
    assert_eq!(
        screen_index(im_dock, &screens),
        1,
        "nearest screen wins, not index 0"
    );
    // No overlap at all -> nearest centre.
    assert_eq!(
        screen_index(Rect::new(900.0, -3000.0, 100.0, 100.0), &screens),
        1
    );
}

// ---------- Kevins zwei Bildschirme (measured 2026-09-18) ----------

#[test]
fn test_auto_grid_per_screen() {
    // Three sessions on the ultrawide: three columns over the FULL height, no half tiles.
    let g3 = auto_grid(OLED, 3);
    assert_eq!(g3, (3, 1));
    let f3 = grid_layout(OLED, g3.0, g3.1, Right, &["a", "b", "c"]);
    assert_eq!(f3.frames["a"].height(), OLED.height(), "full height");
    assert_eq!(f3.frames["a"].width(), 1002.0);
    // Six sessions: fill the columns first, then a second row.
    assert_eq!(auto_grid(OLED, 6), (3, 2));
    // The same set on the MacBook: only two columns, otherwise too narrow.
    assert_eq!(auto_grid(MACBOOK, 3), (2, 2));
    assert_eq!(auto_grid(MACBOOK, 2), (2, 1));
    assert_eq!(auto_grid(MACBOOK, 1), (1, 1));
}

#[test]
fn test_reading_width_on_ultrawide() {
    // One window across the full width would be unreadable: the main window stands centred.
    let (main, left, right) = reading_layout(OLED, Right);
    assert_eq!(main.width(), READING_WIDTH);
    assert!(
        left.is_some() && right.is_some(),
        "3008 pt hold strips on BOTH sides"
    );
    let (left, right) = (left.unwrap(), right.unwrap());
    assert!(
        left.max_x() == main.min_x() && main.max_x() == right.min_x(),
        "gapless"
    );
    assert!((left.width() - right.width()).abs() <= 1.0, "centred");
    assert!(left.min_x() == OLED.min_x() && right.max_x() == OLED.max_x());
    // On the laptop that does not fit: the view falls back to one strip.
    let (m2, l2, r2) = reading_layout(MACBOOK, Right);
    assert!(l2.is_none() && r2.is_some(), "only one strip, right");
    assert_eq!(m2.max_x(), r2.unwrap().min_x());
}

// ---------- Ziel-Knoepfe + Leitstand (Kevin 2026-09-18, replaces the waiting room) ----------

#[test]
fn test_resolve_target() {
    let s = lg_and_macbook();
    assert_eq!(resolve_target(&External, &s, None), Some(Single(0)));
    assert_eq!(resolve_target(&Builtin, &s, None), Some(Single(1)));
    assert_eq!(
        resolve_target(&Both, &s, None),
        Some(room(0, 1, Left, false))
    );
    // The order of the screens must not matter.
    let umgedreht = [s[1].clone(), s[0].clone()];
    assert_eq!(resolve_target(&External, &umgedreht, None), Some(Single(1)));
    assert_eq!(
        resolve_target(&Both, &umgedreht, None),
        Some(room(1, 0, Left, false))
    );
    // MacBook alone: every choice lands on the MacBook, the choice itself is kept.
    let nur_macbook = [s[1].clone()];
    assert_eq!(
        resolve_target(&External, &nur_macbook, None),
        Some(Single(0)),
        "missing LG -> present screen"
    );
    assert_eq!(
        resolve_target(&Both, &nur_macbook, None),
        Some(Single(0)),
        "control room needs two screens"
    );
    // Lid closed, only the LG.
    assert_eq!(
        resolve_target(&Builtin, &[s[0].clone()], None),
        Some(Single(0))
    );
    assert_eq!(resolve_target(&External, &[], None), None);
}

#[test]
fn test_target_moves_all_to_one_screen() {
    let w = spread_windows();
    for (target, ziel, anderer) in [(Single(1), MACBOOK, OLED), (Single(0), OLED, MACBOOK)] {
        for mode in LayoutMode::ALL {
            let plan = plan_layout(&input(mode, target, &w, Right));
            assert_eq!(
                keys(&plan),
                [0, 1, 2, 3],
                "{mode:?}: all AI windows, not the plain shell"
            );
            for r in plan.values() {
                assert!(
                    ziel.contains(*r),
                    "{mode:?}: {r:?} is not on the target screen"
                );
                assert!(
                    !anderer.intersects(*r),
                    "{mode:?}: the other screen stays free"
                );
            }
        }
    }
}

#[test]
fn test_focus_main_is_active_session() {
    let w = spread_windows();
    let plan = plan_layout(&PlanInput {
        focused: Some(2),
        ..input(Reading, Single(0), &w, Right)
    });
    assert_eq!(
        plan.get(&2),
        Some(&reading_layout(OLED, Right).0),
        "the active session stands in the middle"
    );
    let ohne = plan_layout(&input(Reading, Single(0), &w, Right));
    assert_eq!(
        ohne.get(&0),
        Some(&reading_layout(OLED, Right).0),
        "without active session: the one seen first"
    );
}

// ---------- Fokus 2 (Kevin 2026-09-18, design v3) ----------

/// Middle = active session, LEFT waits for Kevin (question first, then done by time of being done), RIGHT runs.
#[test]
fn test_focus2_sides_by_status() {
    let w = spread_windows(); // 0 runs, 1 done, 2 question, 3 runs
    let plan = plan_layout(&PlanInput {
        focused: Some(0),
        ..input(Focus2, Single(0), &w, Right)
    });
    let (main, left, right) = sides(OLED);
    assert_eq!(
        plan.get(&0),
        Some(&main),
        "the active session stands in the middle"
    );
    for i in [1, 2] {
        assert!(
            left.contains(plan[&i]),
            "window {i} waits for Kevin -> left"
        );
    }
    assert!(right.contains(plan[&3]), "window 3 runs -> right");
    assert!(
        plan[&2].max_y() > plan[&1].max_y(),
        "the question stands above the one that is only done"
    );
    assert!(!plan.contains_key(&4), "plain shell stays where it is");
}

/// A session that gets done glides left, and back once it works again. The active session stays in the middle.
#[test]
fn test_focus2_follows_status() {
    let mut w = spread_windows();
    let (main, left, right) = sides(OLED);
    let before = plan_layout(&PlanInput {
        focused: Some(1),
        ..input(Focus2, Single(0), &w, Right)
    });
    assert_eq!(
        before.get(&1),
        Some(&main),
        "a done active session stands in the middle too"
    );
    assert!(right.contains(before[&3]), "3 runs -> right");

    let frame = w[3].frame;
    w[3] = fake_window_with(true, w[3].first_seen, Some(9000.0), false, true);
    w[3].frame = frame;
    let after = plan_layout(&PlanInput {
        focused: Some(1),
        ..input(Focus2, Single(0), &w, Right)
    });
    assert!(left.contains(after[&3]), "3 is done -> now left");
    assert!(
        after[&2].max_y() > after[&3].max_y(),
        "the open question stays on top"
    );
    assert!(right.contains(after[&0]), "0 keeps running on the right");
}

/// MacBook (too narrow for two sides): main window left, right column waiting ones on top. Since plan 1d the running
/// ones are never windows there: they lie parked behind the middle and stand as rows in the WORKING list.
#[test]
fn test_focus2_narrow() {
    let w = spread_windows();
    let a = plan_arrangement(&PlanInput {
        focused: Some(0),
        ..input(Focus2, Single(1), &w, Right)
    });
    let main = reading_layout(MACBOOK, Right).0;
    assert_eq!(a.frames.get(&0), Some(&main), "main window big on the left");
    for i in [1, 2] {
        assert!(
            a.frames[&i].min_x() >= main.max_x(),
            "window {i} waits -> right column"
        );
    }
    assert!(
        a.frames[&2].max_y() > a.frames[&1].max_y(),
        "the question stands on top"
    );
    assert!(
        a.frames.get(&3) == Some(&main) && a.parked == BTreeSet::from([3]),
        "the running window lies parked"
    );
    assert_eq!(entries(&a), [vec![header("ARBEITET (1)"), Row(3)]]);
    assert!(
        a.lists[0].rect.max_y() <= a.frames[&1].min_y().min(a.frames[&2].min_y()),
        "list below the waiting ones"
    );
}

// ---------- Raster mit fester Auswahl (Kevin 2026-09-18: "Fest") ----------

#[test]
fn test_grid_choices_fit_screen() {
    assert_eq!(
        GRID_CHOICES.map(GridSpec::key),
        ["2x2", "3x2", "4x2", "3x3", "4x3", "5x2"]
    );
    assert_eq!(grid_choices(OLED).len(), 6, "all fit on the LG");
    assert_eq!(
        grid_choices(MACBOOK)
            .iter()
            .map(|g| g.key())
            .collect::<Vec<_>>(),
        ["2x2", "3x2", "3x3"],
        "MacBook ≤ 3 columns"
    );
    for screen in [OLED, MACBOOK] {
        for g in grid_choices(screen) {
            assert!(
                (screen.width() / g.cols as f64).floor() >= MIN_CARD_W,
                "{}: column too narrow",
                g.key()
            );
            assert!(
                (screen.height() / g.rows as f64).floor() >= MIN_CARD_H,
                "{}: row too low",
                g.key()
            );
        }
    }
    // Stored choice does not fit the target screen -> trimmed to the largest that fits.
    assert_eq!(fitted_grid(GridSpec::new(5, 2), MACBOOK).key(), "3x2");
    assert_eq!(fitted_grid(GridSpec::new(4, 3), MACBOOK).key(), "3x3");
    assert_eq!(fitted_grid(GridSpec::new(5, 2), OLED).key(), "5x2");
    assert!(
        GridSpec::parse("4x3") == Some(GridSpec::new(4, 3)) && GridSpec::parse("quatsch").is_none()
    );
}

#[test]
fn test_fixed_grid_keeps_cell_size() {
    let g = GridSpec::new(3, 3);
    let (cell_w, cell_h) = ((OLED.width() / 3.0).floor(), (OLED.height() / 3.0).floor());
    let four = fixed_grid(OLED, g, &[7, 8, 9, 10]);
    assert_eq!(four.len(), 4);
    for r in four.values() {
        assert!(
            r.width() == cell_w && r.height() == cell_h,
            "every tile a ninth, got {r:?}"
        );
    }
    assert!(
        four[&7].min_x() == OLED.min_x() && four[&7].max_y() == OLED.max_y(),
        "first place top left"
    );
    assert!(
        four[&10].min_x() == OLED.min_x() && four[&10].max_y() == OLED.max_y() - cell_h,
        "fourth: second row left"
    );

    // More windows than places: the others stay untouched.
    let ten = fixed_grid(OLED, g, &(0..10).collect::<Vec<_>>());
    assert!(
        ten.len() == 9 && !ten.contains_key(&9),
        "the tenth window gets no place"
    );
    for (a, ra) in &ten {
        for (b, rb) in &ten {
            if a < b {
                assert!(!ra.intersects(*rb), "{a} and {b} overlap");
            }
        }
    }
}

#[test]
fn test_grid_ignores_clicks() {
    let w = spread_windows();
    let g = GridSpec::new(3, 3);
    let a = plan_layout(&PlanInput {
        focused: Some(0),
        grid: g,
        ..input(Grid, Single(0), &w, Right)
    });
    let b = plan_layout(&PlanInput {
        focused: Some(3),
        grid: g,
        ..input(Grid, Single(0), &w, Right)
    });
    assert_eq!(a, b, "a click rearranges nothing in the grid");
    assert_eq!(
        a.get(&0),
        fixed_grid(OLED, g, &[0]).get(&0),
        "order by first sighting, fixed place"
    );
}

// ---------- Fenster ausschliessen (Kevin 2026-09-18) ----------

#[test]
fn test_excluded_windows_stay_put() {
    let w = spread_windows();
    let raus = BTreeSet::from([w[2].sessions[0].id.clone()]);
    for target in [
        Single(0),
        Single(1),
        room(0, 1, Left, false),
        room(0, 1, Right, false),
    ] {
        for mode in LayoutMode::ALL {
            let plan = plan_layout(&PlanInput {
                focused: Some(2),
                excluded: &raus,
                ..input(mode, target, &w, Right)
            });
            assert_eq!(
                keys(&plan),
                [0, 1, 3],
                "{mode:?}/{target:?}: excluded window stays where it is"
            );
        }
    }
    assert!(is_excluded(&w[2], &raus) && !is_excluded(&w[1], &raus));
}

// ---------- Leitstand tests (Swift section "Fehler 3") ----------

/// Control room (Kevin 2026-09-24): whoever is done or asks moves up from the MacBook LEFT beside the active
/// session; the right stays free for Chrome, the MacBook shows what runs.
#[test]
fn test_leitstand() {
    let w = spread_windows(); // 0 runs, 1 done, 2 question, 3 runs, 4 shell
    let target = room(0, 1, Left, false);
    let (main, left, right) = sides(OLED);
    let plan = plan_layout(&PlanInput {
        focused: Some(3),
        ..input(Reading, target, &w, Right)
    });
    assert_eq!(
        plan.get(&3),
        Some(&main),
        "active session centred on the big screen"
    );
    for i in [1, 2] {
        assert!(
            left.contains(plan[&i]),
            "window {i} waits -> left beside the active one"
        );
    }
    assert!(
        plan[&2].max_y() > plan[&1].max_y(),
        "the question stands above the one that is only done"
    );
    assert!(MACBOOK.contains(plan[&0]), "window 0 runs -> MacBook");
    assert!(
        !plan.values().any(|r| r.intersects(right)),
        "the right stays free for Chrome"
    );
    assert!(!plan.contains_key(&4), "plain shell stays where it is");
    let ohne = plan_layout(&input(Grid, target, &w, Right));
    assert_eq!(
        ohne.get(&0),
        Some(&main),
        "without active session: the one seen first"
    );

    // More waiting ones than the column holds: the rest queues LEFT on the MacBook.
    let t0 = 1000.0;
    let many: Vec<TermWindow> = (0..10).map(|i| fake_window(i > 0, t0 + i as f64)).collect();
    let full = plan_layout(&PlanInput {
        focused: Some(0),
        ..input(Reading, target, &many, Right)
    });
    assert!(
        (1..=6).all(|i| left.contains(full[&i])),
        "whoever was done first stands beside the active one"
    );
    for i in [7, 8, 9] {
        assert!(
            MACBOOK.contains(full[&i])
                && full[&i].max_x() <= MACBOOK.min_x() + mini_rail_width(MACBOOK),
            "window {i} queues left on the MacBook"
        );
    }
    // The MacBook column full too (6 + 5): the rest stands first in the grid instead of lying where another window
    // goes meanwhile (review quick 2026-09-24).
    let flood: Vec<TermWindow> = (0..15).map(|i| fake_window(i > 0, t0 + i as f64)).collect();
    let flooded = plan_layout(&PlanInput {
        focused: Some(0),
        ..input(Reading, target, &flood, Right)
    });
    assert_eq!(flooded.len(), 15, "every window gets a place");
    let rects: Vec<Rect> = flooded.values().copied().collect();
    for i in 0..rects.len() {
        for j in i + 1..rects.len() {
            assert!(
                !rects[i].intersects(rects[j]),
                "no overlap: {:?} / {:?}",
                rects[i],
                rects[j]
            );
        }
    }
}

/// Control room 2 (Kevin 2026-09-24): "fertig sollte immer neben aktiv sein". Big screen: left free for Chrome,
/// middle active, right beside it whoever waits; MacBook: what runs.
#[test]
fn test_leitstand2() {
    let mut w = spread_windows();
    let target = room(0, 1, Right, false);
    let (main, left, right) = sides(OLED);
    let plan = plan_layout(&PlanInput {
        focused: Some(0),
        ..input(Reading, target, &w, Left)
    });
    assert_eq!(
        plan.get(&0),
        Some(&main),
        "active session centred on the big screen"
    );
    for i in [1, 2] {
        assert!(
            right.contains(plan[&i]),
            "window {i} waits -> right beside the active one"
        );
    }
    assert!(
        plan[&2].max_y() > plan[&1].max_y(),
        "the question stands above the one that is only done"
    );
    assert!(MACBOOK.contains(plan[&3]), "window 3 runs -> MacBook");
    assert!(
        !plan.values().any(|r| r.intersects(left)),
        "the left stays free for Chrome"
    );
    assert!(!plan.contains_key(&4), "plain shell stays where it is");

    // Status changes: 3 gets done -> up beside the active one, 1 works again -> onto the MacBook.
    w[3] = fake_window_with(true, w[3].first_seen, Some(9000.0), false, true);
    w[1] = fake_window(false, w[1].first_seen);
    let after = plan_layout(&PlanInput {
        focused: Some(0),
        ..input(Reading, target, &w, Right)
    });
    assert!(
        right.contains(after[&3]) && MACBOOK.contains(after[&1]),
        "follows the status"
    );

    // More waiting ones than the column holds: the rest queues right on the MacBook.
    let t0 = 1000.0;
    let many: Vec<TermWindow> = (0..10).map(|i| fake_window(i > 0, t0 + i as f64)).collect();
    let full = plan_layout(&PlanInput {
        focused: Some(0),
        ..input(Reading, target, &many, Right)
    });
    let in_column: Vec<usize> = (1..10).filter(|i| right.contains(full[i])).collect();
    assert_eq!(
        in_column,
        (1..=6).collect::<Vec<_>>(),
        "whoever was done first stands beside the active one"
    );
    for i in [7, 8, 9] {
        assert!(
            MACBOOK.contains(full[&i])
                && full[&i].min_x() >= MACBOOK.max_x() - mini_rail_width(MACBOOK),
            "window {i} queues right on the MacBook"
        );
    }
    assert_eq!(
        resolve_target(&Both2, &lg_and_macbook(), None),
        Some(target)
    );
    assert_eq!(
        resolve_target(&Both2, &[lg_and_macbook()[1].clone()], None),
        Some(Single(0)),
        "needs two screens"
    );
}

/// Control room 3 (Kevin 2026-09-28): MacBook = work screen (waiting ones left, active right), the LG shows the
/// running ones at its left and right edge, the middle (half the screen) stays free for Chrome.
#[test]
fn test_leitstand3() {
    let w = spread_windows();
    let target = room(1, 0, Left, true);
    assert_eq!(
        resolve_target(&Both3, &lg_and_macbook(), None),
        Some(target)
    );
    assert_eq!(
        resolve_target(&Both3, &[lg_and_macbook()[1].clone()], None),
        Some(Single(0)),
        "needs two screens"
    );
    let (main, column) = focus_layout(MACBOOK, Left);
    let middle = Rect::new(
        OLED.min_x() + OLED.width() / 4.0,
        OLED.min_y(),
        OLED.width() / 2.0,
        OLED.height(),
    );
    let plan = plan_layout(&PlanInput {
        focused: Some(3),
        ..input(Reading, target, &w, Right)
    });
    assert_eq!(
        plan.get(&3),
        Some(&main),
        "active session on the MacBook, right"
    );
    for i in [1, 2] {
        assert!(
            column.contains(plan[&i]),
            "window {i} waits -> left beside it"
        );
    }
    assert!(
        plan[&2].max_y() > plan[&1].max_y(),
        "the question stands above the one that is only done"
    );
    assert!(
        OLED.contains(plan[&0]) && plan[&0].max_x() <= middle.min_x(),
        "window 0 runs -> LG left"
    );
    assert!(
        !plan.values().any(|r| r.intersects(middle)),
        "the middle stays free for Chrome"
    );
    assert!(!plan.contains_key(&4), "plain shell stays where it is");

    // Many: 5 waiting ones fit on the MacBook, the rest + the running spread left/right on the LG.
    let t0 = 1000.0;
    let many: Vec<TermWindow> = (0..12)
        .map(|i| fake_window(i > 0 && i < 9, t0 + i as f64))
        .collect();
    let full = plan_layout(&PlanInput {
        focused: Some(0),
        ..input(Reading, target, &many, Right)
    });
    assert!(
        (1..=5).all(|i| column.contains(full[&i])),
        "whoever was done first stands beside the active one"
    );
    assert!(
        (6..12).all(|i| OLED.contains(full[&i]) && !full[&i].intersects(middle)),
        "the rest at the LG edges"
    );
    // Flood: every window gets a place, nothing overlaps (if need be in the middle).
    let flood: Vec<TermWindow> = (0..25).map(|i| fake_window(false, t0 + i as f64)).collect();
    let flooded = plan_layout(&PlanInput {
        focused: Some(0),
        ..input(Reading, target, &flood, Right)
    });
    assert_eq!(flooded.len(), 25, "every window gets a place");
    let rects: Vec<Rect> = flooded.values().copied().collect();
    for i in 0..rects.len() {
        for j in i + 1..rects.len() {
            assert!(
                !rects[i].intersects(rects[j]),
                "no overlap: {:?} / {:?}",
                rects[i],
                rects[j]
            );
        }
    }
}

/// Preview page 2026-09-18: "a click on a tile SWAPS it", only the two windows involved move.
#[test]
fn test_click_swaps_only_two_windows() {
    let mut w = spread_windows(); // AI windows 0…3 in first-sighting order
    // All run: in the control room the status pulls done ones to the big screen anyway, and in Focus 1 only running
    // ones swap since plan 1d (question and done follow the time of being done, `test_click_across_groups`).
    for i in [1, 2] {
        let f = w[i].frame;
        w[i] = fake_window(false, w[i].first_seen);
        w[i].frame = f;
    }
    let ordered = layout_candidates(&w, &BTreeSet::new());
    assert_eq!(ordered, [0, 1, 2, 3]);
    for target in [Single(0), room(0, 1, Left, false), room(0, 1, Right, false)] {
        let vorher = plan_layout(&PlanInput {
            focused: Some(0),
            ..input(Reading, target, &w, Right)
        });
        let shown = rest_sequence(&ordered, 0, &[]);
        let rest = if target == Single(0) {
            status_swap(&shown, 0, 2, &w)
        } else {
            swapped_rest(&shown, 0, 2)
        };
        assert_eq!(rest, [1, 0, 3]);
        let nachher = plan_layout(&PlanInput {
            focused: Some(2),
            rest_hint: &rest,
            ..input(Reading, target, &w, Right)
        });
        assert_eq!(
            nachher.get(&2),
            vorher.get(&0),
            "{target:?}: the clicked window stands where the old main stood"
        );
        assert_eq!(
            nachher.get(&0),
            vorher.get(&2),
            "{target:?}: the old main window takes the freed place"
        );
        assert!(
            nachher.get(&1) == vorher.get(&1) && nachher.get(&3) == vorher.get(&3),
            "{target:?}: others stay"
        );
    }
    // The remembered order survives vanished and new windows.
    assert_eq!(
        rest_sequence(&[0, 1, 2, 3], 1, &[3, 9, 0]),
        [3, 0, 2],
        "9 is gone, 2 is new"
    );
}

#[test]
fn test_legacy_layout_names_still_work() {
    assert_eq!(
        LayoutMode::parse("grid32"),
        Grid,
        "old configuration does not fall to the default"
    );
    assert_eq!(LayoutMode::parse("grid22"), Grid);
    assert_eq!(
        LayoutMode::parse("stack"),
        Reading,
        "the stack is gone (Kevin 2026-09-18) -> Focus"
    );
    assert_eq!(
        LayoutMode::parse("focus"),
        Reading,
        "the old focus mode went into the reading width"
    );
    assert_eq!(LayoutMode::parse("focus2"), Focus2);
    assert_eq!(LayoutMode::parse("quatsch"), Grid);
    assert_eq!(
        LayoutMode::ALL,
        [Reading, Focus2, Grid],
        "Ctrl+Opt+1 Focus, 2 Focus 2, 3 Grid"
    );
    assert_eq!(
        LayoutMode::ALL.map(LayoutMode::label),
        ["Fokus", "Fokus 2", "Raster"]
    );
}

// ---------- Plan 1d: status decides the order, nothing disappears on the MacBook ----------

#[test]
fn test_rail_split() {
    let split = |cards, rows, more| RailSplit { cards, rows, more };
    assert_eq!(
        rail_split(1005.0, 8, 0),
        split(4, 4, 0),
        "4 × 192 + 28 + 4 × 46 = 980"
    );
    assert_eq!(rail_split(1005.0, 6, 0), split(4, 2, 0));
    assert_eq!(
        rail_split(1005.0, 5, 0),
        split(5, 0, 0),
        "all fits: no list"
    );
    assert_eq!(
        rail_split(1239.0, 6, 0),
        split(6, 0, 0),
        "LG: six windows per strip"
    );
    assert_eq!(
        rail_split(1239.0, 7, 0),
        split(6, 1, 0),
        "LG: from the 7th the list starts"
    );
    assert_eq!(rail_split(1005.0, 2, 8), split(2, 8, 0));
    assert_eq!(
        rail_split(1005.0, 0, 30),
        split(0, 20, 10),
        "+10 more (in the menu)"
    );
    assert_eq!(rail_split(1005.0, 0, 0), split(0, 0, 0));
}

#[test]
fn test_mac_book_focus1_three_columns() {
    let w = eleven_sessions();
    let p = PlanInput {
        focused: Some(0),
        ..input(Reading, Single(1), &w, Right)
    };
    let a = plan_arrangement(&p);
    let cols = three_column_layout(MACBOOK);
    assert!(cols.is_some(), "MacBook: three columns");
    let (left, main, right) = cols.unwrap();
    assert!(
        left.width() == MIN_CARD_W && right.width() == MIN_CARD_W,
        "both sides 480 pt"
    );
    assert!(
        main.width() == MACBOOK.width() - 2.0 * MIN_CARD_W && main.min_x() == left.max_x(),
        "middle = rest"
    );
    assert!(
        a.frames.get(&0) == Some(&main) && a.main == Some(0),
        "the active session in the middle"
    );
    for i in [1, 2] {
        assert!(
            left.contains(a.frames[&i]) && !a.parked.contains(&i),
            "window {i} waits -> left"
        );
    }
    assert!(
        a.frames[&2].max_y() > a.frames[&1].max_y(),
        "the question stands on top"
    );
    let rechts: Vec<usize> = (3..11).filter(|i| right.contains(a.frames[i])).collect();
    assert_eq!(
        rechts,
        [3, 4, 5, 6],
        "the first four running ones as windows on the right"
    );
    assert_eq!(
        a.parked,
        BTreeSet::from([7, 8, 9, 10]),
        "the rest is parked"
    );
    for i in &a.parked {
        assert_eq!(
            a.frames.get(i),
            Some(&main),
            "parked = exactly on the middle"
        );
    }
    assert_eq!(
        entries(&a),
        [vec![header("+4 weitere"), Row(7), Row(8), Row(9), Row(10)]]
    );
    assert!(
        right.contains(a.lists[0].rect) && a.lists[0].rect.min_y() == MACBOOK.min_y(),
        "list bottom right"
    );
    assert_roles(&a, 11, MACBOOK);
    assert_eq!(
        plan_layout(&p),
        a.frames,
        "plan_layout gives the same frames"
    );
}

#[test]
fn test_mac_book_focus2_one_rail() {
    let w = eleven_sessions();
    let a = plan_arrangement(&PlanInput {
        focused: Some(0),
        ..input(Focus2, Single(1), &w, Right)
    });
    let (main, left, rail) = reading_layout(MACBOOK, Right);
    assert!(
        left.is_none() && a.frames.get(&0) == Some(&main),
        "Focus 2 on the MacBook: one edge"
    );
    let rail = rail.unwrap();
    for i in [1, 2] {
        assert!(
            rail.contains(a.frames[&i]) && !a.parked.contains(&i),
            "2 cards"
        );
    }
    assert_eq!(
        a.parked,
        (3..11).collect::<BTreeSet<_>>(),
        "all 8 running ones parked"
    );
    for i in 3..11 {
        assert_eq!(a.frames.get(&i), Some(&main), "exactly on the main frame");
    }
    let expected: Vec<ListEntry> = std::iter::once(header("ARBEITET (8)"))
        .chain((3..11).map(Row))
        .collect();
    assert_eq!(entries(&a), [expected], "8 rows");
    assert!(rail.contains(a.lists[0].rect));
    assert_roles(&a, 11, MACBOOK);
}

/// Case 7 waiting + 3 running: extra waiting ones stand as rows, in Focus 1 left, in Focus 2 above WORKING.
#[test]
fn test_seven_waiting_three_running() {
    let t0 = 1000.0;
    let mut w = vec![fake_window(false, t0)];
    for i in 1..=7 {
        w.push(fake_window_with(
            true,
            t0 + i as f64,
            Some(5000.0 + i as f64),
            false,
            true,
        ));
    }
    for i in 8..=10 {
        w.push(fake_window(false, t0 + i as f64));
    }
    let f1 = plan_arrangement(&PlanInput {
        focused: Some(0),
        ..input(Reading, Single(1), &w, Right)
    });
    assert_eq!(
        f1.parked,
        BTreeSet::from([5, 6, 7]),
        "Focus 1: 4 waiting ones as windows, 3 as rows"
    );
    assert_eq!(
        entries(&f1),
        [vec![header("+3 weitere"), Row(5), Row(6), Row(7)]]
    );
    assert_roles(&f1, 11, MACBOOK);
    let f2 = plan_arrangement(&PlanInput {
        focused: Some(0),
        ..input(Focus2, Single(1), &w, Right)
    });
    assert_eq!(
        f2.parked,
        BTreeSet::from([4, 5, 6, 7, 8, 9, 10]),
        "Focus 2: 3 waiting ones as windows"
    );
    assert_eq!(
        entries(&f2),
        [vec![
            Row(4),
            Row(5),
            Row(6),
            Row(7),
            header("ARBEITET (3)"),
            Row(8),
            Row(9),
            Row(10)
        ]],
        "extra waiting ones right above WORKING"
    );
    assert_roles(&f2, 11, MACBOOK);
}

/// R2-2: when it gets tight, Lookout cuts the WORKING list first; waiting ones go first.
#[test]
fn test_focus2_waiting_first() {
    let t0 = 1000.0;
    let mut w = vec![fake_window(false, t0)];
    for i in 1..=5 {
        w.push(fake_window_with(
            true,
            t0 + i as f64,
            Some(5000.0 + i as f64),
            false,
            true,
        ));
    }
    for i in 6..=25 {
        w.push(fake_window(false, t0 + i as f64));
    }
    assert_eq!(
        rail_split(MACBOOK.height(), 5, 20),
        RailSplit {
            cards: 0,
            rows: 20,
            more: 5
        }
    );
    let a = plan_arrangement(&PlanInput {
        focused: Some(0),
        ..input(Focus2, Single(1), &w, Right)
    });
    let e = a
        .lists
        .first()
        .map(|l| l.entries.clone())
        .unwrap_or_default();
    assert_eq!(
        e.iter().take(5).cloned().collect::<Vec<_>>(),
        (1..=5).map(Row).collect::<Vec<_>>(),
        "all waiting ones"
    );
    assert!(
        e.contains(&header("ARBEITET (20)")) && e.last() == Some(&More(5)),
        "WORKING list ends with +5 more"
    );
    assert_eq!(e.iter().filter(|x| matches!(x, Row(_))).count(), 20);
    assert!(a.lists[0].rect.height() <= MACBOOK.height() && MACBOOK.contains(a.lists[0].rect));
    assert_eq!(
        a.parked.len(),
        25,
        "whoever has no row still lies parked (reachable in the menu)"
    );
    assert_eq!(a.lists[0].count(), 25, "the list counts rows and '+N more'");
}

/// LG, Focus 1: the side windows follow the status (question, done, running); the order fills left up to half, then
/// right.
#[test]
fn test_focus1_on_lg_by_status() {
    let w = spread_windows(); // 0 runs (active), 1 done, 2 question, 3 runs
    let plan = plan_layout(&PlanInput {
        focused: Some(0),
        ..input(Reading, Single(0), &w, Right)
    });
    let (main, left, right) = sides(OLED);
    assert_eq!(plan.get(&0), Some(&main));
    assert!(
        left.contains(plan[&2]) && left.contains(plan[&1]),
        "question and done left"
    );
    assert!(plan[&2].max_y() > plan[&1].max_y(), "the question in front");
    assert!(right.contains(plan[&3]), "running right");
}

/// GP 5 / R2-3: only running ones swap. A click across a group border: the old main goes to its group's place; if it
/// runs, to the start of the running ones.
#[test]
fn test_click_across_groups() {
    let w = spread_windows(); // 0 runs, 1 done, 2 question, 3 runs
    let a = status_swap(&[1, 2, 3], 0, 1, &w);
    assert_eq!(
        status_sequence(&a, &w),
        [2, 0, 3],
        "question first, the old main as first running one"
    );
    let b = status_swap(&[0, 2, 3], 1, 3, &w);
    assert_eq!(
        status_sequence(&b, &w),
        [2, 1, 0],
        "the done old main goes to the done ones"
    );
    assert_eq!(
        status_swap(&[1, 2, 3], 0, 3, &w),
        [1, 2, 0],
        "two running ones swap"
    );
    let plan = plan_layout(&PlanInput {
        focused: Some(1),
        rest_hint: &a,
        ..input(Reading, Single(0), &w, Right)
    });
    let (main, left, right) = sides(OLED);
    assert!(
        plan.get(&1) == Some(&main)
            && left.contains(plan[&2])
            && left.contains(plan[&0])
            && right.contains(plan[&3]),
        "the old main now stands left below the question"
    );
}

/// Narrow screen (1280 pt): no three columns (the middle would be narrower than Warp's minimum), one edge as before.
#[test]
fn test_narrow_screen_keeps_one_rail() {
    let narrow = Rect::new(0.0, 0.0, 1280.0, 800.0);
    assert!(
        three_column_layout(narrow).is_none(),
        "below 3 × 480 pt no three columns"
    );
    let w = spread_windows();
    let a = plan_arrangement(&PlanInput {
        focused: Some(0),
        ..PlanInput::new(Reading, Single(0), &w, &[narrow], Right)
    });
    let (main, left, right) = reading_layout(narrow, Right);
    assert!(
        left.is_none() && a.frames.get(&0) == Some(&main),
        "one edge as today"
    );
    let right = right.unwrap();
    for i in [1, 2, 3] {
        assert!(right.contains(a.frames[&i]), "window {i} on the edge");
    }
    assert!(
        a.frames[&2].max_y() > a.frames[&1].max_y() && a.frames[&1].max_y() > a.frames[&3].max_y(),
        "by status"
    );
}

// ---------- Phase 2: teilbar (only the asserts on wave A code; the rest is wave B) ----------

/// English interface: the same places as the German tests.
#[test]
fn test_english_texts() {
    set_german(false);
    let tones = [
        LabelTone::Waiting,
        LabelTone::Ready,
        LabelTone::Running,
        LabelTone::Watching,
    ];
    assert_eq!(
        tones.map(LabelTone::word),
        ["WAITING FOR YOU", "DONE", "WORKING", "WATCHING"]
    );
    assert_eq!(
        LayoutMode::ALL.map(LayoutMode::label),
        ["Focus", "Focus 2", "Grid"]
    );
    let w = eleven_sessions();
    let f1 = plan_arrangement(&PlanInput {
        focused: Some(0),
        ..input(Reading, Single(1), &w, Right)
    });
    assert_eq!(
        f1.lists.first().and_then(|l| l.entries.first()),
        Some(&header("+4 more"))
    );
    let f2 = plan_arrangement(&PlanInput {
        focused: Some(0),
        ..input(Focus2, Single(1), &w, Right)
    });
    assert_eq!(
        f2.lists.first().and_then(|l| l.entries.first()),
        Some(&header("WORKING (8)"))
    );
}

// ---------- Jeder Bildschirm (Kevin 2026-10-01 evening) ----------
//
// "Wer es runterlaedt, hat nicht unbedingt einen LG und ein MacBook": every screen is recognised and selectable with
// its real name; from three screens on the two control-room screens are chosen in the menu.

fn screen(x: f64, y: f64, w: f64, h: f64, builtin: bool, id: &str, name: &str) -> ScreenInfo {
    ScreenInfo {
        id: id.into(),
        name: name.into(),
        ..ScreenInfo::new(Rect::new(x, y, w, h), builtin)
    }
}

fn dell_links() -> ScreenInfo {
    screen(0.0, 0.0, 2560.0, 1415.0, false, "A", "DELL U2723QE")
}

fn dell_rechts() -> ScreenInfo {
    screen(2560.0, 0.0, 2560.0, 1440.0, false, "B", "DELL U2723QE")
}

fn hp(x: f64) -> ScreenInfo {
    screen(x, 0.0, 1920.0, 1080.0, false, "H", "HP 24")
}

fn small_macbook() -> ScreenInfo {
    screen(600.0, -982.0, 1512.0, 944.0, true, "M", "MacBook")
}

fn pair(work: &str, control: &str) -> Pair<String> {
    Pair {
        work: work.into(),
        control: control.into(),
    }
}

fn ids(s: &[ScreenInfo]) -> Vec<&str> {
    s.iter().map(|x| x.id.as_str()).collect()
}

fn titles(s: &[ScreenInfo]) -> Vec<String> {
    target_options(s)
        .iter()
        .filter_map(|o| target_title(o, s, None))
        .collect()
}

fn screen_choice(id: &str) -> TargetChoice {
    Screen(id.into())
}

/// Mac mini with two equal monitors: both selectable, equal names get (1)/(2) from the left.
#[test]
fn test_every_screen_selectable() {
    let mini = [dell_links(), dell_rechts()];
    assert_eq!(
        screen_names(&mini),
        ["DELL U2723QE (1)", "DELL U2723QE (2)"]
    );
    let opts = target_options(&mini);
    assert_eq!(
        opts,
        [screen_choice("A"), screen_choice("B"), Both, Both2, Both3],
        "every screen, from the left"
    );
    assert_eq!(
        target_options(&[dell_rechts(), dell_links()]),
        opts,
        "order as on the desk, not as macOS lists them"
    );
    assert_eq!(
        opts.iter()
            .filter_map(|o| target_title(o, &mini, None))
            .collect::<Vec<_>>(),
        [
            "Alle auf DELL U2723QE (1)",
            "Alle auf DELL U2723QE (2)",
            "Leitstand (fertige links)",
            "Leitstand 2 (fertige rechts)",
            "Leitstand 3 (aktiv auf DELL U2723QE (1))"
        ]
    );
    assert!(
        resolve_target(&screen_choice("A"), &mini, None) == Some(Single(0))
            && resolve_target(&screen_choice("B"), &mini, None) == Some(Single(1))
    );
    assert_eq!(
        resolve_target(&screen_choice("weg"), &mini, None),
        Some(Single(1)),
        "unplugged: the largest present"
    );
    assert_eq!(
        target_title(&screen_choice("weg"), &mini, None),
        None,
        "not attached: no entry"
    );
    // The raw value is stored; the previous values stay readable.
    assert!(
        TargetChoice::from_raw("screen:A") == Some(screen_choice("A"))
            && screen_choice("A").raw() == "screen:A"
    );
    assert_eq!(
        ["external", "builtin", "both", "both2", "both3"]
            .into_iter()
            .filter_map(TargetChoice::from_raw)
            .collect::<Vec<_>>(),
        [External, Builtin, Both, Both2, Both3]
    );
    assert!(
        TargetChoice::from_raw("quatsch").is_none() && TargetChoice::from_raw("screen:").is_none()
    );
}

/// Two equal monitors without serial number can report the same display id: then the position decides.
#[test]
fn test_duplicate_screen_ids() {
    let (mut a, mut b) = (dell_links(), dell_rechts());
    a.id = "X".into();
    b.id = "X".into();
    let u = unique_screen_ids(&[b.clone(), a.clone()]); // macOS lists the right one first
    assert_eq!(
        ids(&u),
        ["X#2", "X"],
        "the left keeps the id, the right gets #2"
    );
    assert_eq!(
        target_options(&u),
        [screen_choice("X"), screen_choice("X#2"), Both, Both2, Both3],
        "two entries"
    );
    assert_eq!(
        resolve_target(&screen_choice("X#2"), &u, None),
        Some(Single(0)),
        "the right one"
    );
    // review quick (Codex): if the left one is unplugged, the right one is called "X" again; "X#2" still finds it.
    let mb = screen(0.0, -982.0, 3000.0, 2000.0, true, "M", "MacBook");
    let nur_rechts = unique_screen_ids(&[b, mb]);
    assert_eq!(
        resolve_target(&screen_choice("X#2"), &nur_rechts, None),
        Some(Single(0)),
        "twin, not the largest screen"
    );
    assert_eq!(
        ids(&unique_screen_ids(&[dell_links(), dell_rechts()])),
        ["A", "B"],
        "different ids stay"
    );
    let mut leer = dell_rechts();
    leer.id = String::new();
    assert_eq!(
        ids(&unique_screen_ids(&[dell_links(), leer.clone(), leer])),
        ["A", "", ""],
        "unknown stays unknown"
    );
}

/// "MacBook" was fixed in the menu: wrong on an iMac, and on a Mac mini control room 3 said "active on the MacBook"
/// without one.
#[test]
fn test_screen_names_not_fixed() {
    let lg = ScreenInfo {
        id: "LG".into(),
        name: "LG ULTRAGEAR+".into(),
        ..ScreenInfo::new(OLED, false)
    };
    let mb = ScreenInfo {
        id: "MB".into(),
        name: "Built-in Retina Display".into(),
        ..ScreenInfo::new(MACBOOK, true)
    };
    let kevin = [lg, mb];
    assert_eq!(
        titles(&kevin),
        [
            "Alle auf LG ULTRAGEAR+",
            "Alle aufs MacBook",
            "Leitstand (fertige links)",
            "Leitstand 2 (fertige rechts)",
            "Leitstand 3 (aktiv aufs MacBook)"
        ],
        "Kevin's menu stays word for word"
    );
    let imac = ScreenInfo {
        laptop: false,
        ..screen(0.0, 0.0, 2240.0, 1235.0, true, "I", "iMac")
    };
    assert_eq!(
        titles(std::slice::from_ref(&imac)),
        ["Alle auf den iMac"],
        "iMac alone: no MacBook, no control room"
    );
    assert_eq!(
        target_title(&Both3, &[dell_rechts(), imac], None).as_deref(),
        Some("Leitstand 3 (aktiv auf den iMac)")
    );
    assert!(
        target_title(&External, &kevin, None).is_none()
            && target_title(&Builtin, &kevin, None).is_none(),
        "old values no longer stand as entries of their own"
    );
    set_german(false);
    assert_eq!(
        titles(&kevin),
        [
            "All on LG ULTRAGEAR+",
            "All on the MacBook",
            "Control room (done on the left)",
            "Control room 2 (done on the right)",
            "Control room 3 (active on the MacBook)"
        ]
    );
}

/// Three screens (MacBook + 2 monitors): automatically the largest and the smallest as before; selectable in the menu.
#[test]
fn test_leitstand_screens_choice() {
    let three = [dell_links(), hp(2560.0), small_macbook()];
    assert_eq!(
        resolve_target(&Both, &three, None),
        Some(room(0, 2, Left, false)),
        "automatic: largest + smallest"
    );
    let p = pair("H", "A");
    assert_eq!(
        resolve_target(&Both, &three, Some(&p)),
        Some(room(1, 0, Left, false)),
        "chosen"
    );
    assert_eq!(
        resolve_target(&Both2, &three, Some(&p)),
        Some(room(1, 0, Right, false))
    );
    assert_eq!(
        resolve_target(&Both3, &three, Some(&p)),
        Some(room(1, 0, Left, true))
    );
    assert_eq!(
        target_title(&Both3, &three, Some(&p)).as_deref(),
        Some("Leitstand 3 (aktiv auf HP 24)")
    );
    assert_eq!(
        resolve_target(
            &Both,
            &[dell_links(), small_macbook(), dell_rechts()],
            Some(&p)
        ),
        Some(room(2, 1, Left, false)),
        "chosen screen missing: automatic"
    );
    assert_eq!(
        resolve_target(&Both, &[dell_links(), hp(2560.0)], Some(&p)),
        Some(room(0, 1, Left, false)),
        "two screens: automatic, the choice counts from three on"
    );
    assert_eq!(
        resolve_target(&screen_choice("M"), &three, Some(&p)),
        Some(Single(2)),
        "single target untouched"
    );
    // The same screen for both roles swaps them.
    assert_eq!(
        choose_pair(&pair("A", "M"), Some("H"), None, &three),
        pair("H", "M")
    );
    assert_eq!(
        choose_pair(&pair("A", "M"), Some("M"), None, &three),
        pair("M", "A"),
        "swap"
    );
    assert_eq!(
        choose_pair(&pair("A", "M"), None, Some("A"), &three),
        pair("M", "A"),
        "swap"
    );
}

/// Menu check mark and arrangement resolve the chosen pair the same way (review quick 2026-10-01).
#[test]
fn test_pair_shared_resolution() {
    let (mut links, mut rechts) = (dell_links(), dell_rechts());
    links.id = "X".into();
    rechts.id = "X".into();
    let p = pair("X#2", "H");
    let vier = unique_screen_ids(&[links, rechts.clone(), hp(5120.0), small_macbook()]); // X, X#2, H, M
    assert_eq!(
        active_pair(Some(&p), &vier),
        Some(Pair {
            work: 1,
            control: 2
        }),
        "all there: the chosen pair"
    );
    // The left one is unplugged, the right one is called "X" again: arrangement and check mark both use the pair.
    let drei = unique_screen_ids(&[rechts.clone(), hp(5120.0), small_macbook()]); // X, H, M
    let active = active_pair(Some(&p), &drei);
    assert_eq!(
        active,
        Some(Pair {
            work: 0,
            control: 1
        }),
        "the twin counts in the menu too"
    );
    assert_eq!(
        resolve_target(&Both, &drei, Some(&p)),
        Some(room(0, 1, Left, false)),
        "the arrangement uses the pair"
    );
    assert_eq!(
        resolve_target(&Both, &drei, None),
        Some(room(0, 2, Left, false)),
        "automatic would be another"
    );
    // Otherwise automatic: two screens, empty id, both roles on the same screen, nothing chosen.
    assert_eq!(
        active_pair(Some(&p), &[rechts, small_macbook()]),
        None,
        "two screens"
    );
    assert_eq!(
        active_pair(Some(&pair("", "H")), &vier),
        None,
        "empty id does not count"
    );
    assert_eq!(
        active_pair(Some(&pair("X#2", "X")), &drei),
        None,
        "both roles on the one X"
    );
    assert_eq!(active_pair(None, &vier), None);
}

/// Equal monitors after a neighbour is unplugged: the choice stays with the right screen, also when it is set anew in
/// the menu (review quick 2026-10-01 round 2, GLM).
#[test]
fn test_twin_choice_survives_replug() {
    let dell = |x: f64| screen(x, 0.0, 2560.0, 1440.0, false, "X", "DELL U2723QE");
    let (links, mitte, rechts) = (dell(0.0), dell(2560.0), dell(5120.0));
    // Three equal ones, the right one (X#3) chosen, the left one unplugged: the right one is now X#2.
    let x3 = screen_choice("X#3");
    assert_eq!(
        resolve_target(
            &x3,
            &unique_screen_ids(&[mitte.clone(), rechts.clone()]),
            None
        ),
        Some(Single(1))
    );
    assert_eq!(
        resolve_target(&x3, &unique_screen_ids(&[links.clone(), rechts]), None),
        Some(Single(1)),
        "middle gone"
    );
    assert_eq!(
        resolve_target(
            &x3,
            &unique_screen_ids(&[links.clone(), mitte.clone()]),
            None
        ),
        Some(Single(1)),
        "right gone"
    );
    // Pair work = right one of two equal ones (X#2), control HP. Left unplugged, only control moved to the MacBook:
    // X#2 stays stored; otherwise "X" would mean the left one after replugging.
    let ohne_links = unique_screen_ids(&[mitte.clone(), hp(5120.0), small_macbook()]);
    let neu = choose_pair(&pair("X#2", "H"), None, Some("M"), &ohne_links);
    assert_eq!(neu, pair("X#2", "M"));
    assert_eq!(
        resolve_target(
            &Both,
            &unique_screen_ids(&[links, mitte, hp(5120.0), small_macbook()]),
            Some(&neu)
        ),
        Some(room(1, 3, Left, false)),
        "replugged: work stays the right one"
    );
    // The same screen under the short name: swap or change nothing, the more precise id stays.
    assert_eq!(
        choose_pair(&pair("X#2", "H"), None, Some("X"), &ohne_links),
        pair("H", "X#2")
    );
    assert_eq!(
        choose_pair(&pair("X#2", "H"), Some("X"), None, &ohne_links),
        pair("X#2", "H")
    );
    assert!(same_screen("X", "X#2", &ohne_links) && !same_screen("X", "H", &ohne_links));
    assert!(!same_screen("weg", "auch-weg", &ohne_links) && same_screen("weg", "weg", &ohne_links));
    // Broken stored id with a huge number: answered at once, not millions of rounds per tick.
    let t0 = std::time::Instant::now();
    assert_eq!(
        screen_index_by_id("X#10000000", &ohne_links),
        Some(0),
        "the remaining twin"
    );
    assert_eq!(
        screen_index_by_id("X#9223372036854775807", &ohne_links),
        Some(0),
        "largest number (review quick R3)"
    );
    assert_eq!(
        screen_index_by_id("X#99999999999999999999", &ohne_links),
        None,
        "too large for a number: nothing"
    );
    assert!(t0.elapsed().as_secs_f64() < 0.1, "took {:?}", t0.elapsed());
}

// ---------- Parking lot (Kevin 2026-11-06) ----------

/// A window whose (only) session is called `name`.
fn named(name: &str, ready: bool, first_seen: Time) -> TermWindow {
    let mut w = fake_window(ready, first_seen);
    w.sessions[0].name = name.into();
    w
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// 0 lead (main), 1 worker running, 2 worker watching, 3 done, 4 running, 5 question.
fn lot_windows() -> Vec<TermWindow> {
    let t0 = 1000.0;
    let mut w = vec![
        named("Ultivor · Team lead", false, t0),
        named("Ultivor · Worker 1", false, t0 + 1.0),
        named("Ultivor · Worker 2", false, t0 + 2.0),
        named("Shop", true, t0 + 3.0),
        named("Docs", false, t0 + 4.0),
        fake_window_with(true, t0 + 5.0, None, true, true),
    ];
    w[2].sessions[0].watching = true;
    w
}

#[test]
fn parking_running_and_watching_workers_park_on_main() {
    let w = lot_windows();
    let (workers, leads) = (names(&["Worker"]), names(&["Ultivor · Team lead"]));
    let p = PlanInput {
        focused: Some(0),
        worker_names: &workers,
        lead_names: &leads,
        ..input(Reading, Single(0), &w, Right)
    };
    let a = plan_arrangement(&p);
    let (main, left, right) = sides(OLED);
    assert!(w[2].is_watching(), "fixture: window 2 watches");
    assert_eq!(
        a.lot,
        BTreeSet::from([1, 2]),
        "running and watching worker are in the lot"
    );
    for i in [1, 2] {
        assert!(
            a.frames.get(&i) == Some(&main) && a.parked.contains(&i),
            "worker {i} lies behind the main window"
        );
    }
    assert!(
        left.contains(a.frames[&5]) && left.contains(a.frames[&3]) && right.contains(a.frames[&4]),
        "others as before"
    );
    assert_roles(&a, 6, OLED);
}

#[test]
fn parking_ready_and_asking_workers_stay_in_waiting_column() {
    let t0 = 1000.0;
    let mut w = vec![
        named("Main", false, t0),
        named("worker done", true, t0 + 1.0),
        named("worker asks", true, t0 + 2.0),
        named("worker runs", false, t0 + 3.0),
        named("Docs", false, t0 + 4.0),
    ];
    w[2].sessions[0].waiting = true;
    let workers = names(&["WORKER"]);
    let a = plan_arrangement(&PlanInput {
        focused: Some(0),
        worker_names: &workers,
        ..input(Focus2, Single(0), &w, Right)
    });
    let (main, left, right) = sides(OLED);
    assert_eq!(a.lot, BTreeSet::from([3]));
    assert!(
        left.contains(a.frames[&1]) && left.contains(a.frames[&2]),
        "done and asking workers wait on the left"
    );
    assert!(
        a.frames[&2].max_y() > a.frames[&1].max_y(),
        "the question on top"
    );
    assert!(
        right.contains(a.frames[&4]) && a.frames.get(&3) == Some(&main),
        "running: right, worker: parked"
    );
    assert_eq!(a.parked, BTreeSet::from([3]));
    assert_roles(&a, 5, OLED);
}

#[test]
fn parking_never_takes_lead_main_excluded_or_hidden_question() {
    let t0 = 1000.0;
    let mut hidden = tab_window(
        vec![
            Session {
                digest: Some(Digest {
                    name: Some("Worker D".into()),
                    ..Digest::default()
                }),
                ..Session::new(
                    "claude:d",
                    Engine::Claude,
                    Some(4),
                    "Worker D",
                    "",
                    false,
                    true,
                )
            },
            tab_session("old", true, true, Some("Alte Aufgabe")),
        ],
        "◐ Worker D",
    );
    hidden.first_seen = t0 + 3.0;
    let w = vec![
        named("Worker A", false, t0),
        named("Worker lead", false, t0 + 1.0),
        named("Worker C", false, t0 + 2.0),
        hidden,
        named("Worker E", false, t0 + 4.0),
    ];
    assert!(
        w[3].hidden_question() && is_worker(&w[3], &names(&["worker"]), &[]),
        "fixture: worker with hidden question"
    );
    let excluded = BTreeSet::from([w[2].sessions[0].id.clone()]);
    let (workers, leads) = (names(&["worker"]), names(&["Worker lead"]));
    let a = plan_arrangement(&PlanInput {
        focused: Some(0),
        worker_names: &workers,
        lead_names: &leads,
        excluded: &excluded,
        ..input(Reading, Single(0), &w, Right)
    });
    let main = reading_layout(OLED, Right).0;
    assert_eq!(
        a.lot,
        BTreeSet::from([4]),
        "only the plain running worker parks"
    );
    assert!(
        a.frames.get(&0) == Some(&main) && !a.parked.contains(&0),
        "the focused worker is the main window"
    );
    assert!(
        a.frames.contains_key(&1) && a.frames.get(&1) != Some(&main),
        "the lead keeps its own place"
    );
    assert!(
        !a.frames.contains_key(&2),
        "the excluded worker stays where it is"
    );
    assert!(
        a.frames.contains_key(&3) && !a.parked.contains(&3),
        "a hidden question keeps the worker visible"
    );
}

#[test]
fn parking_needs_a_visible_session() {
    let both = |title: &str| {
        let mut w = tab_window(
            vec![
                tab_session("worker-1", false, false, Some("Eins")),
                tab_session("worker-2", false, false, Some("Zwei")),
            ],
            title,
        );
        w.first_seen = 1001.0;
        w
    };
    let workers = names(&["worker"]);
    assert!(
        both("Beta").visible_session().is_none(),
        "fixture: two Claude tabs, no glyph"
    );
    assert!(
        !is_worker(&both("Beta"), &workers, &[]),
        "no visible session, no worker"
    );
    assert!(
        is_worker(&both("◐ Zwei"), &workers, &[]),
        "visible tab named worker-2"
    );
    assert!(
        !is_worker(&tab_window(vec![], "◐ worker"), &workers, &[]),
        "no session at all"
    );
    let w = vec![named("Main", false, 1000.0), both("Beta")];
    let a = plan_arrangement(&PlanInput {
        focused: Some(0),
        worker_names: &workers,
        ..input(Reading, Single(0), &w, Right)
    });
    assert!(a.lot.is_empty() && a.parked.is_empty());
}

#[test]
fn parking_matches_case_insensitively_and_by_transcript_name() {
    let workers = names(&["Worker"]);
    assert!(
        is_worker(&named("ultivor · WORKER 3", false, 1.0), &workers, &[]),
        "case-insensitive"
    );
    let mut by_transcript = named("2f9c-uuid", false, 1.0);
    by_transcript.sessions[0].digest = Some(Digest {
        name: Some("Shop · worker".into()),
        ..Digest::default()
    });
    assert!(
        is_worker(&by_transcript, &workers, &[]),
        "own name from the transcript"
    );
    assert!(
        is_worker(&named("Straße 1", false, 1.0), &names(&["STRASSE"]), &[]),
        "ß matches ss like Foundation"
    );
    assert!(!is_worker(&named("Docs", false, 1.0), &workers, &[]));
    assert!(
        !is_worker(&named("Docs", false, 1.0), &names(&[""]), &[]),
        "an empty name matches nothing"
    );
    assert!(
        !is_worker(
            &named("Worker lead", false, 1.0),
            &workers,
            &names(&["Worker lead"])
        ),
        "lead by status name"
    );
    assert!(
        !is_worker(&by_transcript, &workers, &names(&["Shop · worker"])),
        "lead by own name"
    );
    assert!(
        is_worker(
            &named("Worker lead 2", false, 1.0),
            &workers,
            &names(&["Worker lead"])
        ),
        "lead means exactly"
    );
}

#[test]
fn parking_in_every_control_room_target() {
    let t0 = 1000.0;
    let w = vec![
        named("Main", false, t0),
        named("worker", false, t0 + 1.0),
        named("Shop", true, t0 + 2.0),
        named("Docs", false, t0 + 3.0),
    ];
    let workers = names(&["worker"]);
    for target in [
        room(0, 1, Left, false),
        room(0, 1, Right, false),
        room(1, 0, Left, true),
    ] {
        for mode in LayoutMode::ALL {
            let a = plan_arrangement(&PlanInput {
                focused: Some(0),
                worker_names: &workers,
                ..input(mode, target, &w, Right)
            });
            assert!(
                a.lot == BTreeSet::from([1])
                    && a.parked == BTreeSet::from([1])
                    && a.frames.get(&1) == a.frames.get(&0),
                "{target:?}/{mode:?}: the worker parks behind the main window"
            );
            assert_eq!(
                keys(&a.frames),
                [0, 1, 2, 3],
                "{target:?}/{mode:?}: the others keep their places"
            );
        }
    }
}

#[test]
fn parking_none_in_single_screen_grid() {
    let w = lot_windows();
    let workers = names(&["Worker"]);
    let plain = input(Grid, Single(0), &w, Right);
    let a = plan_arrangement(&PlanInput {
        focused: Some(0),
        worker_names: &workers,
        ..plain
    });
    assert!(
        a.lot.is_empty() && a.parked.is_empty(),
        "the grid has no lot"
    );
    assert_eq!(
        a,
        plan_arrangement(&PlanInput {
            focused: Some(0),
            ..plain
        }),
        "same as without worker names"
    );
}

#[test]
fn parking_focus1_mac_book_three_columns() {
    let mut w = eleven_sessions();
    for i in [3, 4] {
        w[i].sessions[0].name = format!("Worker {i}");
    }
    let workers = names(&["worker"]);
    let a = plan_arrangement(&PlanInput {
        focused: Some(0),
        worker_names: &workers,
        ..input(Reading, Single(1), &w, Right)
    });
    let (_, main, right) = three_column_layout(MACBOOK).unwrap();
    assert!(
        a.lot == BTreeSet::from([3, 4])
            && a.frames.get(&3) == Some(&main)
            && a.frames.get(&4) == Some(&main)
    );
    let rechts: Vec<usize> = (5..11).filter(|i| right.contains(a.frames[i])).collect();
    assert_eq!(
        rechts,
        [5, 6, 7, 8],
        "the next running ones move up as windows"
    );
    assert_eq!(a.parked, BTreeSet::from([3, 4, 9, 10]));
    assert_eq!(
        entries(&a),
        [vec![header("+2 weitere"), Row(9), Row(10)]],
        "the lot has no rows"
    );
    assert_roles(&a, 11, MACBOOK);
}

#[test]
fn parking_focus2_mac_book_one_rail() {
    let mut w = eleven_sessions();
    for i in [3, 4] {
        w[i].sessions[0].name = format!("worker-{i}");
    }
    let workers = names(&["Worker"]);
    let a = plan_arrangement(&PlanInput {
        focused: Some(0),
        worker_names: &workers,
        ..input(Focus2, Single(1), &w, Right)
    });
    let main = reading_layout(MACBOOK, Right).0;
    assert!(
        a.lot == BTreeSet::from([3, 4])
            && a.frames.get(&3) == Some(&main)
            && a.frames.get(&4) == Some(&main)
    );
    assert_eq!(a.parked, (3..11).collect::<BTreeSet<_>>());
    let expected: Vec<ListEntry> = std::iter::once(header("ARBEITET (6)"))
        .chain((5..11).map(Row))
        .collect();
    assert_eq!(
        entries(&a),
        [expected],
        "WORKING lists the running ones outside the lot"
    );
    assert_roles(&a, 11, MACBOOK);
}

#[test]
fn parking_peek_tiles_the_largest_screen() {
    let t0 = 1000.0;
    let mut w = vec![named("Main", false, t0), named("Shop", true, t0 + 1.0)];
    w.extend((2..9).map(|i| named(&format!("worker {i}"), false, t0 + i as f64)));
    let workers = names(&["worker"]);
    let base = PlanInput {
        focused: Some(0),
        worker_names: &workers,
        ..input(Reading, Single(1), &w, Right)
    };
    let parked = plan_arrangement(&base);
    let peek = plan_arrangement(&PlanInput { peek: true, ..base });
    assert_eq!(
        peek.lot,
        (2..9).collect::<BTreeSet<_>>(),
        "the lot is complete"
    );
    assert!(
        peek.lot
            .iter()
            .all(|i| !peek.parked.contains(i) && OLED.contains(peek.frames[i])),
        "tiled on the LG"
    );
    let tiles: Vec<Rect> = peek.lot.iter().map(|i| peek.frames[i]).collect();
    for i in 0..tiles.len() {
        for j in i + 1..tiles.len() {
            assert!(
                !tiles[i].intersects(tiles[j]),
                "tiles overlap: {:?} / {:?}",
                tiles[i],
                tiles[j]
            );
        }
    }
    assert!(
        peek.frames[&0] == parked.frames[&0] && peek.frames[&1] == parked.frames[&1],
        "the rest stays"
    );
    assert!(
        parked.lot == peek.lot && parked.lot == parked.parked,
        "without peek the same lot lies parked"
    );

    // 26 workers: 18 grid tiles + 6 in the mini stack; the last two by first sighting stay parked.
    let mut many = vec![named("Main", false, t0)];
    many.extend((1..27).map(|i| named(&format!("worker {i}"), false, t0 + i as f64)));
    let full = plan_arrangement(&PlanInput {
        peek: true,
        ..PlanInput {
            windows: &many,
            ..base
        }
    });
    assert_eq!(full.lot.len(), 26);
    assert_eq!(
        full.parked,
        BTreeSet::from([25, 26]),
        "overflow stays parked"
    );
    assert!(
        full.parked
            .iter()
            .all(|i| full.frames[i] == full.frames[&0]),
        "behind the main window"
    );
}

/// The plan line is Swift's (log 2026-11-06 17:22:17), plus the parking lot.
#[test]
fn plan_log_line_like_swift() {
    let target = Target::ControlRoom {
        work: 1,
        control: 0,
        ready_side: Left,
        flipped: true,
    };
    let mut arr = Arrangement::default();
    for i in 0..15 {
        arr.frames.insert(i, Rect::ZERO);
    }
    assert_eq!(
        plan_log_line(&Both3, &target, Reading, &arr),
        "Plan: ziel=both3 -> leitstand(work: 1, control: 0, readySide: Lookout.StackSide.left, flipped: true), \
         anordnung=reading, 15 Fenster"
    );
    arr.parked = BTreeSet::from([3, 4, 5]);
    arr.lot = BTreeSet::from([4, 5]);
    assert_eq!(
        plan_log_line(&Screen("ID".into()), &Single(0), Focus2, &arr),
        "Plan: ziel=screen:ID -> single(0), anordnung=focus2, 15 Fenster, 1 geparkt, Parkplatz 2"
    );
}
