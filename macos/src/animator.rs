//! Moving windows (Animator.swift): windows glide to their target instead of jumping. The timing is core
//! `lookout_core::overlay::Glide` (time-based, ease-in-out, staggered start, only moving on the way); this runs it on the
//! AX queue, ONE short enqueue per frame (`FRAME_INTERVAL`), so the queue stays free between frames. A generation token
//! replaces a running move: a new layout change wins.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Instant;

use lookout_core::geom::Rect;
use lookout_core::motion::{AxOrigin, FRAME_TOLERANCE, frames_differ};
use lookout_core::overlay::{FRAME_INTERVAL, Glide, GlideStep, MOVE_DURATION};

use crate::ax;

/// One window to move, Cocoa rectangles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMove {
    pub id: u64,
    pub origin: AxOrigin,
    pub from: Rect,
    pub to: Rect,
}

/// The generation token: `run` and `cancel` bump it, every frame checks it. Frames hold it weakly (Swift `[weak self]`):
/// a dropped animator stops its move too.
pub struct Animator(Arc<AtomicU64>);

type Frames = Vec<(u64, Rect)>;

/// A running move, handed from frame to frame on the AX queue.
struct Run {
    generation: Weak<AtomicU64>,
    token: u64,
    moves: Vec<WindowMove>,
    screen_h: f64,
    glide: Glide,
    start: Instant,
    measured: Frames,
    on_step: Box<dyn Fn(Frames) + Send>,
    on_finish: Box<dyn FnOnce(Frames) + Send>,
}

impl Run {
    fn replaced(&self) -> bool {
        self.generation
            .upgrade()
            .is_none_or(|g| g.load(Ordering::SeqCst) != self.token)
    }
}

impl Animator {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Animator {
        Animator(Arc::new(AtomicU64::new(0)))
    }

    /// Glides `moves` (duration `MOVE_DURATION`). Per frame on the AX queue: `Shrink` -> `ax::resize_in_place`, `Move`
    /// -> `ax::move_position`, `Arrive` -> `ax::set_frame` with read-back, logging like Swift when it stops elsewhere
    /// ("Animator: Fenster bleibt bei 804×619 @(0,1) statt 804×677 @(0,-57) — akzeptiert", integers, more than
    /// `FRAME_TOLERANCE` off). `on_step` (AX queue): per frame the Cocoa rectangles of the windows that moved, the
    /// measured one on arrival; the last frame comes before `on_finish`. `on_finish` (AX queue): the MEASURED final frames
    /// of the windows whose read-back worked; not called when a newer run or `cancel` replaced this one. No moves:
    /// `on_finish(vec![])` right away.
    pub fn run(
        &self,
        moves: Vec<WindowMove>,
        screen_h: f64,
        on_step: impl Fn(Vec<(u64, Rect)>) + Send + Sync + 'static,
        on_finish: impl FnOnce(Vec<(u64, Rect)>) + Send + 'static,
    ) {
        // Swift: on the caller's thread, and a running move goes on (no new generation).
        if moves.is_empty() {
            on_finish(vec![]);
            return;
        }
        let token = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        let generation = Arc::downgrade(&self.0);
        crate::ax_queue().exec_async(move || {
            let pairs: Vec<(Rect, Rect)> = moves.iter().map(|m| (m.from, m.to)).collect();
            frame(Run {
                generation,
                token,
                glide: Glide::new(&pairs, MOVE_DURATION),
                moves,
                screen_h,
                // The clock starts with the first frame on the AX queue, as in Swift: work queued before it does not eat
                // into the glide.
                start: Instant::now(),
                measured: vec![],
                on_step: Box::new(on_step),
                on_finish: Box::new(on_finish),
            });
        });
    }

    /// Stops the running move (pause, quit); its `on_finish` never comes.
    pub fn cancel(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// One frame: each window shrinks at ITS start (still at the old place), glides, and is set exactly with read-back at
/// ITS arrival (core `Glide`).
fn frame(mut run: Run) {
    if run.replaced() {
        return; // touch nothing more
    }
    let elapsed = run.start.elapsed().as_secs_f64();
    let mut steps = vec![];
    for step in run.glide.frame(elapsed) {
        match step {
            GlideStep::Shrink(i, size) => ax::resize_in_place(run.moves[i].id, size),
            GlideStep::Move(i, r) => {
                let m = run.moves[i];
                ax::move_position(m.id, r, m.origin, run.screen_h);
                steps.push((m.id, r));
            }
            GlideStep::Arrive(i) => {
                let m = run.moves[i];
                let Some(got) = ax::set_frame(m.id, m.to, m.origin, run.screen_h) else {
                    continue;
                };
                run.measured.push((m.id, got));
                steps.push((m.id, got));
                if frames_differ(got, m.to, FRAME_TOLERANCE) {
                    crate::log(&off_target(got, m.to));
                }
            }
        }
    }
    // Swift could not be replaced within a frame (its token lived on the AX queue); this token can, from main.
    if run.replaced() {
        return;
    }
    if !steps.is_empty() {
        (run.on_step)(steps);
    }
    if run.glide.done() {
        (run.on_finish)(run.measured);
        return;
    }
    crate::after(crate::ax_queue(), FRAME_INTERVAL, move || frame(run));
}

/// Swift's line for a window that stopped elsewhere; `as i64` truncates toward zero like Swift's `Int()`.
fn off_target(got: Rect, to: Rect) -> String {
    format!(
        "Animator: Fenster bleibt bei {}×{} @({},{}) statt {}×{} @({},{}) — akzeptiert",
        got.width() as i64,
        got.height() as i64,
        got.min_x() as i64,
        got.min_y() as i64,
        to.width() as i64,
        to.height() as i64,
        to.min_x() as i64,
        to.min_y() as i64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn nothing_to_move_finishes_at_once() {
        let (tx, rx) = mpsc::channel();
        Animator::new().run(
            vec![],
            1000.0,
            |_| unreachable!("no step without moves"),
            move |f| tx.send(f).unwrap(),
        );
        assert_eq!(
            rx.try_recv(),
            Ok(vec![]),
            "on_finish(vec![]) before run returns"
        );
    }

    /// The frame loop on the real AX queue, with an id the registry never hands out: every AX call returns before it
    /// could touch a window, so no read-back and nothing measured.
    #[test]
    fn a_run_finishes_once_unless_replaced_or_cancelled() {
        let ghost = |x| WindowMove {
            id: u64::MAX,
            origin: AxOrigin::TopLeft,
            from: Rect::new(0.0, 0.0, 100.0, 100.0),
            to: Rect::new(x, 0.0, 100.0, 100.0),
        };
        let animator = Animator::new();
        let (tx, rx) = mpsc::channel();
        let finish = |name: &'static str| {
            let tx = tx.clone();
            move |f: Frames| tx.send((name, f)).unwrap()
        };
        let (step_tx, steps) = mpsc::channel();
        animator.run(vec![ghost(500.0)], 1000.0, |_| {}, finish("replaced"));
        animator.run(
            vec![ghost(800.0)],
            1000.0,
            move |s| step_tx.send(s).unwrap(),
            finish("newest"),
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)),
            Ok(("newest", vec![])),
            "only the newer run finishes"
        );
        let xs: Vec<f64> = steps.try_iter().flatten().map(|(_, r)| r.x).collect();
        assert!(
            !xs.is_empty()
                && xs.windows(2).all(|w| w[0] <= w[1])
                && xs.iter().all(|&x| (0.0..800.0).contains(&x)),
            "the steps glide from 0 toward 800: {xs:?}"
        );
        animator.run(vec![ghost(500.0)], 1000.0, |_| {}, finish("cancelled"));
        animator.cancel();
        assert!(
            rx.recv_timeout(Duration::from_secs(1)).is_err(),
            "neither the cancelled nor the replaced run finishes"
        );
    }

    #[test]
    fn off_target_line_is_swifts() {
        let line =
            "Animator: Fenster bleibt bei 804×619 @(0,1) statt 804×677 @(0,-57) — akzeptiert";
        assert_eq!(
            off_target(
                Rect::new(0.0, 1.0, 804.0, 619.0),
                Rect::new(0.0, -57.0, 804.0, 677.0)
            ),
            line
        );
        assert_eq!(
            off_target(
                Rect::new(-0.5, 1.9, 804.9, 619.9),
                Rect::new(0.0, -57.9, 804.0, 677.0)
            ),
            line,
            "truncated toward zero, not rounded or floored"
        );
    }
}
