//! Pure parts of moving windows (Common.swift): coordinate conversion at the AX boundary, the order of frame writes,
//! the glide path, when to move again, and the menu bar badge.
//!
//! Coordinate convention (plan, convention 1): everything inside is Cocoa (origin bottom-left, y up); conversion
//! happens only where AX is written (`cocoa_to_ax`) or read (`ax_to_cocoa`).

use crate::geom::{Point, Rect, Size};

pub const WARP_BID: &str = "dev.warp.Warp-Stable";
pub const GHOSTTY_BID: &str = "com.mitchellh.ghostty";
pub const TERMINAL_BIDS: [&str; 2] = [WARP_BID, GHOSTTY_BID];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxOrigin {
    CocoaBottomLeft,
    TopLeft,
}

/// AX origin per app. Spike 2026-09-18: Warp and Ghostty both report top-left (checked against CGWindowList), which is
/// also the default for any other app.
pub fn ax_origin(_bundle_id: &str) -> AxOrigin {
    AxOrigin::TopLeft
}

/// Cocoa (bottom-left) -> AX; `screen_h` is the height of the primary screen.
pub fn cocoa_to_ax(r: Rect, o: AxOrigin, screen_h: f64) -> Rect {
    if o != AxOrigin::TopLeft {
        return r;
    }
    Rect::new(r.min_x(), screen_h - r.max_y(), r.width(), r.height())
}

/// AX -> Cocoa: the same mirror, so its own inverse.
pub fn ax_to_cocoa(r: Rect, o: AxOrigin, screen_h: f64) -> Rect {
    cocoa_to_ax(r, o, screen_h)
}

// ---------- Write order when moving (design v3, bug 2) ----------
//
// Measured 2026-09-18 on own test windows: if a position write pushes a Warp window that still has its old size across
// the edge of its screen onto the other one, every following shrink clamps the bottom edge (804×677 @(0,-57) instead of
// 804×619 @(0,1), exactly as in the log, on every retry). Ghostty does not clamp. The old order "position, size,
// position" triggered exactly that whenever a tall window had to go to a lower tile.

/// One write to a window, in AX coordinates (top-left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AxWrite {
    Position(Point),
    Size(Size),
}

/// Shrink at the old place (top-left stays), then move (in that size the window fits the target), grow only at the
/// target. No intermediate state leaves the old or the new frame; growing only at the target also keeps macOS from
/// cutting a size that no longer fit at the old place.
pub fn frame_writes(current: Rect, target: Rect) -> Vec<AxWrite> {
    let shrunk = Size {
        w: current.width().min(target.width()),
        h: current.height().min(target.height()),
    };
    let mut out = vec![];
    if shrunk != current.size() {
        out.push(AxWrite::Size(shrunk));
    }
    out.push(AxWrite::Position(target.origin()));
    if shrunk != target.size() {
        out.push(AxWrite::Size(target.size()));
    }
    out
}

/// Rescue for a window that already clamps across two screens (e.g. from v2): first move it in its current size so
/// its bottom edge lies on the target's, then the size, then the position. Measured: 804×677 @(0,-57) -> exactly
/// 804×619 @(0,1).
pub fn rescue_writes(current: Rect, target: Rect) -> [AxWrite; 3] {
    [
        AxWrite::Position(Point {
            x: target.min_x(),
            y: target.max_y() - current.height(),
        }),
        AxWrite::Size(target.size()),
        AxWrite::Position(target.origin()),
    ]
}

/// Glide path of a move (Cocoa): the size does not change on the way. Shrink to `min(old, new)` at the old place
/// (anchored top-left like AX), glide in that size, grow at the target. Size changes in every frame clamped Warp
/// windows as soon as a frame poked over the screen edge, and every terminal rebuilds its character grid on each.
pub fn glide_path(from: Rect, to: Rect) -> (Rect, Rect) {
    let w = from.width().min(to.width());
    let h = from.height().min(to.height());
    (
        Rect::new(from.min_x(), from.max_y() - h, w, h),
        Rect::new(to.min_x(), to.max_y() - h, w, h),
    )
}

/// Deviation between target and actual frame from which a window is really moved (plan, convention 4).
pub const FRAME_TOLERANCE: f64 = 24.0;

pub fn frames_differ(a: Rect, b: Rect, tolerance: f64) -> bool {
    (a.min_x() - b.min_x()).abs() > tolerance
        || (a.min_y() - b.min_y()).abs() > tolerance
        || (a.width() - b.width()).abs() > tolerance
        || (a.height() - b.height()).abs() > tolerance
}

/// Where was a window last sent, and where did it stop?
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveMemory {
    pub target: Rect,
    pub reached: Rect,
}

/// Must the window move? Not if it already stands at its target, and not if it missed exactly this target before and
/// has not moved since; otherwise every tick (every 3 s) repeats the same futile try (log 2026-09-18: 883 times). A new
/// target or a window Kevin moved -> arrange again.
pub fn needs_move(current: Rect, dest: Rect, memory: Option<&MoveMemory>) -> bool {
    if !frames_differ(current, dest, FRAME_TOLERANCE) {
        return false;
    }
    memory.is_none_or(|m| {
        frames_differ(m.target, dest, FRAME_TOLERANCE)
            || frames_differ(m.reached, current, FRAME_TOLERANCE)
    })
}

/// Menu bar symbol state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Badge {
    pub filled: bool,
    pub dimmed: bool,
}

/// The mini app icon in the menu bar, the symbol only (Kevin 2026-09-26: "?", "!" and numbers beside it irritate):
/// its top-right tile is filled as soon as a session is done or asks. Without Accessibility access the symbol is
/// grey. How many and which ones the menu says; question or done the frames show.
pub fn status_badge(ready: usize, asking: usize, trusted: bool) -> Badge {
    Badge {
        filled: trusted && ready + asking > 0,
        dimmed: !trusted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- Koordinaten (convention 1) ----------

    #[test]
    fn test_coordinate_round_trip() {
        let screen_h = 1269.0;
        let cocoa = Rect::new(100.0, 200.0, 800.0, 600.0);
        let ax = cocoa_to_ax(cocoa, AxOrigin::TopLeft, screen_h);
        assert_eq!(ax.y, screen_h - cocoa.max_y(), "AX y measures from the top");
        assert_eq!(ax.x, cocoa.x, "x stays x");
        assert_eq!(
            ax_to_cocoa(ax, AxOrigin::TopLeft, screen_h),
            cocoa,
            "there and back must be identical"
        );
        let unchanged = cocoa_to_ax(cocoa, AxOrigin::CocoaBottomLeft, screen_h);
        assert_eq!(
            unchanged, cocoa,
            "without conversion the rectangle stays the same"
        );
    }

    #[test]
    fn test_frames_differ() {
        let a = Rect::new(0.0, 0.0, 100.0, 100.0);
        assert!(
            !frames_differ(a, a.offset_by(20.0, 0.0), FRAME_TOLERANCE),
            "20 pt are within tolerance"
        );
        assert!(
            frames_differ(a, a.offset_by(30.0, 0.0), FRAME_TOLERANCE),
            "30 pt are a real deviation"
        );
    }

    // ---------- Fehler 2: kein Endlos-Neuversuch ----------

    #[test]
    fn test_no_endless_retry() {
        let soll = Rect::new(0.0, 1.0, 804.0, 619.0);
        let ist = Rect::new(0.0, -57.0, 804.0, 677.0);
        assert!(needs_move(ist, soll, None), "first try");
        assert!(!needs_move(soll, soll, None), "already in place");
        let gemerkt = MoveMemory {
            target: soll,
            reached: ist,
        };
        assert!(
            !needs_move(ist, soll, Some(&gemerkt)),
            "same target, same actual frame -> not again"
        );
        assert!(
            needs_move(ist, soll.offset_by(400.0, 0.0), Some(&gemerkt)),
            "new target -> move"
        );
        assert!(
            needs_move(ist.offset_by(300.0, 0.0), soll, Some(&gemerkt)),
            "Kevin moved the window -> arrange again"
        );
    }

    /// Measured 2026-09-18: if a Warp window pokes over its screen after a move (still in its old size), every
    /// following shrink clamps its bottom edge. Invariant: no intermediate state leaves the old or the new frame.
    #[test]
    fn test_frame_writes_stay_inside() {
        fn apply(writes: &[AxWrite], start: Rect, mut check: impl FnMut(Rect)) -> Rect {
            let mut r = start;
            for w in writes {
                match *w {
                    AxWrite::Position(p) => (r.x, r.y) = (p.x, p.y),
                    AxWrite::Size(s) => (r.w, r.h) = (s.w, s.h), // AX: top-left stays
                }
                check(r);
            }
            r
        }
        // AX coordinates (y down): full-height LG strip -> lower card, MacBook -> LG, growing.
        let pairs = [
            (
                Rect::new(804.0, 30.0, 1400.0, 1239.0),
                Rect::new(0.0, 649.0, 804.0, 619.0),
            ),
            (
                Rect::new(687.0, 1301.0, 1728.0, 1007.0),
                Rect::new(0.0, 649.0, 804.0, 619.0),
            ),
            (
                Rect::new(1935.0, 1805.0, 480.0, 503.0),
                Rect::new(804.0, 30.0, 1400.0, 1239.0),
            ),
            (
                Rect::new(687.0, 1301.0, 1728.0, 503.0),
                Rect::new(1935.0, 1301.0, 480.0, 1007.0),
            ),
        ];
        for (from, to) in pairs {
            let end = apply(&frame_writes(from, to), from, |r| {
                assert!(
                    from.contains(r) || to.contains(r),
                    "intermediate {r:?} leaves old {from:?} and new {to:?}"
                );
            });
            assert_eq!(end, to, "the target stands at the end");
        }
        // Rescue of an already clamped window: first in its current size, bottom-aligned on the target.
        let klemmt = Rect::new(0.0, 649.0, 804.0, 677.0);
        let ziel = Rect::new(0.0, 649.0, 804.0, 619.0);
        let end = apply(&rescue_writes(klemmt, ziel), klemmt, |r| {
            assert!(
                r.max_y() <= ziel.max_y(),
                "intermediate {r:?} pokes below the target"
            );
        });
        assert_eq!(end, ziel);
    }

    /// The size no longer changes on the way: shrink at the old place (top-left stays), glide, grow at the target.
    #[test]
    fn test_glide_path() {
        let pairs = [
            (
                Rect::new(804.0, 0.0, 1400.0, 1239.0),
                Rect::new(0.0, 1.0, 804.0, 619.0),
            ),
            (
                Rect::new(1935.0, -1039.0, 480.0, 503.0),
                Rect::new(804.0, 0.0, 1400.0, 1239.0),
            ),
            (
                Rect::new(687.0, -536.0, 1728.0, 503.0),
                Rect::new(1935.0, -1039.0, 480.0, 1007.0),
            ),
        ];
        for (from, to) in pairs {
            let (a, b) = glide_path(from, to);
            assert_eq!(a.size(), b.size(), "no size change on the way");
            assert!(
                from.contains(a) && to.contains(b),
                "start in the old, end in the new frame"
            );
            assert!(
                a.min_x() == from.min_x() && a.max_y() == from.max_y(),
                "shrinking is anchored top-left (like AX)"
            );
            assert!(
                b.min_x() == to.min_x() && b.max_y() == to.max_y(),
                "at the target it grows from the top left"
            );
        }
    }

    // ---------- Plan 1b: Menueleisten-Symbol ----------

    /// Kevin 2026-09-26: "?", "!" and numbers next to the symbol irritate; only the symbol.
    #[test]
    fn test_status_badge() {
        let badge = |filled, dimmed| Badge { filled, dimmed };
        assert_eq!(
            status_badge(0, 0, true),
            badge(false, false),
            "nothing waits: empty tile"
        );
        assert_eq!(status_badge(3, 0, true), badge(true, false), "done: filled");
        assert_eq!(
            status_badge(0, 1, true),
            badge(true, false),
            "question: filled"
        );
        assert_eq!(
            status_badge(2, 1, true),
            badge(true, false),
            "question and done: filled"
        );
        assert_eq!(
            status_badge(2, 1, false),
            badge(false, true),
            "no access: grey, tile empty"
        );
    }
}
