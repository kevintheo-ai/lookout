//! Moving windows (Animator.swift): windows glide to their target instead of jumping. The timing is core
//! `lookout_core::overlay::Glide` (time-based, ease-in-out, staggered start, only moving on the way); this runs it on the
//! AX queue, ONE short enqueue per frame (`FRAME_INTERVAL`), so the queue stays free between frames. A generation token
//! replaces a running move: a new layout change wins.

use lookout_core::geom::Rect;
use lookout_core::motion::AxOrigin;

/// One window to move, Cocoa rectangles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMove {
    pub id: u64,
    pub origin: AxOrigin,
    pub from: Rect,
    pub to: Rect,
}

pub struct Animator(());

impl Animator {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Animator {
        todo!()
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
        let _ = (moves, screen_h, on_step, on_finish);
        todo!()
    }

    /// Stops the running move (pause, quit); its `on_finish` never comes.
    pub fn cancel(&self) {
        todo!()
    }
}
