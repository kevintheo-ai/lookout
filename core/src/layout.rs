//! Layout math. Pure functions, everything in Cocoa coordinates (origin bottom-left); conversion to AX happens only
//! when a frame is written to a window. Slot 0 is always top-left, then row by row to the right and down.

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};

use crate::geom::{Rect, cmp_f64};
use crate::i18n::tr;
use crate::model::{
    Arrangement, CardButton, GridSpec, LabelTone, LayoutMode, ListEntry, Pair, RailSplit,
    ScreenInfo, SideList, StackSide, Target, TargetChoice, TermWindow, Time, fold_case,
};

/// Smallest window Warp allows (spike: 480×192; Ghostty 90×48).
pub const MIN_CARD_W: f64 = 480.0;
pub const MIN_CARD_H: f64 = 192.0;
/// Target width of a terminal tile: 1000 pt hold about 119 columns of 14-pt monospace, enough for code without
/// unreadably long lines.
pub const IDEAL_CARD_W: f64 = 1000.0;
/// Comfortable reading width for ONE big window. Across a full 21:9 screen a line would hold ~358 characters.
pub const READING_WIDTH: f64 = 1400.0;
/// List row: two text lines like the labels (2 × 19 pt) plus 4 pt margin each. The header is smaller.
pub const LIST_ROW_H: f64 = 46.0;
pub const LIST_HEADER_H: f64 = 28.0;

/// Swift `Int(x)` for the small counts here: truncates toward zero. Swift traps on NaN and overflow; this saturates.
fn int(x: f64) -> i64 {
    x as i64
}

/// `max(1, Int(floor(x)))` as a count.
fn count_at_least_one(x: f64) -> usize {
    int(x.floor()).max(1) as usize
}

/// Columns and rows for `count` windows on this screen. Fill the width first (full height per window), rows only when
/// the columns run out. Minimum sizes are never undercut.
pub fn auto_grid(vis: Rect, count: usize) -> (usize, usize) {
    if count == 0 {
        return (1, 1);
    }
    let by_ideal = int((vis.width() / IDEAL_CARD_W).round()).max(1);
    let by_minimum = int((vis.width() / MIN_CARD_W).floor()).max(1);
    let cols = (by_ideal.min(by_minimum) as usize).min(count).max(1);
    let max_rows = count_at_least_one(vis.height() / MIN_CARD_H);
    (cols, max_rows.min(count.div_ceil(cols)).max(1))
}

#[derive(Clone, Debug, PartialEq)]
pub struct GridResult<K> {
    pub frames: BTreeMap<K, Rect>,
    pub overflow: Vec<K>,
}

/// Width of the mini stack at the edge: a sixth of the area, never below the minimum width Warp allows (spike: 480 pt).
pub fn mini_rail_width(vis: Rect) -> f64 {
    MIN_CARD_W.max((vis.width() / 6.0).floor())
}

fn rail_rect(vis: Rect, side: StackSide, width: f64) -> Rect {
    let x = if side == StackSide::Right {
        vis.max_x() - width
    } else {
        vis.min_x()
    };
    Rect::new(x, vis.min_y(), width, vis.height())
}

/// The area without the edge strip.
pub fn main_area(vis: Rect, side: StackSide, rail_width: f64) -> Rect {
    let x = if side == StackSide::Right {
        vis.min_x()
    } else {
        vis.min_x() + rail_width
    };
    Rect::new(x, vis.min_y(), vis.width() - rail_width, vis.height())
}

/// cols×rows tiles. If not all ids fit, the extra ones go into the mini stack, and the tiles only get the remaining
/// area (rail taken off first).
pub fn grid_layout<K: Ord + Clone>(
    vis: Rect,
    cols: usize,
    rows: usize,
    side: StackSide,
    ids: &[K],
) -> GridResult<K> {
    let mut out = GridResult {
        frames: BTreeMap::new(),
        overflow: vec![],
    };
    if cols == 0 || rows == 0 || ids.is_empty() {
        return out;
    }
    let capacity = cols.saturating_mul(rows);
    let overflow = ids.len() > capacity;
    if overflow {
        out.overflow = ids[capacity..].to_vec();
    }
    let area = if overflow {
        main_area(vis, side, mini_rail_width(vis))
    } else {
        vis
    };
    // Shrink the grid to the actual number: a single window belongs on the whole screen, not in the top-left corner of
    // an empty 2×2 (Kevin, live run 2026-09-18). The capacity stays cols×rows; it only decides the overflow.
    let shown = ids.len().min(capacity);
    let use_cols = cols.min(shown).max(1);
    let use_rows = shown.div_ceil(use_cols).max(1);
    let cell_w = (area.width() / use_cols as f64).floor();
    let cell_h = (area.height() / use_rows as f64).floor();
    for (i, id) in ids.iter().take(capacity).enumerate() {
        let (col, row) = (i % use_cols, i / use_cols);
        let r = Rect::new(
            area.min_x() + col as f64 * cell_w,
            area.max_y() - (row + 1) as f64 * cell_h,
            cell_w,
            cell_h,
        );
        out.frames.insert(id.clone(), r);
    }
    out
}

/// Mini stack at the edge: equally wide cards one below the other. What no longer fits the height stays untouched:
/// better arrange fewer than stack windows on top of each other that nobody finds again.
pub fn mini_stack<K: Ord + Clone>(vis: Rect, side: StackSide, ids: &[K]) -> BTreeMap<K, Rect> {
    if ids.is_empty() {
        return BTreeMap::new();
    }
    let width = mini_rail_width(vis);
    let rail = rail_rect(vis, side, width);
    let count = ids
        .len()
        .min(count_at_least_one(rail.height() / MIN_CARD_H));
    let card_h = (rail.height() / count as f64).floor();
    let card = |i: usize| {
        Rect::new(
            rail.min_x(),
            rail.max_y() - (i + 1) as f64 * card_h,
            width,
            card_h,
        )
    };
    ids.iter()
        .take(count)
        .enumerate()
        .map(|(i, id)| (id.clone(), card(i)))
        .collect()
}

/// Focus view: one window fills the remaining area, the others stand beside it as a mini stack. Returns
/// `(main, rail)`.
pub fn focus_layout(vis: Rect, side: StackSide) -> (Rect, Rect) {
    let width = mini_rail_width(vis);
    (main_area(vis, side, width), rail_rect(vis, side, width))
}

/// Reading-width focus: the main window stands CENTRED at a comfortable reading width, the others lie as strips at
/// both edges. If two strips do not fit beside it (laptop), the view falls back to one strip and equals
/// `focus_layout`. Returns `(main, left, right)`.
pub fn reading_layout(vis: Rect, side: StackSide) -> (Rect, Option<Rect>, Option<Rect>) {
    let rail = mini_rail_width(vis);
    let both_fit = vis.width() >= READING_WIDTH + 2.0 * rail;
    if !both_fit {
        let (main, r) = focus_layout(vis, side);
        return (
            main,
            (side == StackSide::Left).then_some(r),
            (side == StackSide::Right).then_some(r),
        );
    }
    let main_w = READING_WIDTH.min(vis.width() - 2.0 * rail);
    let left_w = ((vis.width() - main_w) / 2.0).floor();
    let right_w = vis.width() - main_w - left_w;
    let left = Rect::new(vis.min_x(), vis.min_y(), left_w, vis.height());
    let main = Rect::new(vis.min_x() + left_w, vis.min_y(), main_w, vis.height());
    let right = Rect::new(main.max_x(), vis.min_y(), right_w, vis.height());
    (main, Some(left), Some(right))
}

/// Cards one below the other in any strip (not only at the screen edge).
pub fn strip_stack<K: Ord + Clone>(rail: Rect, ids: &[K]) -> BTreeMap<K, Rect> {
    let wide = rail.width() > 0.0;
    if ids.is_empty() || !wide {
        return BTreeMap::new();
    }
    let count = ids
        .len()
        .min(count_at_least_one(rail.height() / MIN_CARD_H));
    let card_h = (rail.height() / count as f64).floor();
    let card = |i: usize| {
        Rect::new(
            rail.min_x(),
            rail.max_y() - (i + 1) as f64 * card_h,
            rail.width(),
            card_h,
        )
    };
    ids.iter()
        .take(count)
        .enumerate()
        .map(|(i, id)| (id.clone(), card(i)))
        .collect()
}

/// Who needs Kevin first? (Kevin 2026-09-18) First sessions with an open QUESTION (nothing goes on without an
/// answer), then done sessions, and among those whoever was done FIRST; whoever is done later queues behind. Others
/// after, by first sighting. Decides the left column in Focus 2 (top = first). Returns positions in `windows`.
pub fn attention_order<W: Borrow<TermWindow>>(windows: &[W]) -> Vec<usize> {
    let keys: Vec<(bool, bool, Time, Time, u64)> = windows
        .iter()
        .map(|w| {
            let w = w.borrow();
            let s = w.shown_state();
            (
                s.waiting,
                s.ready,
                w.ready_since.unwrap_or(w.first_seen),
                w.first_seen,
                w.id,
            )
        })
        .collect();
    let mut order: Vec<usize> = (0..windows.len()).collect();
    order.sort_by(|&a, &b| {
        let (wa, ra, since_a, first_a, id_a) = keys[a];
        let (wb, rb, since_b, first_b, id_b) = keys[b];
        wb.cmp(&wa)
            .then(rb.cmp(&ra))
            .then_with(|| {
                if ra && rb {
                    cmp_f64(since_a, since_b)
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .then_with(|| cmp_f64(first_a, first_b))
            // Windows found in the same tick (all of them after a restart): never by how they came in.
            .then(id_a.cmp(&id_b))
    });
    order
}

/// `ids` (indices into `windows`) in attention order.
fn by_attention(ids: &[usize], windows: &[TermWindow]) -> Vec<usize> {
    let picked: Vec<&TermWindow> = ids.iter().map(|&i| &windows[i]).collect();
    attention_order(&picked)
        .into_iter()
        .map(|k| ids[k])
        .collect()
}

// ---------- Planning across all screens ----------

/// Screen a window lies on: the LARGEST overlap wins. A centre test falls short: if the centre lies in the Dock strip
/// (not part of `visibleFrame`) it hits no screen, and a fallback to 0 would pull the window from the second monitor
/// onto the main screen. Without any overlap the nearest centre decides.
pub fn screen_index(frame: Rect, screens: &[Rect]) -> usize {
    let mut best = 0;
    let mut best_area = 0.0;
    for (i, s) in screens.iter().enumerate() {
        let cut = s.intersection(frame);
        let area = if cut.is_null() {
            0.0
        } else {
            cut.width() * cut.height()
        };
        if area > best_area {
            best_area = area;
            best = i;
        }
    }
    if best_area > 0.0 {
        return best;
    }
    let mut best_dist = f64::MAX;
    for (i, s) in screens.iter().enumerate() {
        let (dx, dy) = (s.mid_x() - frame.mid_x(), s.mid_y() - frame.mid_y());
        let d = dx * dx + dy * dy;
        if d < best_dist {
            best_dist = d;
            best = i;
        }
    }
    best
}

// ---------- Target: where do the windows go? (Kevin 2026-09-18, replaces the waiting room) ----------
//
// The waiting room sent every session that was "done" between two answers to the MacBook, also the one Kevin was
// working in, and every status change switched the screen. Now Kevin decides by button: all on one screen, or control
// room.

/// Index of the screen with this id; empty: none. Equal monitors are called X, X#2, X#3 from the left
/// (`unique_screen_ids`); when one is unplugged, the ones to its right move up a number. If "X#3" is missing, the
/// highest remaining twin counts, not the left one (review quick 2026-10-01: Codex R1, GLM + Kimi R2).
/// ponytail: only the order tells equal monitors apart, macOS itself does not. Unplugging and replugging can move the
/// choice to the neighbour (X#2 chosen, X gone, X#3 there -> the right neighbour). Remedy if ever needed: store the
/// position of the chosen screen too.
pub fn screen_index_by_id(id: &str, screens: &[ScreenInfo]) -> Option<usize> {
    if id.is_empty() {
        return None;
    }
    if let Some(i) = screens.iter().position(|s| s.id == id) {
        return Some(i);
    }
    // Swift matches `#\d+$` and then needs `Int(...)`: only ASCII digits that fit an Int count.
    let (base, digits) = id.rsplit_once('#')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: i64 = digits.parse().ok()?;
    // No number can be higher than the number of screens; this also bounds a broken stored "X#99999999".
    let start = (n - 1).min(screens.len() as i64);
    (1..=start).rev().find_map(|k| {
        let wanted = if k == 1 {
            base.to_owned()
        } else {
            format!("{base}#{k}")
        };
        screens.iter().position(|s| s.id == wanted)
    })
}

/// Do two ids point at the same screen right now, also "X#2" and "X" while the twin is unplugged?
pub fn same_screen(a: &str, b: &str, screens: &[ScreenInfo]) -> bool {
    a == b
        || screen_index_by_id(a, screens).is_some_and(|i| Some(i) == screen_index_by_id(b, screens))
}

/// The control-room pair chosen in the menu as screen indices when it applies: from three screens on, both found (also
/// via the twin), different; otherwise `None` = automatic. Arrangement and menu check mark both read here (review quick
/// 2026-10-01).
pub fn active_pair(pair: Option<&Pair<String>>, screens: &[ScreenInfo]) -> Option<Pair<usize>> {
    let pair = pair?;
    if screens.len() < 3 {
        return None;
    }
    let work = screen_index_by_id(&pair.work, screens)?;
    let control = screen_index_by_id(&pair.control, screens)?;
    (work != control).then_some(Pair { work, control })
}

/// Index of the first largest area (Swift `max(by:)` keeps the first of equals).
fn first_max(indices: impl Iterator<Item = usize>, area: impl Fn(usize) -> f64) -> Option<usize> {
    indices.reduce(|best, i| if area(best) < area(i) { i } else { best })
}

/// Index of the first smallest area (Swift `min(by:)` keeps the first of equals).
fn first_min(indices: impl Iterator<Item = usize>, area: impl Fn(usize) -> f64) -> Option<usize> {
    indices.reduce(|best, i| if area(i) < area(best) { i } else { best })
}

fn rect_area(r: Rect) -> f64 {
    r.width() * r.height()
}

/// Choice + screens -> target. If the chosen screen is missing, the target falls back to the largest present one. The
/// control room needs two screens: work = largest, control = smallest; from three screens on the pair chosen in the
/// menu counts while both are attached (Kevin 2026-10-01, `active_pair`). `None` only without any screen.
pub fn resolve_target(
    choice: &TargetChoice,
    screens: &[ScreenInfo],
    pair: Option<&Pair<String>>,
) -> Option<Target> {
    let area = |i: usize| rect_area(screens[i].visible);
    let largest = first_max(0..screens.len(), area)?;
    let room = |work, control, ready_side, flipped| Target::ControlRoom {
        work,
        control,
        ready_side,
        flipped,
    };
    let ready_side = if *choice == TargetChoice::Both2 {
        StackSide::Right
    } else {
        StackSide::Left
    };
    Some(match choice {
        TargetChoice::External => Target::Single(
            first_max((0..screens.len()).filter(|&i| !screens[i].builtin), area).unwrap_or(largest),
        ),
        TargetChoice::Builtin => Target::Single(
            (0..screens.len())
                .find(|&i| screens[i].builtin)
                .unwrap_or(largest),
        ),
        TargetChoice::Screen(id) => {
            Target::Single(screen_index_by_id(id, screens).unwrap_or(largest))
        }
        TargetChoice::Both | TargetChoice::Both2 | TargetChoice::Both3 => {
            if let Some(p) = active_pair(pair, screens) {
                return Some(if *choice == TargetChoice::Both3 {
                    room(p.work, p.control, StackSide::Left, true)
                } else {
                    room(p.work, p.control, ready_side, false)
                });
            }
            let Some(smallest) = first_min((0..screens.len()).filter(|&i| i != largest), area)
            else {
                return Some(Target::Single(largest));
            };
            if *choice == TargetChoice::Both3 {
                // Work screen is the MacBook, as the button says; only without a built-in screen the smallest (review
                // quick).
                let work = (0..screens.len())
                    .find(|&i| screens[i].builtin && i != largest)
                    .unwrap_or(smallest);
                room(work, largest, StackSide::Left, true)
            } else {
                room(largest, smallest, ready_side, false)
            }
        }
    })
}

/// Entries under SCREEN: every screen on its own (external ones from left to right as on the desk, the built-in one
/// last), from two screens on the three control rooms.
pub fn target_options(screens: &[ScreenInfo]) -> Vec<TargetChoice> {
    let mut single: Vec<&ScreenInfo> = screens.iter().filter(|s| !s.id.is_empty()).collect();
    single.sort_by(|a, b| {
        a.builtin
            .cmp(&b.builtin)
            .then(cmp_f64(a.visible.min_x(), b.visible.min_x()))
            .then(cmp_f64(-a.visible.max_y(), -b.visible.max_y()))
    });
    let mut out: Vec<TargetChoice> = single
        .into_iter()
        .map(|s| TargetChoice::Screen(s.id.clone()))
        .collect();
    if screens.len() > 1 {
        out.extend([TargetChoice::Both, TargetChoice::Both2, TargetChoice::Both3]);
    }
    out
}

/// Positions of `indices` ordered left to right, top first (Swift's `place` tuple).
fn by_place(screens: &[ScreenInfo], mut indices: Vec<usize>) -> Vec<usize> {
    indices.sort_by(|&a, &b| {
        let (ra, rb) = (screens[a].visible, screens[b].visible);
        cmp_f64(ra.min_x(), rb.min_x()).then(cmp_f64(-ra.max_y(), -rb.max_y()))
    });
    indices
}

/// Two equal monitors without serial number can report the same id: from left to right the first keeps it, the others
/// get "#2", "#3"; otherwise both menu entries would point at the same screen. An empty id stays empty.
pub fn unique_screen_ids(screens: &[ScreenInfo]) -> Vec<ScreenInfo> {
    let mut out = screens.to_vec();
    let ids: BTreeSet<&str> = screens
        .iter()
        .map(|s| s.id.as_str())
        .filter(|id| !id.is_empty())
        .collect();
    for id in ids {
        let same = by_place(
            screens,
            (0..screens.len())
                .filter(|&i| screens[i].id == id)
                .collect(),
        );
        for (n, &i) in same.iter().enumerate().skip(1) {
            out[i].id = format!("{id}#{}", n + 1);
        }
    }
    out
}

/// Names in the menu: external ones as macOS calls them, the built-in one MacBook or iMac. Equal names get (1), (2)
/// from left to right.
pub fn screen_names(screens: &[ScreenInfo]) -> Vec<String> {
    let base: Vec<&str> = screens
        .iter()
        .map(|s| {
            if !s.builtin {
                s.name.as_str()
            } else if s.laptop {
                "MacBook"
            } else {
                "iMac"
            }
        })
        .collect();
    (0..base.len())
        .map(|i| {
            let same: Vec<usize> = (0..base.len()).filter(|&j| base[j] == base[i]).collect();
            if same.len() <= 1 {
                return base[i].to_owned();
            }
            match by_place(screens, same).iter().position(|&j| j == i) {
                Some(n) => format!("{} ({})", base[i], n + 1),
                None => base[i].to_owned(),
            }
        })
        .collect()
}

/// Menu title of a target with the screens' real names (until 2026-10-01 "MacBook" was fixed in it); `None` if it does
/// not work with the attached screens.
pub fn target_title(
    option: &TargetChoice,
    screens: &[ScreenInfo],
    pair: Option<&Pair<String>>,
) -> Option<String> {
    let names = screen_names(screens);
    let on = |i: usize| -> String {
        if !screens[i].builtin {
            return format!("{}{}", tr("auf ", "on "), names[i]);
        }
        let s = if screens[i].laptop {
            tr("aufs MacBook", "on the MacBook")
        } else {
            tr("auf den iMac", "on the iMac")
        };
        s.to_owned()
    };
    let several = screens.len() > 1;
    match option {
        TargetChoice::Screen(id) => screens
            .iter()
            .position(|s| s.id == *id)
            .map(|i| format!("{}{}", tr("Alle ", "All "), on(i))),
        TargetChoice::External | TargetChoice::Builtin => None,
        TargetChoice::Both => several.then(|| {
            tr(
                "Leitstand (fertige links)",
                "Control room (done on the left)",
            )
            .to_owned()
        }),
        TargetChoice::Both2 => several.then(|| {
            tr(
                "Leitstand 2 (fertige rechts)",
                "Control room 2 (done on the right)",
            )
            .to_owned()
        }),
        TargetChoice::Both3 => {
            if !several {
                return None;
            }
            let Some(Target::ControlRoom { work, .. }) =
                resolve_target(&TargetChoice::Both3, screens, pair)
            else {
                return None;
            };
            Some(format!(
                "{}{})",
                tr("Leitstand 3 (aktiv ", "Control room 3 (active "),
                on(work)
            ))
        }
    }
}

/// Choose the control-room screens in the menu (from three screens on, Kevin 2026-10-01): the same screen for both
/// roles swaps them. Screens are compared, not names, and the previous ids stay: "X#2" (right one of two equal ones,
/// the left one unplugged right now) would otherwise become "X" and after replugging mean the left one (review quick
/// 2026-10-01 round 2, GLM).
pub fn choose_pair(
    current: &Pair<String>,
    work: Option<&str>,
    control: Option<&str>,
    screens: &[ScreenInfo],
) -> Pair<String> {
    let same = |a: &str, b: &str| same_screen(a, b, screens);
    let mut p = current.clone();
    if let Some(work) = work.filter(|&w| !same(w, &p.work)) {
        if same(work, &p.control) {
            p = Pair {
                work: p.control,
                control: p.work,
            };
        } else {
            p.work = work.to_owned();
        }
    }
    if let Some(control) = control.filter(|&c| !same(c, &p.control)) {
        if same(control, &p.work) {
            p = Pair {
                work: p.control,
                control: p.work,
            };
        } else {
            p.control = control.to_owned();
        }
    }
    p
}

/// Did Kevin take this window out of the arrangement? Remembered per session id (Kevin 2026-09-18); a window with
/// several sessions is out as soon as one of them is.
pub fn is_excluded(w: &TermWindow, excluded: &BTreeSet<String>) -> bool {
    w.sessions.iter().any(|s| excluded.contains(&s.id))
}

/// Worker window (parking lot, Kevin 2026-11-06): the name of its VISIBLE session (status-file name or own name from
/// the transcript) contains a `worker_names` entry, case-insensitive. A session named exactly like a `lead_names` entry
/// is never a worker; no visible session, no worker.
pub fn is_worker(w: &TermWindow, worker_names: &[String], lead_names: &[String]) -> bool {
    let Some(s) = w.visible_session() else {
        return false;
    };
    let names: Vec<&str> = [
        Some(s.name.as_str()),
        s.digest.as_ref().and_then(|d| d.name.as_deref()),
    ]
    .into_iter()
    .flatten()
    .collect();
    if names.iter().any(|n| lead_names.iter().any(|l| l == n)) {
        return false;
    }
    let wanted: Vec<String> = worker_names
        .iter()
        .filter(|n| !n.is_empty())
        .map(|n| fold_case(n))
        .collect();
    names.iter().any(|n| {
        let n = fold_case(n);
        wanted.iter().any(|w| n.contains(w.as_str()))
    })
}

/// Is a window meant for the parking lot (the check mark in the menu)? A worker by name unless taken off by hand, or
/// sent there by hand (Kevin 2026-11-06 evening).
pub fn lot_member(
    w: &TermWindow,
    worker_names: &[String],
    lead_names: &[String],
    hand_parked: &BTreeSet<String>,
    hand_kept: &BTreeSet<String>,
) -> bool {
    let has = |set: &BTreeSet<String>| w.sessions.iter().any(|s| set.contains(&s.id));
    (is_worker(w, worker_names, lead_names) && !has(hand_kept)) || has(hand_parked)
}

/// Does a lot member park right now? Only while it runs or watches: done, asking or with a question in a tab behind
/// it, it stands with the waiting ones.
fn on_lot(w: &TermWindow, p: &PlanInput) -> bool {
    lot_member(w, p.worker_names, p.lead_names, p.hand_parked, p.hand_kept)
        && !w.is_ready()
        && !w.hidden_question()
}

/// Windows Lookout arranges: with an AI session, movable and not excluded, stable by first sighting. Plain terminals,
/// minimised, full-screen and excluded windows stay untouched.
pub fn layout_candidates(windows: &[TermWindow], excluded: &BTreeSet<String>) -> Vec<usize> {
    let mut out: Vec<usize> = (0..windows.len())
        .filter(|&i| {
            !windows[i].sessions.is_empty()
                && windows[i].layoutable()
                && !is_excluded(&windows[i], excluded)
        })
        .collect();
    out.sort_by(|&a, &b| {
        cmp_f64(windows[a].first_seen, windows[b].first_seen)
            .then(windows[a].id.cmp(&windows[b].id))
    });
    out
}

/// Order of the non-main windows in Focus and control room: first the remembered order (`hint`, only windows that
/// still exist), then newcomers by first sighting.
pub fn rest_sequence(ordered: &[usize], main: usize, hint: &[usize]) -> Vec<usize> {
    let known: Vec<usize> = hint
        .iter()
        .copied()
        .filter(|&i| i != main && ordered.contains(&i))
        .collect();
    let new = ordered
        .iter()
        .copied()
        .filter(|&i| i != main && !known.contains(&i));
    known.iter().copied().chain(new).collect()
}

/// Click on a side window or tile: the new main window SWAPS places with the old one; only these two move (preview
/// page 2026-09-18). Before, a click re-sorted all side windows by first sighting.
pub fn swapped_rest(rest: &[usize], old_main: usize, new_main: usize) -> Vec<usize> {
    let mut out = rest.to_vec();
    if let Some(i) = rest.iter().position(|&x| x == new_main) {
        out[i] = old_main;
    }
    out
}

// ---------- Status decides the order (plan 1d, Kevin 2026-09-26) ----------
//
// One rule for Focus 1 (GP 5, R2-3), three separate groups: question · done without question · running. In question
// and done only the time of being done decides the place (`attention_order`), nothing is swapped there; among the
// running the click order counts. Focus 2 and control room keep their order.

/// Focus 1: question, then done (by time of being done), then the running in click order (`rest`).
pub fn status_sequence(rest: &[usize], windows: &[TermWindow]) -> Vec<usize> {
    let waiting: Vec<usize> = rest
        .iter()
        .copied()
        .filter(|&i| windows[i].is_ready())
        .collect();
    let mut out = by_attention(&waiting, windows);
    out.extend(rest.iter().copied().filter(|&i| !windows[i].is_ready()));
    out
}

/// Click in Focus 1: if old main and clicked window both run, they swap places (`swapped_rest`), nothing else moves.
/// Across a group border the old main goes to its group's place; if it runs, to the start of the running ones
/// (suggested by the cross-check, to be checked in practice).
pub fn status_swap(
    rest: &[usize],
    old_main: usize,
    new_main: usize,
    windows: &[TermWindow],
) -> Vec<usize> {
    let old_runs = !windows[old_main].is_ready();
    if old_runs && !windows[new_main].is_ready() {
        return swapped_rest(rest, old_main, new_main);
    }
    let others = rest.iter().copied().filter(|&i| i != new_main);
    if old_runs {
        std::iter::once(old_main).chain(others).collect()
    } else {
        others.chain([old_main]).collect()
    }
}

/// Three columns for Focus 1 where two strips do not fit beside the reading width (MacBook): both sides `MIN_CARD_W`
/// (Warp's minimum width), the middle gets the rest (MacBook: 768 of 1728 pt). `None` below 3 × 480 pt: the middle
/// would be narrower than Warp allows; there it stays one edge. Returns `(left, main, right)`.
pub fn three_column_layout(vis: Rect) -> Option<(Rect, Rect, Rect)> {
    let fits = vis.width() >= 3.0 * MIN_CARD_W;
    if !fits {
        return None;
    }
    let left = Rect::new(vis.min_x(), vis.min_y(), MIN_CARD_W, vis.height());
    let right = Rect::new(
        vis.max_x() - MIN_CARD_W,
        vis.min_y(),
        MIN_CARD_W,
        vis.height(),
    );
    let main = Rect::new(
        left.max_x(),
        vis.min_y(),
        right.min_x() - left.max_x(),
        vis.height(),
    );
    Some((left, main, right))
}

// ---------- Overflow: window or row (plan 1d) ----------
//
// If not all fit on a side as windows, the others stand as rows at the bottom of the same side; their windows lie on
// the main window's frame, behind it ("parked"). Before, they stayed where they were and covered waiting ones.

/// How many windows and rows fit on a side of height `height`? `flexible` (in order) may be windows, `list_only`
/// always stand as a row (Focus 2: WORKING). Windows = the largest number at which they (at least `MIN_CARD_H` each)
/// and all rows fit; MacBook, 8 running: 4 × 192 + 28 + 4 × 46 = 980 ≤ 1005. If not even all rows fit, it is cut from
/// the back, `list_only` first (waiting ones go first, R2-2), and the last row says "+N more (in the menu)". Plan and
/// list use the same arithmetic.
pub fn rail_split(height: f64, flexible: usize, list_only: usize) -> RailSplit {
    for cards in (0..=flexible).rev() {
        let rows = flexible - cards + list_only;
        let list = if rows > 0 {
            LIST_HEADER_H + rows as f64 * LIST_ROW_H
        } else {
            0.0
        };
        if cards as f64 * MIN_CARD_H + list <= height {
            return RailSplit {
                cards,
                rows,
                more: 0,
            };
        }
    }
    let fit = count_at_least_one((height - LIST_HEADER_H) / LIST_ROW_H); // rows including "+N more"
    RailSplit {
        cards: 0,
        rows: fit - 1,
        more: (flexible + list_only).saturating_sub(fit - 1),
    }
}

/// Fill a side: windows one below the other at the top (`strip_stack`), the list below. Header "+N more" (Focus 1) or
/// "WORKING (N)" (Focus 2, extra waiting ones right above it). The windows of the rows lie on `main_rect`.
fn fill_side(
    rail: Rect,
    flexible: &[usize],
    list_only: &[usize],
    main_rect: Rect,
    out: &mut Arrangement,
) {
    let split = rail_split(rail.height(), flexible.len(), list_only.len());
    let listed: Vec<usize> = flexible[split.cards..]
        .iter()
        .chain(list_only)
        .copied()
        .collect();
    let list_h = if listed.is_empty() {
        0.0
    } else {
        LIST_HEADER_H + (split.rows + usize::from(split.more > 0)) as f64 * LIST_ROW_H
    };
    let cards = Rect::new(
        rail.min_x(),
        rail.min_y() + list_h,
        rail.width(),
        rail.height() - list_h,
    );
    out.frames
        .extend(strip_stack(cards, &flexible[..split.cards]));
    if listed.is_empty() {
        return;
    }
    for &i in &listed {
        out.frames.insert(i, main_rect);
        out.parked.insert(i);
    }
    let shown = &listed[..split.rows.min(listed.len())];
    let mut entries: Vec<ListEntry> = vec![];
    if list_only.is_empty() {
        entries.push(ListEntry::Header(format!(
            "+{} {}",
            listed.len(),
            tr("weitere", "more")
        )));
        entries.extend(shown.iter().map(|&i| ListEntry::Row(i)));
    } else {
        entries.extend(
            shown
                .iter()
                .filter(|i| !list_only.contains(i))
                .map(|&i| ListEntry::Row(i)),
        );
        entries.push(ListEntry::Header(format!(
            "{} ({})",
            LabelTone::Running.word(),
            list_only.len()
        )));
        entries.extend(
            shown
                .iter()
                .filter(|i| list_only.contains(i))
                .map(|&i| ListEntry::Row(i)),
        );
    }
    if split.more > 0 {
        entries.push(ListEntry::More(split.more));
    }
    out.lists.push(SideList {
        rect: Rect::new(rail.min_x(), rail.min_y(), rail.width(), list_h),
        entries,
        inbox: false,
    });
}

// ---------- Grid: fixed choice (Kevin 2026-09-18) ----------

/// The grids in the menu, in Kevin's order.
pub const GRID_CHOICES: [GridSpec; 6] = [
    GridSpec::new(2, 2),
    GridSpec::new(3, 2),
    GridSpec::new(4, 2),
    GridSpec::new(3, 3),
    GridSpec::new(4, 3),
    GridSpec::new(5, 2),
];
/// As long as Kevin has not chosen one: fits both screens.
pub const DEFAULT_GRID: GridSpec = GridSpec::new(3, 2);

/// How many columns/rows fit without a tile falling below Warp's minimum size? Below it the window would stand larger
/// than planned and overlap its neighbours.
fn grid_limits(vis: Rect) -> (usize, usize) {
    (
        count_at_least_one(vis.width() / MIN_CARD_W),
        count_at_least_one(vis.height() / MIN_CARD_H),
    )
}

/// Only what fits the screen is offered (MacBook: at most 3 columns).
pub fn grid_choices(vis: Rect) -> Vec<GridSpec> {
    let (cols, rows) = grid_limits(vis);
    GRID_CHOICES
        .into_iter()
        .filter(|g| g.cols <= cols && g.rows <= rows)
        .collect()
}

/// Trim the stored choice to the target screen: 5×2 from the LG becomes 3×2 on the MacBook.
pub fn fitted_grid(g: GridSpec, vis: Rect) -> GridSpec {
    let (cols, rows) = grid_limits(vis);
    GridSpec::new(g.cols.min(cols), g.rows.min(rows))
}

/// Fixed grid (Kevin 2026-09-18: "Fest"): every tile is one place big, however many windows there are; free places
/// stay free. Slot 0 top-left, then row by row. More windows than places: the others get no place and stay untouched.
pub fn fixed_grid(vis: Rect, g: GridSpec, ids: &[usize]) -> BTreeMap<usize, Rect> {
    let cell_w = (vis.width() / g.cols as f64).floor();
    let cell_h = (vis.height() / g.rows as f64).floor();
    let cell = |i: usize| {
        Rect::new(
            vis.min_x() + (i % g.cols) as f64 * cell_w,
            vis.max_y() - (i / g.cols + 1) as f64 * cell_h,
            cell_w,
            cell_h,
        )
    };
    ids.iter()
        .take(g.cols.saturating_mul(g.rows))
        .enumerate()
        .map(|(i, &id)| (id, cell(i)))
        .collect()
}

/// Control-room tiles: columns from the screen width, the extra ones into the mini stack.
fn grid_arrange(vis: Rect, side: StackSide, ids: &[usize]) -> BTreeMap<usize, Rect> {
    let (cols, rows) = auto_grid(vis, ids.len());
    let grid = grid_layout(vis, cols, rows, side, ids);
    let mut out = grid.frames;
    out.extend(mini_stack(vis, side, &grid.overflow));
    out
}

/// Focus 1 on one screen: `main` centred, the rest by status (`status_sequence`). If two strips fit beside the reading
/// width (LG), the order fills the left side up to half, then the right one. Otherwise from 1440 pt three columns
/// (plan 1d): left whoever waits, right what runs, both as real windows. Below that one edge as before.
fn focus_arrange(
    vis: Rect,
    side: StackSide,
    main: usize,
    rest: &[usize],
    windows: &[TermWindow],
) -> Arrangement {
    let (main_rect, left, right) = reading_layout(vis, side);
    if (left.is_none() || right.is_none())
        && let Some((left, middle, right)) = three_column_layout(vis)
    {
        let mut out = Arrangement {
            frames: BTreeMap::from([(main, middle)]),
            main: Some(main),
            ..Default::default()
        };
        let (ready, running): (Vec<usize>, Vec<usize>) =
            rest.iter().partition(|&&i| windows[i].is_ready());
        fill_side(left, &ready, &[], middle, &mut out);
        fill_side(right, &running, &[], middle, &mut out);
        return out;
    }
    let mut out = Arrangement {
        frames: BTreeMap::from([(main, main_rect)]),
        main: Some(main),
        ..Default::default()
    };
    let rails: Vec<Rect> = [left, right].into_iter().flatten().collect();
    let half = if rails.len() == 2 {
        rest.len().div_ceil(2)
    } else {
        rest.len()
    };
    for (rail, part) in rails.into_iter().zip([&rest[..half], &rest[half..]]) {
        fill_side(rail, part, &[], main_rect, &mut out);
    }
    out
}

/// Focus 2 (Kevin 2026-09-18): `main` centred at reading width, LEFT whoever waits for Kevin (`attention_order`), RIGHT
/// what runs. A session that gets done lands left on the next tick, and right again once it works. If only one strip
/// fits beside the main window (MacBook), since plan 1d (2026-09-26) only the waiting ones stand there as windows; the
/// running lie behind the middle and stand in the WORKING list.
fn focus2_arrange(
    vis: Rect,
    side: StackSide,
    main: usize,
    others: &[usize],
    windows: &[TermWindow],
) -> Arrangement {
    let (main_rect, left, right) = reading_layout(vis, side);
    let (ready, running): (Vec<usize>, Vec<usize>) =
        others.iter().partition(|&&i| windows[i].is_ready());
    let waiting = by_attention(&ready, windows);
    let mut out = Arrangement {
        frames: BTreeMap::from([(main, main_rect)]),
        main: Some(main),
        ..Default::default()
    };
    match (left, right) {
        (Some(left), Some(right)) => {
            fill_side(left, &waiting, &[], main_rect, &mut out);
            fill_side(right, &running, &[], main_rect, &mut out);
        }
        (Some(rail), None) | (None, Some(rail)) => {
            fill_side(rail, &waiting, &running, main_rect, &mut out)
        }
        (None, None) => {}
    }
    out
}

/// Width of the inbox window (Kevin 2026-11-09, preview "Eingang").
pub const INBOX_W: f64 = 430.0;
const INBOX_GAP: f64 = 8.0;

/// Inbox: the list window with the terminal right beside it, the pair centred; what is left of a wide screen stays free.
/// The terminal keeps the reading width while it fits, and the list gives way before the terminal falls below Warp's
/// minimum. Returns `(panel, main)`.
pub fn inbox_layout(vis: Rect) -> (Rect, Rect) {
    let panel_w = INBOX_W
        .min(vis.width() - INBOX_GAP - MIN_CARD_W)
        .max(0.0)
        .floor();
    let main_w = READING_WIDTH.min(vis.width() - panel_w - INBOX_GAP);
    let x = vis.min_x() + ((vis.width() - panel_w - INBOX_GAP - main_w) / 2.0).floor();
    (
        Rect::new(x, vis.min_y(), panel_w, vis.height()),
        Rect::new(x + panel_w + INBOX_GAP, vis.min_y(), main_w, vis.height()),
    )
}

/// Inbox (Kevin 2026-11-09: "die Anzeigen unnötig"): one window with tabs, whoever wants something from Kevin
/// (`attention_order`) and what works (by first sighting, then the parking lot); the active terminal beside
/// it, every other window behind that terminal. The terminal in front keeps its card: that card is the open one.
fn inbox_arrange(
    vis: Rect,
    main: usize,
    others: &[usize],
    windows: &[TermWindow],
    lot: &[usize],
) -> Arrangement {
    let (panel, main_rect) = inbox_layout(vis);
    let mut all: Vec<usize> = others.to_vec();
    all.push(main);
    all.sort_by(|&a, &b| {
        cmp_f64(windows[a].first_seen, windows[b].first_seen)
            .then(windows[a].id.cmp(&windows[b].id))
    });
    let (ready, working): (Vec<usize>, Vec<usize>) =
        all.into_iter().partition(|&i| windows[i].is_ready());
    let entries: Vec<ListEntry> = by_attention(&ready, windows)
        .into_iter()
        .map(ListEntry::Row)
        .chain(working.into_iter().map(ListEntry::Working))
        .chain(lot.iter().map(|&i| ListEntry::Lot(i)))
        .collect();
    let mut frames = BTreeMap::from([(main, main_rect)]);
    frames.extend(others.iter().map(|&i| (i, main_rect)));
    Arrangement {
        frames,
        parked: others.iter().copied().collect(),
        lists: vec![SideList {
            rect: panel,
            entries,
            inbox: true,
        }],
        main: Some(main),
        ..Default::default()
    }
}

/// First session id of a window -> where it stood when Kevin marked it read in the inbox: since when it is done, and its
/// latest answer. Both, because only Claude sessions have an answer time, and a quick new answer need not be seen as
/// "running" in between. Kept with the settings over a restart (Kevin 2026-11-12, `read_answers_lines`).
/// ponytail: never thinned out, like the parked ones; drop the stamps of sessions long gone if the settings grow.
pub type ReadAnswers = BTreeMap<String, (Option<Time>, Option<Time>)>;

/// The stamps as lines for the settings (Kevin 2026-11-12: kept over a restart): "session id<TAB>done since<TAB>latest
/// answer" each, an unknown time left empty. Rust prints a time so that it reads back exactly.
pub fn read_answers_lines(read: &ReadAnswers) -> Vec<String> {
    let time = |t: &Option<Time>| t.map(|t| t.to_string()).unwrap_or_default();
    read.iter()
        .map(|(id, (since, answer))| format!("{id}\t{}\t{}", time(since), time(answer)))
        .collect()
}

/// The stamps back from `read_answers_lines`, the times read from the right (an id may hold a tab). A line that cannot
/// be read is dropped: its card shows again, nothing is hidden by mistake.
pub fn read_answers_from(lines: &[String]) -> ReadAnswers {
    let time = |s: &str| -> Option<Option<Time>> {
        if s.is_empty() {
            Some(None)
        } else {
            s.parse().ok().map(Some)
        }
    };
    lines
        .iter()
        .filter_map(|l| {
            let mut parts = l.rsplitn(3, '\t');
            let answer = time(parts.next()?)?;
            let since = time(parts.next()?)?;
            Some((parts.next()?.to_owned(), (since, answer)))
        })
        .collect()
}

/// The loaded stamps as a restarted Lookout sees them: it meets every done window anew, done since its latest answer
/// (`first_ready_time`), so a stamp taken while the window stood done since earlier (a quick new answer, or an answer
/// time that came late) takes that time too (review quick 2026-11-12, Grok, GLM). Without an answer time (Codex, Grok)
/// nothing tells the same done from a new one: such a card shows again after a restart.
pub fn stamps_after_restart(mut read: ReadAnswers) -> ReadAnswers {
    for (since, answer) in read.values_mut() {
        if since.is_some() && answer.is_some() {
            *since = *answer;
        }
    }
    read
}

fn last_answer(w: &TermWindow) -> Option<Time> {
    w.sessions
        .iter()
        .filter_map(|s| s.digest.as_ref()?.answer_at)
        .max_by(|a, b| cmp_f64(*a, *b))
}

/// Since when a window that is newly seen as done has been waiting: its last answer if the transcript knows it. Else a
/// restart would put every window at "now", and the longest waiting would no longer stand on top (Kevin 2026-11-09).
pub fn first_ready_time(w: &TermWindow, now: Time) -> Time {
    last_answer(w).filter(|&at| at <= now).unwrap_or(now)
}

fn read_stamp(w: &TermWindow) -> (Option<Time>, Option<Time>) {
    (w.ready_since, last_answer(w))
}

pub fn mark_read(w: &TermWindow, read: &mut ReadAnswers) {
    if let Some(s) = w.sessions.first() {
        read.insert(s.id.clone(), read_stamp(w));
    }
}

fn stamped(w: &TermWindow, read: &ReadAnswers) -> bool {
    w.sessions
        .first()
        .is_some_and(|s| read.get(&s.id) == Some(&read_stamp(w)))
}

/// A done window Kevin marked read, until it is done anew. A question is never read: it stays until answered.
pub fn is_read(w: &TermWindow, read: &ReadAnswers) -> bool {
    !w.is_waiting() && stamped(w, read)
}

/// Read in its project's or team's area (Kevin 2026-11-12), a question too: until it does something new (a new answer,
/// work again), it stands under "Gelesen" and leaves the tile quiet.
pub fn marked_read(w: &TermWindow, read: &ReadAnswers) -> bool {
    can_mark_read(w) && stamped(w, read)
}

/// Whether "Gelesen" can hold the window: a question without an answer time (Codex, Grok) could not be told from a
/// later one, so it is never read, as it is never parked (`is_snoozed`).
pub fn can_mark_read(w: &TermWindow) -> bool {
    !w.is_waiting() || last_answer(w).is_some()
}

/// Parked by Kevin (Kevin 2026-11-09: a session that watches a mailbox all day belongs neither under waiting nor under
/// running). Whatever it is doing it stays out of the way, until it asks something new: the question he parked stays
/// parked (`press_button` stamps it), a later one comes through. Without an answer time (Codex, Grok) a later question
/// could not be told from the parked one, so a question there stays under Waiting. The stamp outlives a restart with
/// `ReadAnswers`.
pub fn is_snoozed(w: &TermWindow, snoozed: &BTreeSet<String>, read: &ReadAnswers) -> bool {
    is_excluded(w, snoozed) && (!w.is_waiting() || (last_answer(w).is_some() && stamped(w, read)))
}

/// Pinned by Kevin (Kevin 2026-11-09): the card stands on top of Waiting, whatever its session does,
/// until he unpins it. `pinned` keeps the order he pinned in, the first one on top: a new pin goes below, so no pinned
/// card ever moves (no sorting by hand, Kevin 2026-11-09). Returns where the window stands among the pins.
pub fn pin_rank(w: &TermWindow, pinned: &[String]) -> Option<usize> {
    w.sessions
        .iter()
        .filter_map(|s| pinned.iter().position(|p| *p == s.id))
        .min()
}

/// Pins every session of `w`, after the pins there are, or unpins them. Both end a park and a read mark like "Bring
/// back": a card Kevin pins is one he wants to see, and once he unpins it, it goes back to where its state puts it, not
/// to an old park (review quick 2026-11-09: a parked session that asked again, pinned, then worked again dropped out of
/// Running). A mark on a tab that joins the window while it is pinned counts for nothing then (`inbox_target`).
pub fn set_pinned(
    w: &TermWindow,
    pinned: &mut Vec<String>,
    snoozed: &mut BTreeSet<String>,
    read: &mut ReadAnswers,
    on: bool,
) {
    pinned.retain(|p| !w.sessions.iter().any(|s| s.id == *p));
    if on {
        pinned.extend(w.sessions.iter().map(|s| s.id.clone()));
    }
    unpark(w, snoozed, read);
}

/// Every tab's marks go, not only the first one's: a read mark stays under the tab that stood first when it was set,
/// and comes back once the tabs before it close (review quick 2026-11-09, round 2).
fn unpark(w: &TermWindow, snoozed: &mut BTreeSet<String>, read: &mut ReadAnswers) {
    for s in &w.sessions {
        snoozed.remove(&s.id);
        read.remove(&s.id);
    }
}

/// Whom "read, next" passes over (`next_waiting`'s `excluded`): what Kevin excluded, and the pinned cards. Those stand on
/// top outside the queue below, and a pinned card offers no "read" to walk on with (review quick 2026-11-09, round 3).
pub fn walk_skips(excluded: &BTreeSet<String>, pinned: &[String]) -> BTreeSet<String> {
    excluded.iter().chain(pinned).cloned().collect()
}

/// Who comes to the middle after "read, next": the next in the waiting list after `main`, from the end back to the top;
/// the top one if `main` does not wait. Stepping past `main` is what walks the list: "the most urgent other one" would
/// only swap the top two, as a session Kevin looked at without answering stays where it was. `None`: nobody else waits.
/// What Kevin read is passed over, a question read in its area too (review quick 2026-11-12, GLM).
pub fn next_waiting(
    windows: &[TermWindow],
    excluded: &BTreeSet<String>,
    snoozed: &BTreeSet<String>,
    main: Option<usize>,
    read: &ReadAnswers,
) -> Option<usize> {
    let ready: Vec<usize> = layout_candidates(windows, excluded)
        .into_iter()
        .filter(|&i| {
            windows[i].is_ready()
                && !marked_read(&windows[i], read)
                && !is_snoozed(&windows[i], snoozed, read)
        })
        .collect();
    let order = by_attention(&ready, windows);
    match main.and_then(|m| order.iter().position(|&i| i == m)) {
        Some(p) => Some(order[(p + 1) % order.len()]).filter(|&i| Some(i) != main),
        None => order.first().copied(),
    }
}

/// Parks every window in `members` the way "Park" parks one card, or brings each back like "Bring back" (Kevin
/// 2026-11-12: "ganze projekte parken"): a question there now stays parked, a later one comes through (`is_snoozed`).
/// Moves nothing: no window comes to the middle.
pub fn park_all(
    windows: &[TermWindow],
    members: &[usize],
    on: bool,
    read: &mut ReadAnswers,
    snoozed: &mut BTreeSet<String>,
) {
    for &i in members {
        let w = &windows[i];
        if !on {
            unpark(w, snoozed, read);
            continue;
        }
        if w.is_waiting() {
            mark_read(w, read);
        }
        snoozed.extend(w.sessions.iter().map(|s| s.id.clone()));
    }
}

/// A command on the card of window `w`; returns who comes to the middle next, only if `w` was in the middle: a right-click
/// on another card moves nothing. Who comes next is decided while `w` still stands in the list, then it leaves: the other
/// way round every click would start again at the top of the list instead of going down it (review quick 2026-11-09).
/// Bringing back moves nothing. Pin and Unpin are not for here: they go to `set_pinned`.
pub fn press_button(
    button: CardButton,
    windows: &[TermWindow],
    excluded: &BTreeSet<String>,
    main: Option<usize>,
    w: usize,
    read: &mut ReadAnswers,
    snoozed: &mut BTreeSet<String>,
) -> Option<usize> {
    let window = &windows[w];
    if button == CardButton::Unpark {
        unpark(window, snoozed, read);
        return None;
    }
    if button == CardButton::MarkRead {
        // An old park ends first: the fresh stamp would hold the card parked again (review quick 2026-11-12, Codex).
        unpark(window, snoozed, read);
        mark_read(window, read);
        return None;
    }
    let next =
        next_waiting(windows, excluded, snoozed, main, read).filter(|&i| i != w && main == Some(w));
    if button == CardButton::ReadNext || window.is_waiting() {
        mark_read(window, read);
    }
    if button == CardButton::Park {
        snoozed.extend(window.sessions.iter().map(|s| s.id.clone()));
    }
    next
}

/// Control room (Kevin 2026-09-24: "fertig sollte immer neben aktiv sein"): big screen, middle the active session,
/// right beside it on `side` whoever waits for Kevin (`attention_order`); the other side stays free for Chrome. The
/// MacBook shows what runs; waiting ones that do not fit the column queue up on the same side of the MacBook. Follows
/// the status every tick like Focus 2. The stack side does not apply here.
fn ready_beside_arrange(
    work: Rect,
    control: Rect,
    side: StackSide,
    main: usize,
    rest: &[usize],
    windows: &[TermWindow],
) -> BTreeMap<usize, Rect> {
    let (main_rect, left, right) = reading_layout(work, side);
    let (ready, running): (Vec<usize>, Vec<usize>) =
        rest.iter().partition(|&&i| windows[i].is_ready());
    let waiting = by_attention(&ready, windows);
    let column = if side == StackSide::Left { left } else { right }
        .expect("reading_layout always returns the column on `side`");
    let fits = count_at_least_one(column.height() / MIN_CARD_H).min(waiting.len());
    let rail_fits = count_at_least_one(control.height() / MIN_CARD_H);
    let spill = &waiting[fits..];
    let rail = &spill[..rail_fits.min(spill.len())];

    let mut out = BTreeMap::from([(main, main_rect)]);
    out.extend(strip_stack(column, &waiting[..fits]));
    out.extend(mini_stack(control, side, rail));
    // If the MacBook column is full too, the remaining waiting ones stand first in the grid; left lying they overlapped
    // where another window goes meanwhile (review quick).
    let area = if rail.is_empty() {
        control
    } else {
        main_area(control, side, mini_rail_width(control))
    };
    let grid_ids: Vec<usize> = spill[rail.len()..]
        .iter()
        .chain(&running)
        .copied()
        .collect();
    for (k, r) in grid_arrange(area, side, &grid_ids) {
        out.entry(k).or_insert(r);
    }
    out
}

/// Control room 3 (Kevin 2026-09-28: "in die andere Richtung"): MacBook = work screen, active session right, whoever
/// waits for Kevin LEFT beside it (`attention_order`, column `mini_rail_width`). The LG shows what runs in one column
/// each at its left and right edge (a quarter of the width), the middle (half the screen) stays free for Chrome. Waiting
/// ones that do not fit the MacBook column stand first in the LG columns. If not all fit there either, the rest gets
/// the middle as a grid: better cover Chrome than leave windows lying on top of each other (review quick 2026-09-24).
fn flipped_arrange(
    work: Rect,
    control: Rect,
    main: usize,
    rest: &[usize],
    windows: &[TermWindow],
) -> BTreeMap<usize, Rect> {
    let (main_rect, column) = focus_layout(work, StackSide::Left);
    let (ready, running): (Vec<usize>, Vec<usize>) =
        rest.iter().partition(|&&i| windows[i].is_ready());
    let waiting = by_attention(&ready, windows);
    let fits = count_at_least_one(column.height() / MIN_CARD_H).min(waiting.len());
    let others: Vec<usize> = waiting[fits..].iter().chain(&running).copied().collect();

    let mut out = BTreeMap::from([(main, main_rect)]);
    out.extend(strip_stack(column, &waiting[..fits]));

    let side_w = MIN_CARD_W.max((control.width() / 4.0).floor()); // Warp allows nothing narrower (review quick)
    let left = Rect::new(control.min_x(), control.min_y(), side_w, control.height());
    let right = Rect::new(
        control.max_x() - side_w,
        control.min_y(),
        side_w,
        control.height(),
    );
    let per_side = count_at_least_one(control.height() / MIN_CARD_H);
    let left_count = per_side.min(others.len().div_ceil(2));
    let right_count = per_side.min(others.len() - left_count);
    out.extend(strip_stack(left, &others[..left_count]));
    out.extend(strip_stack(
        right,
        &others[left_count..left_count + right_count],
    ));
    let middle = Rect::new(
        left.max_x(),
        control.min_y(),
        right.min_x() - left.max_x(),
        control.height(),
    );
    for (k, r) in grid_arrange(middle, StackSide::Left, &others[left_count + right_count..]) {
        out.entry(k).or_insert(r);
    }
    out
}

static NO_EXCLUSIONS: BTreeSet<String> = BTreeSet::new();

/// Everything a plan depends on. `PlanInput::new` sets Swift's defaults: no focused window, no remembered order,
/// `DEFAULT_GRID`, nothing excluded, no worker or lead names, no peeking.
#[derive(Clone, Copy, Debug)]
pub struct PlanInput<'a> {
    pub mode: LayoutMode,
    pub target: Target,
    pub windows: &'a [TermWindow],
    pub screens: &'a [Rect],
    pub side: StackSide,
    /// The active session (Swift `focusedIndex`).
    pub focused: Option<usize>,
    /// Remembered click order (Focus 1 `status_swap`, control room `swapped_rest`).
    pub rest_hint: &'a [usize],
    /// Kevin's grid choice, trimmed to the target screen.
    pub grid: GridSpec,
    pub excluded: &'a BTreeSet<String>,
    /// Parking lot (Kevin 2026-11-06): name parts of worker sessions (`is_worker`).
    pub worker_names: &'a [String],
    pub lead_names: &'a [String],
    /// Session ids Kevin sent to the parking lot by hand (Ctrl+Opt+5 or the menu, Kevin 2026-11-06 evening): such a
    /// window parks like a worker, whatever its name, a lead too.
    pub hand_parked: &'a BTreeSet<String>,
    /// Session ids Kevin took off the parking lot by hand: a worker whose session is here never parks.
    pub hand_kept: &'a BTreeSet<String>,
    /// Show the parking lot: tiled on the largest screen instead of parked behind the main window.
    pub peek: bool,
}

impl<'a> PlanInput<'a> {
    pub fn new(
        mode: LayoutMode,
        target: Target,
        windows: &'a [TermWindow],
        screens: &'a [Rect],
        side: StackSide,
    ) -> PlanInput<'a> {
        PlanInput {
            mode,
            target,
            windows,
            screens,
            side,
            focused: None,
            rest_hint: &[],
            grid: DEFAULT_GRID,
            excluded: &NO_EXCLUSIONS,
            worker_names: &[],
            lead_names: &[],
            hand_parked: &NO_EXCLUSIONS,
            hand_kept: &NO_EXCLUSIONS,
            peek: false,
        }
    }
}

/// Target frames for arrangement + target, with parked windows and lists (plan 1d). Only windows with an AI session that
/// can move and are not excluded get a place; all others stay where they are. Order stable by first sighting; the
/// active session (`focused`) stands in the middle in Focus and Focus 2 and alone on the work screen in the control
/// room. The arrangement only applies to `Single`; the control room is fixed.
///
/// Parking lot (Kevin 2026-11-06): worker windows that run or watch (`is_worker` or sent by hand, `hand_parked`; not
/// done, no hidden question; never the main window) stay out of the arrangement and lie behind the main window without a list row, in `parked` and
/// `lot`. With `peek` they are tiled on the largest screen instead (only `lot`); whoever gets no tile stays parked. A
/// single-screen grid has no lot.
pub fn plan_arrangement(p: &PlanInput) -> Arrangement {
    let windows = p.windows;
    let ordered = layout_candidates(windows, p.excluded);
    let Some(&first) = ordered.first() else {
        return Arrangement::default();
    };
    if p.screens.is_empty() {
        return Arrangement::default();
    }
    let main = p.focused.filter(|f| ordered.contains(f)).unwrap_or(first);
    let screen = |i: usize| p.screens[i.min(p.screens.len() - 1)];
    let parks = |&i: &usize| i != main && on_lot(&windows[i], p);
    let lot: Vec<usize> = ordered.iter().copied().filter(parks).collect();
    let rest: Vec<usize> = rest_sequence(&ordered, main, p.rest_hint)
        .into_iter()
        .filter(|i| !lot.contains(i))
        .collect();

    let mut out = match p.target {
        Target::ControlRoom {
            work,
            control,
            flipped: true,
            ..
        } => Arrangement {
            frames: flipped_arrange(screen(work), screen(control), main, &rest, windows),
            main: Some(main),
            ..Default::default()
        },
        Target::ControlRoom {
            work,
            control,
            ready_side,
            flipped: false,
        } => Arrangement {
            frames: ready_beside_arrange(
                screen(work),
                screen(control),
                ready_side,
                main,
                &rest,
                windows,
            ),
            main: Some(main),
            ..Default::default()
        },
        Target::Single(s) => {
            let vis = screen(s);
            match p.mode {
                LayoutMode::Reading => {
                    focus_arrange(vis, p.side, main, &status_sequence(&rest, windows), windows)
                }
                LayoutMode::Focus2 | LayoutMode::Inbox => {
                    // The sides follow the status, not clicks: the running ones by first sighting.
                    let others: Vec<usize> = ordered
                        .iter()
                        .copied()
                        .filter(|&i| i != main && !lot.contains(&i))
                        .collect();
                    if p.mode == LayoutMode::Inbox {
                        inbox_arrange(vis, main, &others, windows, &lot)
                    } else {
                        focus2_arrange(vis, p.side, main, &others, windows)
                    }
                }
                // Fixed and by first sighting: a click rearranges nothing in the grid (Kevin 2026-09-18). No lot.
                LayoutMode::Grid => {
                    return Arrangement {
                        frames: fixed_grid(vis, fitted_grid(p.grid, vis), &ordered),
                        ..Default::default()
                    };
                }
            }
        }
    };

    if let Some(&main_rect) = out.frames.get(&main) {
        let largest = first_max(0..p.screens.len(), |i| rect_area(p.screens[i]))
            .map_or(main_rect, |i| p.screens[i]);
        let peeking = p.peek && !lot.is_empty();
        let (tiles, list) = if peeking {
            peek_layout(largest, p.side, &lot)
        } else {
            (BTreeMap::new(), None)
        };
        // The peek covers its screen: the arrangement's own lists there would lie under the tiles, only the peek's shows.
        // Lists on other screens stay, and an empty lot changes nothing (review quick 2026-11-06).
        if peeking {
            out.lists.retain(|l| !l.rect.intersects(largest));
            out.lists.extend(list);
        }
        for &i in &lot {
            match tiles.get(&i) {
                Some(&tile) => {
                    out.frames.insert(i, tile);
                }
                None => {
                    out.frames.insert(i, main_rect);
                    out.parked.insert(i);
                }
            }
        }
        out.lot = lot.into_iter().collect();
    }
    out
}

/// Width of the list beside the peek's tiles: name, duration and step stay readable (Kevin 2026-11-06 evening).
pub const PEEK_LIST_W: f64 = 400.0;

/// The peek (Kevin 2026-11-06 evening, "Kacheln + Liste", for 20 to 25 workers): as many tiles on `vis` as fit, never
/// below Warp's minimum size, as large as their number allows; whoever finds no tile stands in a list at the side
/// edge (status, name, step, like the labels; a click brings the worker into the middle). MacBook: 15 tiles, or 10
/// beside the list; a large screen 35.
fn peek_layout(
    vis: Rect,
    side: StackSide,
    ids: &[usize],
) -> (BTreeMap<usize, Rect>, Option<SideList>) {
    let room = |area: Rect| {
        count_at_least_one(area.width() / MIN_CARD_W)
            * count_at_least_one(area.height() / MIN_CARD_H)
    };
    if ids.len() <= room(vis) {
        return (best_grid(vis, ids), None);
    }
    let column = rail_rect(vis, side, PEEK_LIST_W);
    let area = main_area(vis, side, PEEK_LIST_W);
    let (tiled, rest) = ids.split_at(room(area).min(ids.len()));
    let fit = ((column.height() - LIST_HEADER_H) / LIST_ROW_H)
        .floor()
        .max(0.0) as usize;
    let (rows, more) = if rest.len() <= fit {
        (rest.len(), 0)
    } else {
        let rows = fit.saturating_sub(1);
        (rows, rest.len() - rows)
    };
    let mut entries = vec![ListEntry::Header(format!(
        "+{} {}",
        rest.len(),
        tr("weitere auf dem Parkplatz", "more in the parking lot")
    ))];
    entries.extend(rest[..rows].iter().map(|&i| ListEntry::Row(i)));
    if more > 0 {
        entries.push(ListEntry::More(more));
    }
    let height = LIST_HEADER_H + (rows + usize::from(more > 0)) as f64 * LIST_ROW_H;
    let list = SideList {
        rect: Rect::new(
            column.min_x(),
            column.max_y() - height,
            column.width(),
            height,
        ),
        entries,
        inbox: false,
    };
    (best_grid(area, tiled), Some(list))
}

/// All `ids` on `area` (the caller made sure they fit at Warp's minimum size) in the grid with the fewest empty cells;
/// on a tie the cells closest to a usual window shape (16:10).
fn best_grid(area: Rect, ids: &[usize]) -> BTreeMap<usize, Rect> {
    let n = ids.len();
    if n == 0 {
        return BTreeMap::new();
    }
    let max_cols = count_at_least_one(area.width() / MIN_CARD_W).min(n);
    let max_rows = count_at_least_one(area.height() / MIN_CARD_H);
    let shape = |cols: usize, rows: usize| {
        let aspect = (area.width() / cols as f64) / (area.height() / rows as f64);
        (aspect / 1.6).ln().abs()
    };
    let (cols, rows) = (1..=max_cols)
        .map(|c| (c, n.div_ceil(c)))
        .filter(|&(_, r)| r <= max_rows)
        .min_by(|&(c1, r1), &(c2, r2)| {
            (c1 * r1)
                .cmp(&(c2 * r2))
                .then(cmp_f64(shape(c1, r1), shape(c2, r2)))
        })
        .unwrap_or((max_cols, max_rows));
    let (w, h) = (
        (area.width() / cols as f64).floor(),
        (area.height() / rows as f64).floor(),
    );
    ids.iter()
        .enumerate()
        .map(|(k, &i)| {
            let (col, row) = (k % cols, k / cols);
            (
                i,
                Rect::new(
                    area.min_x() + col as f64 * w,
                    area.max_y() - (row + 1) as f64 * h,
                    w,
                    h,
                ),
            )
        })
        .collect()
}

/// Only the frames (`plan_arrangement`).
pub fn plan_layout(p: &PlanInput) -> BTreeMap<usize, Rect> {
    plan_arrangement(p).frames
}

/// Swift's `"\(target)"` in the plan log line: the log stays Swift's byte for byte, so both apps can be compared.
pub fn target_log_text(t: &Target) -> String {
    match *t {
        Target::Single(s) => format!("single({s})"),
        Target::ControlRoom {
            work,
            control,
            ready_side,
            flipped,
        } => format!(
            "leitstand(work: {work}, control: {control}, readySide: Lookout.StackSide.{}, flipped: {flipped})",
            ready_side.raw()
        ),
    }
}

/// The plan log line (Main.swift `logPlan`), one per change: "Plan: ziel=… -> …, anordnung=…, N Fenster", then the
/// windows behind the middle that a list shows ("M geparkt") and the parking lot ("Parkplatz K", Kevin 2026-11-06).
pub fn plan_log_line(
    choice: &TargetChoice,
    target: &Target,
    mode: LayoutMode,
    arr: &Arrangement,
) -> String {
    let mut line = format!(
        "Plan: ziel={} -> {}, anordnung={}, {} Fenster",
        choice.raw(),
        target_log_text(target),
        mode.raw(),
        arr.frames.len()
    );
    let listed = arr.parked.difference(&arr.lot).count();
    if listed > 0 {
        line += &format!(", {listed} geparkt");
    }
    if !arr.lot.is_empty() {
        line += &format!(", Parkplatz {}", arr.lot.len());
    }
    line
}

#[cfg(test)]
mod tests;
