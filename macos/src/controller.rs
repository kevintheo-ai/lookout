//! The app (Main.swift `LookoutController`): state, tick, focus, arranging, and the parking lot with its peek (Kevin
//! 2026-11-06). The decisions are core functions; this wires them to the system. Everything here runs on the main
//! thread, except the tick's work block (work queue, `tick_work`) and the AX calls it and the moves make (AX queue).
//!
//! The app lives in a main-thread slot (`with_app`); AppKit callbacks (menu, timers, notifications, overlays, hotkeys)
//! reach it through that slot.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy,
    NSApplicationDidChangeScreenParametersNotification, NSCellImagePosition, NSImage, NSMenu,
    NSRunningApplication, NSStatusBar, NSStatusItem, NSVariableStatusItemLength, NSWorkspace,
    NSWorkspaceActiveSpaceDidChangeNotification, NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::{
    NSLocale, NSNotification, NSNotificationCenter, NSObjectProtocol, NSOperationQueue, NSTimer,
};

use lookout_core::config::{AppConfig, color_from_hex, config_from_json};
use lookout_core::geom::{Point, Rect};
use lookout_core::i18n::{is_german, prefers_german, set_german};
use lookout_core::layout::{
    DEFAULT_GRID, PlanInput, attention_order, is_worker, layout_candidates, lot_member,
    plan_arrangement, plan_log_line, resolve_target, rest_sequence, status_swap, swapped_rest,
    unique_screen_ids,
};
use lookout_core::mapping::{
    PING_POLL, PING_ROUNDS, PING_WAIT, Ping, PingState, attach_processes, index_of_window,
    keep_measured_frames, match_sessions, ping_result, ping_schedule, session_in_front,
    title_matches,
};
use lookout_core::model::Engine;
use lookout_core::model::{
    Arrangement, EngineProc, GridSpec, LayoutMode, LookoutLine, OnScreenWindow, Pair, Pid, PsRow,
    ScreenInfo, Session, StackSide, Target, TargetChoice, TermWindow, Time, graphemes, title_glyph,
};
use lookout_core::motion::{Badge, MoveMemory, ax_origin, frames_differ, needs_move, status_badge};
use lookout_core::overlay::{
    GlowTarget, MoveGate, OnScreenMatch, OverlayInput, SinceEntry, advance_since, first_name,
    focus_source, frontmost_number, glow_targets, label_targets, list_target, locate_on_screen,
    set_greeting_name,
};
use lookout_core::status::{
    BackgroundWatch, ClaudeStatus, LogSessionFinder, Standby, TAIL_BYTES, TranscriptReader,
    apply_lookout_lines, fetch_agents, launch_corrected,
};

use crate::animator::{Animator, WindowMove};
use crate::focus::FocusWatcher;
use crate::overlay::{GlowOverlay, LabelOverlay, ListOverlay, PreviewOverlay};
use crate::{
    ax, ax_queue, ax_sync, hotkeys, icon, log, menu, mouse, now, on_main, on_main_after, prefs,
    screens,
};
use crate::{sys, work_queue};

// ---------- Memory (UserDefaults keys of the Swift app) ----------

pub(crate) const DEFAULTS_MODE: &str = "lookout.mode";
pub(crate) const DEFAULTS_TARGET: &str = "lookout.target";
pub(crate) const DEFAULTS_GRID: &str = "lookout.grid";
pub(crate) const DEFAULTS_EXCLUDED: &str = "lookout.excluded";
pub(crate) const DEFAULTS_LABELS: &str = "lookout.labels";
/// Sessions sent to the parking lot by hand (new in the Rust app; the Swift app ignores the key).
pub(crate) const DEFAULTS_PARKED: &str = "lookout.parked";
/// Workers taken off the parking lot by hand (Rust app only, like `lookout.parked`).
pub(crate) const DEFAULTS_KEPT: &str = "lookout.kept";
/// Control-room screens, chosen in the menu from three screens on (Kevin 2026-10-01).
pub(crate) const DEFAULTS_PAIR_WORK: &str = "lookout.leitstand.work";
pub(crate) const DEFAULTS_PAIR_CONTROL: &str = "lookout.leitstand.control";

/// A week: how long a seen answer is remembered (review quick 2026-10-04, GLM).
const SEEN_KEEP: Time = 7.0 * 86_400.0;

// ---------- The app slot ----------

/// Work for the app that had to wait for its turn (`with_app`).
type Job = Box<dyn FnOnce(&mut App)>;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static PENDING: RefCell<Vec<Job>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` on the app (main thread). If AppKit called back into Lookout while the app is busy, `f` runs on the next
/// turn of the main queue instead of being lost.
pub(crate) fn with_app(f: impl FnOnce(&mut App) + 'static) {
    let mut f = Some(f);
    APP.with(|slot| {
        if let Ok(mut slot) = slot.try_borrow_mut()
            && let (Some(app), Some(f)) = (slot.as_mut(), f.take())
        {
            f(app);
        }
    });
    if let Some(f) = f {
        PENDING.with(|p| p.borrow_mut().push(Box::new(f)));
        on_main(drain_pending);
    }
}

/// Runs `f` on the app now, for AppKit callbacks that must answer at once (building the menu). `None` while the app
/// is busy or not running.
pub(crate) fn with_app_now<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|slot| slot.try_borrow_mut().ok()?.as_mut().map(f))
}

fn drain_pending() {
    let jobs = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for job in jobs {
        with_app(job);
    }
}

// ---------- Start ----------

/// Starts the app on the main thread and runs it until Quit.
pub fn run() {
    let mtm = MainThreadMarker::new().expect("Lookout must start on the main thread");
    let ns_app = NSApplication::sharedApplication(mtm);
    ns_app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let app = App::new(mtm);
    APP.with(|slot| *slot.borrow_mut() = Some(app));
    with_app(App::start);
    ns_app.run();
}

/// How the peek ends (`App::end_peek`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PeekEnd {
    /// Shortcut or menu: the tiles go back, Kevin's middle to the front with focus.
    Back,
    /// A click on this tile: it comes into the middle with focus.
    Into(u64),
    /// Only the state ends; the caller arranges (pause, empty lot, a click outside, a session from the menu).
    Only,
}

/// The status engines of the tick. They hold state across ticks and are used only inside the work block, one at a
/// time (`tick_in_flight`).
struct Engines {
    transcripts: TranscriptReader,
    finder: LogSessionFinder,
    background: BackgroundWatch,
    claude: ClaudeStatus,
    standby: Standby,
}

impl Engines {
    fn new(cfg: &AppConfig) -> Engines {
        let home = crate::home();
        let agents_home = home.clone();
        Engines {
            transcripts: TranscriptReader::new(cfg.claude_config_dirs.clone(), TAIL_BYTES),
            finder: LogSessionFinder::new(home, Box::new(sys::process_start)),
            background: BackgroundWatch::new(
                Box::new(sys::nettop_usage),
                Box::new(|pids: &[Pid]| sys::listening_pids(pids)),
                cfg.lead_names.iter().cloned().collect(),
                cfg.other_window_names.clone(),
                Box::new(|l: &str| log(l)),
            ),
            claude: ClaudeStatus::new(
                cfg.claude_config_dirs.clone(),
                Box::new(move |dir: &str| fetch_agents(dir, &agents_home, &mut |l| log(l))),
                Box::new(sys::process_alive),
                Box::new(sys::process_start),
                Box::new(|l: &str| log(l)),
            ),
            standby: Standby::new(
                cfg.other_window_names.clone(),
                cfg.lead_names.iter().cloned().collect(),
                Box::new(|l: &str| log(l)),
            ),
        }
    }
}

// ---------- State ----------

pub(crate) struct App {
    pub(crate) mtm: MainThreadMarker,
    status_item: Retained<NSStatusItem>,
    pub(crate) menu: Retained<NSMenu>,
    pub(crate) delegate: Retained<menu::Delegate>,
    animator: Animator,
    glow: GlowOverlay,
    pub(crate) preview: PreviewOverlay,
    labels: LabelOverlay,
    /// Lists at the foot of the sides (plan 1d): what does not fit as a window.
    lists: ListOverlay,
    focus_watcher: FocusWatcher,
    tile_empty: Retained<NSImage>,
    tile_full: Retained<NSImage>,
    /// Last menu bar symbol set (`update_badge`).
    shown_badge: Option<Badge>,
    engines: Arc<Mutex<Engines>>,
    /// Since when every session is in its state (label duration).
    pub(crate) since: BTreeMap<String, SinceEntry>,
    pub(crate) labels_on: bool,
    last_label_log: String,
    last_glow_log: String,
    last_focus_log: String,
    last_plan_log: String,
    /// Sessions of the windows gliding right now: their label fades out during the ride, their glow rides along.
    moving_ids: BTreeSet<String>,
    /// Clicks during own moves are noted instead of dropped (v4).
    focus_gate: MoveGate,
    /// Arranging during own moves is noted: the remembered frames are wrong then (2026-09-25).
    layout_gate: MoveGate,
    /// Counts start and end of every own move: a tick whose snapshot overlaps one shows intermediate frames.
    move_epoch: u64,
    pub(crate) cfg: AppConfig,
    pub(crate) mode: LayoutMode,
    pub(crate) choice: TargetChoice,
    /// Control-room screens (ids), chosen in the menu from three screens on; `None` = automatic.
    pub(crate) pair: Option<Pair<String>>,
    pub(crate) grid: GridSpec,
    /// Session ids Kevin took out of the arrangement (design v3). ponytail: never thinned out, a few dozen ids weigh
    /// nothing in the defaults.
    pub(crate) excluded: BTreeSet<String>,
    /// Session ids sent to the parking lot by hand (Ctrl+Opt+5 or the menu, Kevin 2026-11-06 evening); they park like
    /// workers. ponytail: never thinned out either, like `excluded`.
    pub(crate) hand_parked: BTreeSet<String>,
    /// Session ids of workers taken off the parking lot by hand: they stay in the arrangement.
    pub(crate) hand_kept: BTreeSet<String>,
    pub(crate) side: StackSide,
    pub(crate) paused: bool,
    tick_in_flight: bool,
    timer: Option<Retained<NSTimer>>,
    pub(crate) trusted: bool,
    sessions: Vec<Session>,
    /// Sessions with a running process (or background agents): the "without window" list.
    pub(crate) live_sessions: Vec<Session>,
    pub(crate) windows: Vec<TermWindow>,
    pid_windows: BTreeMap<Pid, u64>,
    first_seen: BTreeMap<u64, Time>,
    /// Since when a window is done: order of the left column in Focus 2 (whoever was done first stands on top).
    ready_since: BTreeMap<u64, Time>,
    last_session_fetch: Time,
    /// During own moves focus reports are no clicks (convention 7).
    self_move: bool,
    /// The last focused AI window: never gets a glow.
    focused: Option<u64>,
    /// Session id -> when Kevin last had its window in front (`answer_seen`, Kevin 2026-10-04). Written by the tick.
    seen_at: BTreeMap<String, Time>,
    /// The main window in Focus, Focus 2 and control room: the last clicked window Lookout also arranges.
    main: Option<u64>,
    /// Order of the side windows (Focus) or tiles (control room), so a click SWAPS instead of re-sorting all.
    rest_order: Vec<u64>,
    /// Targets a window missed: not every tick again (`needs_move`).
    move_memory: BTreeMap<u64, MoveMemory>,
    /// Processes still (or again) to ping (`ping_schedule`).
    ping_pending: BTreeMap<Pid, PingState>,
    /// The last readable process list: kept when it cannot be read once (review 2026-09-25 finding 10).
    last_procs: Vec<EngineProc>,
    last_rows: Vec<PsRow>,
    /// The last applied plan (plan 1d) and the window ids in its order, so it holds in the pause and until the next tick.
    pub(crate) arrangement: Arrangement,
    pub(crate) arrangement_ids: Vec<u64>,
    /// Where a window stood before Lookout parked it (R2-7), and a way back that rides with the next arrangement.
    park_origin: BTreeMap<u64, Rect>,
    /// One way back per excluded window (review quick 2026-11-06: one slot lost the first of two exclusions in a pause).
    pending_restore: BTreeMap<u64, Rect>,
    /// Last unique window number of the middle (R2-1).
    main_number: Option<(u64, i64)>,
    /// Peek at the parking lot (Ctrl+Opt+4 or the menu, Kevin 2026-11-06): the lot's windows tiled on the largest screen.
    pub(crate) peek: bool,
    /// The peek's tiles stand: while it is open nothing moves, whoever gets done, asks or joins (Kevin 2026-11-06
    /// evening: "es muss schon ruhig bleiben, es soll jetzt nicht die ganze Zeit ein Kommen und ein Gehen sein").
    peek_frozen: bool,
    click_monitor: Option<mouse::ClickMonitor>,
    /// Bring the middle to the front with focus after the next move (the peek ended by shortcut, menu or tile click).
    focus_back: bool,
    /// Focus reports do not count until that focus is back: raising tiles and moving them back makes a tile the key
    /// window for a moment, and its late report put a worker into the middle (live test 2026-11-06 18:57).
    focus_quiet: bool,
    /// Counts the quiet periods: a timer ends only its own (`end_focus_quiet_after`).
    quiet_gen: u64,
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

/// What the tick's work block needs, taken on main.
struct TickInput {
    refresh: bool,
    cached_sessions: Vec<Session>,
    first_seen: BTreeMap<u64, Time>,
    threshold: u64,
    pid_windows: BTreeMap<Pid, u64>,
    pending: BTreeMap<Pid, PingState>,
    known_procs: Vec<EngineProc>,
    known_rows: Vec<PsRow>,
    epoch: u64,
    seen: BTreeMap<String, Time>,
    shown: Vec<TermWindow>,
    front_pid: Option<Pid>,
    screen_h: f64,
    other_window_names: Vec<String>,
}

/// What it brings back to main.
struct TickOutput {
    refresh: bool,
    raw: Vec<Session>,
    live: Vec<Session>,
    windows: Vec<TermWindow>,
    seen: BTreeMap<String, Time>,
    pid_windows: BTreeMap<Pid, u64>,
    pending: BTreeMap<Pid, PingState>,
    procs: Vec<EngineProc>,
    rows: Vec<PsRow>,
    trusted: bool,
    epoch: u64,
}

/// The process of the app in front (NSWorkspace), main thread.
fn frontmost_pid() -> Option<Pid> {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|a| a.processIdentifier())
}

/// Swift's `String.prefix(n)`: the first `n` characters (grapheme clusters).
pub(crate) fn prefix(s: &str, n: usize) -> String {
    graphemes(s).take(n).collect()
}

/// Swift's `String.suffix(n)` for the ASCII session ids in the log.
fn suffix(s: &str, n: usize) -> &str {
    let start = s
        .char_indices()
        .rev()
        .nth(n.saturating_sub(1))
        .map_or(0, |(i, _)| i);
    &s[start..]
}

impl App {
    fn new(mtm: MainThreadMarker) -> App {
        let home = crate::home();
        let cfg = config_from_json(
            &std::fs::read(crate::config_path()).unwrap_or_default(),
            &home,
        );
        let languages: Vec<String> = NSLocale::preferredLanguages()
            .iter()
            .map(|s| s.to_string())
            .collect();
        set_german(prefers_german(cfg.language.as_deref(), &languages));
        set_greeting_name(&first_name(&sys::full_user_name()));

        let mode =
            LayoutMode::parse(&prefs::string(DEFAULTS_MODE).unwrap_or_else(|| cfg.layout.clone()));
        let choice = prefs::string(DEFAULTS_TARGET)
            .and_then(|s| TargetChoice::from_raw(&s))
            .unwrap_or(TargetChoice::External);
        let pair = match (
            prefs::string(DEFAULTS_PAIR_WORK),
            prefs::string(DEFAULTS_PAIR_CONTROL),
        ) {
            (Some(work), Some(control)) => Some(Pair { work, control }),
            _ => None,
        };
        let grid = prefs::string(DEFAULTS_GRID)
            .and_then(|s| GridSpec::parse(&s))
            .unwrap_or(DEFAULT_GRID);
        let excluded: BTreeSet<String> = prefs::string_array(DEFAULTS_EXCLUDED)
            .unwrap_or_default()
            .into_iter()
            .collect();
        let hand_parked: BTreeSet<String> = prefs::string_array(DEFAULTS_PARKED)
            .unwrap_or_default()
            .into_iter()
            .collect();
        let hand_kept: BTreeSet<String> = prefs::string_array(DEFAULTS_KEPT)
            .unwrap_or_default()
            .into_iter()
            .collect();
        let labels_on = prefs::boolean(DEFAULTS_LABELS).unwrap_or(cfg.labels);
        let side = StackSide::parse(&cfg.stack_side);

        let status_item =
            NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        let tile_empty = icon::status_tile_image(false);
        let tile_full = icon::status_tile_image(true);
        let delegate = menu::Delegate::new(mtm);
        let menu = menu::new_menu(mtm, &delegate);
        let engines = Arc::new(Mutex::new(Engines::new(&cfg)));
        App {
            mtm,
            status_item,
            menu,
            delegate,
            animator: Animator::new(),
            glow: GlowOverlay::new(mtm),
            preview: PreviewOverlay::new(mtm),
            labels: LabelOverlay::new(mtm),
            lists: ListOverlay::new(
                mtm,
                Box::new(|ids: Vec<String>| with_app(move |a| a.list_clicked(&ids))),
            ),
            focus_watcher: FocusWatcher::new(Box::new(|| with_app(App::handle_focus_change))),
            tile_empty,
            tile_full,
            shown_badge: None,
            engines,
            since: BTreeMap::new(),
            labels_on,
            last_label_log: String::new(),
            last_glow_log: String::new(),
            last_focus_log: String::new(),
            last_plan_log: String::new(),
            moving_ids: BTreeSet::new(),
            focus_gate: MoveGate::default(),
            layout_gate: MoveGate::default(),
            move_epoch: 0,
            cfg,
            mode,
            choice,
            pair,
            grid,
            excluded,
            hand_parked,
            hand_kept,
            side,
            paused: false,
            tick_in_flight: false,
            timer: None,
            trusted: false,
            sessions: vec![],
            live_sessions: vec![],
            windows: vec![],
            pid_windows: BTreeMap::new(),
            first_seen: BTreeMap::new(),
            ready_since: BTreeMap::new(),
            last_session_fetch: f64::NEG_INFINITY,
            self_move: false,
            focused: None,
            seen_at: BTreeMap::new(),
            main: None,
            rest_order: vec![],
            move_memory: BTreeMap::new(),
            ping_pending: BTreeMap::new(),
            last_procs: vec![],
            last_rows: vec![],
            arrangement: Arrangement::default(),
            arrangement_ids: vec![],
            park_origin: BTreeMap::new(),
            pending_restore: BTreeMap::new(),
            main_number: None,
            peek: false,
            peek_frozen: false,
            click_monitor: None,
            focus_back: false,
            focus_quiet: false,
            quiet_gen: 0,
            observers: vec![],
        }
    }

    /// Swift `applicationDidFinishLaunching`.
    fn start(&mut self) {
        let mtm = self.mtm;
        self.status_item.setMenu(Some(&self.menu));
        if let Some(button) = self.status_item.button(mtm) {
            button.setImage(Some(&self.tile_empty));
            button.setImagePosition(NSCellImagePosition::ImageOnly);
        }
        log(&format!(
            "--- Lookout gestartet (ziel={} anordnung={} raster={} ausgeschlossen={} poll={}s sprache={} claude-ordner={}) ---",
            self.choice.raw(),
            self.mode.raw(),
            self.grid.key(),
            self.excluded.len(),
            self.cfg.poll_sec,
            if is_german() { "de" } else { "en" },
            self.cfg.claude_config_dirs.len()
        ));

        let ready = color_from_hex(&self.cfg.glow_color);
        let waiting = color_from_hex(&self.cfg.glow_color_waiting);
        self.glow.set_tints(ready, waiting);
        self.labels.set_tints(ready, waiting);
        self.lists.set_tints(ready, waiting);
        screens::refresh_metrics(mtm);
        self.observe();

        self.trusted = ax::trusted(true);
        if !self.trusted {
            log("WARNUNG: keine Bedienungshilfen-Berechtigung");
        }
        if self.cfg.hotkeys {
            // Ctrl+Opt+4 and 5 only with the parking lot (worker names): a registration without a task swallows the
            // combination system-wide.
            let digits: &[u8] = if self.has_lot() {
                &[1, 2, 3, 4, 5]
            } else {
                &[1, 2, 3]
            };
            hotkeys::install(digits, Box::new(|i| with_app(move |a| a.hotkey(i))));
        }
        let block = RcBlock::new(|_: NonNull<NSTimer>| with_app(App::tick));
        // SAFETY: the timer runs on the main run loop; the block only reaches the app through the main-thread slot.
        self.timer = Some(unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(
                self.cfg.poll_sec as f64,
                true,
                &block,
            )
        });
        self.tick();
    }

    /// Screen changes, space changes and app switches (Swift: NotificationCenter observers in
    /// `applicationDidFinishLaunching`).
    fn observe(&mut self) {
        let queue = NSOperationQueue::mainQueue();
        let screens = RcBlock::new(|_: NonNull<NSNotification>| {
            with_app(|a| {
                screens::refresh_metrics(a.mtm);
                a.apply_layout();
            })
        });
        // The glow hangs on the visible space: else frames stood over strangers' windows until the next tick.
        let space = RcBlock::new(|_: NonNull<NSNotification>| with_app(App::refresh_overlays));
        // An activated terminal lifts its windows ABOVE the glow panels: back on them at once, and once more shortly
        // after, since macOS finishes the lift only after the report.
        let activated = RcBlock::new(|_: NonNull<NSNotification>| {
            with_app(App::refresh_overlays);
            on_main_after(0.2, || with_app(App::refresh_overlays));
        });
        let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
        // SAFETY: observers on the main queue; the blocks reach the app only through the main-thread slot. The
        // notification names are AppKit's constants.
        unsafe {
            self.observers.push(
                NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                    Some(NSApplicationDidChangeScreenParametersNotification),
                    None,
                    Some(&queue),
                    &screens,
                ),
            );
            self.observers
                .push(workspace.addObserverForName_object_queue_usingBlock(
                    Some(NSWorkspaceActiveSpaceDidChangeNotification),
                    None,
                    Some(&queue),
                    &space,
                ));
            self.observers
                .push(workspace.addObserverForName_object_queue_usingBlock(
                    Some(NSWorkspaceDidActivateApplicationNotification),
                    None,
                    Some(&queue),
                    &activated,
                ));
        }
    }

    // ---------- Tick ----------

    pub(crate) fn tick(&mut self) {
        if self.tick_in_flight {
            return;
        }
        self.tick_in_flight = true;
        let input = TickInput {
            refresh: now() - self.last_session_fetch >= self.cfg.session_poll_sec as f64,
            cached_sessions: self.sessions.clone(),
            first_seen: self.first_seen.clone(),
            threshold: self.cfg.ready_threshold_sec,
            pid_windows: self.pid_windows.clone(),
            pending: self.ping_pending.clone(),
            known_procs: self.last_procs.clone(),
            known_rows: self.last_rows.clone(),
            epoch: self.move_epoch,
            seen: self.seen_at.clone(),
            shown: self.windows.clone(),
            front_pid: frontmost_pid(),
            screen_h: screens::primary_height(),
            other_window_names: self.cfg.other_window_names.clone(),
        };
        let engines = Arc::clone(&self.engines);
        work_queue().exec_async(move || {
            let out = tick_work(input, &engines);
            on_main(move || with_app(move |a| a.tick_done(out)));
        });
    }

    fn tick_done(&mut self, out: TickOutput) {
        let n = now();
        if out.refresh {
            self.last_session_fetch = n;
        }
        self.sessions = out.raw;
        self.live_sessions = out.live;
        if out.refresh {
            self.since = advance_since(&self.since, &self.live_sessions, n);
        }
        // Did Lookout move during the snapshot, it shows intermediate frames: keep the measured ones, else a window that
        // just arrived jumped back (2026-09-25).
        let mut stamped = if self.self_move || self.move_epoch != out.epoch {
            keep_measured_frames(out.windows, &self.windows)
        } else {
            out.windows
        };
        // Ready times move on BEFORE the windows are taken over.
        let mut fresh = BTreeMap::new();
        for w in stamped.iter_mut().filter(|w| w.is_ready()) {
            let since = self.ready_since.get(&w.id).copied().unwrap_or(n);
            w.ready_since = Some(since);
            fresh.insert(w.id, since);
        }
        self.ready_since = fresh;
        let had_session: BTreeSet<u64> = self
            .windows
            .iter()
            .filter(|w| !w.sessions.is_empty())
            .map(|w| w.id)
            .collect();
        self.windows = stamped;
        // Merge (a click during the tick is already in `seen_at`) and keep a week instead of filtering by the running
        // sessions: a session that drops out of the list for one tick would lose its seen answer (review quick
        // 2026-10-04, GLM).
        for (id, at) in out.seen {
            let entry = self.seen_at.entry(id).or_insert(at);
            *entry = entry.max(at);
        }
        self.seen_at.retain(|_, at| n - *at < SEEN_KEEP);
        self.pid_windows = out.pid_windows;
        self.ping_pending = out.pending;
        self.last_procs = out.procs;
        self.last_rows = out.rows;
        self.first_seen = self.windows.iter().map(|w| (w.id, w.first_seen)).collect();
        let ids: BTreeSet<u64> = self.windows.iter().map(|w| w.id).collect();
        self.move_memory.retain(|id, _| ids.contains(id));
        self.trusted = out.trusted;
        self.tick_in_flight = false;

        screens::refresh_metrics(self.mtm);
        self.update_badge();
        let owners: BTreeSet<Pid> = self.windows.iter().map(|w| w.owner_pid).collect();
        self.focus_watcher.rebuild(&owners);
        // Plan first, then draw: frames, labels and lists follow the new plan (parked, plan 1d).
        if !self.paused {
            self.apply_layout();
        }
        self.refresh_overlays();
        // After the start no active session is known yet (focus reports come only with the next change): take the
        // focused AI window. R2-9: likewise when a window just got its first session, else a new window lay parked IN
        // FRONT of the middle until Kevin clicked it.
        let first_session = self
            .windows
            .iter()
            .any(|w| !w.sessions.is_empty() && !had_session.contains(&w.id));
        if self.focused.is_none() || first_session {
            self.handle_focus_change();
        }
    }

    /// Menu bar symbol (plan 1b): top right tile filled when a session is done or asks, grey without access. Only on a
    /// change: setting image and appearance every tick cost WindowServer an error line each (2026-09-27, measured).
    fn update_badge(&mut self) {
        let asking = self.windows.iter().filter(|w| w.is_waiting()).count();
        let ready = self
            .windows
            .iter()
            .filter(|w| w.is_ready() && !w.is_waiting())
            .count();
        let badge = status_badge(ready, asking, self.trusted);
        if self.shown_badge == Some(badge) {
            return;
        }
        self.shown_badge = Some(badge);
        if let Some(button) = self.status_item.button(self.mtm) {
            button.setImage(Some(if badge.filled {
                &self.tile_full
            } else {
                &self.tile_empty
            }));
            button.setAppearsDisabled(badge.dimmed);
        }
    }

    // ---------- Focus: a click on an AI window ----------

    pub(crate) fn handle_focus_change(&mut self) {
        // Own moves are no clicks (convention 7), but a real click in that time is noted and handled after the move
        // (v4). In the pause too the active session follows: the glow belongs to the status display. During the peek
        // only the click watcher counts (Lookout's own raising moves the focus between tiles), and right after it until
        // the focus is back.
        if !self.trusted || self.peek || self.focus_quiet {
            return;
        }
        if !self.focus_gate.event(self.self_move) {
            self.log_focus("Klick während eigener Bewegung — wird danach ausgewertet");
            return;
        }
        let terminals: Vec<Pid> = self.windows.iter().map(|w| w.owner_pid).collect();
        let ai: Vec<Pid> = self
            .windows
            .iter()
            .filter(|w| !w.sessions.is_empty())
            .map(|w| w.owner_pid)
            .collect();
        let Some(pid) = focus_source(frontmost_pid(), &terminals, &ai, self.focused.is_some())
        else {
            return;
        };
        ax_queue().exec_async(move || {
            let at = now();
            let focused = ax::focused_window(pid);
            let title = focused.and_then(ax::title);
            on_main(move || {
                with_app(move |a| {
                    // A reading that started before the peek or its end does not count either.
                    if !a.peek && !a.focus_quiet {
                        a.on_focused(focused, title, Some(at));
                    }
                })
            });
        });
    }

    /// `title`/`at`: title and time of the focus reading. Only then the click counts as "answer seen" (R10; also a short
    /// look between two ticks, review quick 2026-10-04, Codex). The reading is stamped, not this later execution: an
    /// answer that came after it stays unseen. The other callers have no title.
    fn on_focused(&mut self, focused: Option<u64>, title: Option<String>, at: Option<Time>) {
        if let Some(at) = at
            && !sys::screen_locked()
            && let Some(pid) = frontmost_pid()
        {
            let on_screen = ax::on_screen_windows(Some(&BTreeSet::from([pid])));
            if let Some(id) = session_in_front(
                &self.windows,
                focused,
                title.as_deref(),
                Some(pid),
                on_screen.as_deref(),
                screens::primary_height(),
            ) {
                let entry = self.seen_at.entry(id).or_insert(at);
                *entry = entry.max(at);
            }
        }
        let Some(idx) = focused.and_then(|id| index_of_window(id, &self.windows)) else {
            return;
        };
        // Click log (v4): why a click moves nothing stands in the log instead of being guessed.
        if self.windows[idx].sessions.is_empty() {
            let line = format!(
                "Klick ignoriert: Fenster ohne erkannte Session („{}“)",
                prefix(&self.windows[idx].title, 50)
            );
            self.log_focus(&line);
            return;
        }
        // The clicked AI window is the active session: it loses its glow at once, and all glows are placed anew
        // (macOS lifted it above them).
        let id = self.windows[idx].id;
        self.focused = Some(id);
        // It becomes the main window only if Lookout also arranges it: a click into an excluded window changes nothing.
        let candidates = layout_candidates(&self.windows, &self.excluded);
        if !candidates.contains(&idx) {
            let line = format!(
                "Klick: {} — ausgeschlossen oder nicht beweglich, Anordnung bleibt",
                self.focus_name(idx)
            );
            self.log_focus(&line);
            self.refresh_overlays();
            return;
        }
        let changed = self.main != Some(id);
        if changed {
            let line = format!("Klick: {} → Mitte", self.focus_name(idx));
            self.log_focus(&line);
        }
        if changed
            && let Some(old_main) = self.main_index().or_else(|| candidates.first().copied())
            && old_main != idx
            && candidates.contains(&old_main)
        {
            // On the order that is SHOWN right now. In the control room the clicked window swaps places with the old
            // main window; in Focus 1 only among running ones (question and done follow their ready time, plan 1d).
            let shown = rest_sequence(&candidates, old_main, &self.rest_hint());
            let rest = if matches!(self.current_target(), Some(Target::ControlRoom { .. })) {
                swapped_rest(&shown, old_main, idx)
            } else {
                status_swap(&shown, old_main, idx, &self.windows)
            };
            self.rest_order = rest.iter().map(|&i| self.windows[i].id).collect();
        }
        self.main = Some(id);
        self.refresh_overlays();
        if changed && !self.paused {
            self.apply_layout();
        }
    }

    /// Name of a window for the click log: the session name as the label shows it.
    fn focus_name(&self, idx: usize) -> String {
        let w = &self.windows[idx];
        let s = w.visible_session().or(w.sessions.first());
        let name = s
            .map(|s| {
                s.digest
                    .as_ref()
                    .and_then(|d| d.name.clone())
                    .unwrap_or_else(|| s.name.clone())
            })
            .unwrap_or_else(|| w.title.clone());
        prefix(&name, 60)
    }

    /// One line per change: an app activation often reports the same focus twice.
    fn log_focus(&mut self, line: &str) {
        if line != self.last_focus_log {
            self.last_focus_log = line.to_owned();
            log(line);
        }
    }

    pub(crate) fn focused_index(&self) -> Option<usize> {
        self.focused
            .and_then(|id| index_of_window(id, &self.windows))
    }

    pub(crate) fn main_index(&self) -> Option<usize> {
        self.main.and_then(|id| index_of_window(id, &self.windows))
    }

    fn rest_hint(&self) -> Vec<usize> {
        self.rest_order
            .iter()
            .filter_map(|&id| index_of_window(id, &self.windows))
            .collect()
    }

    /// Index of the last plan -> index in the current snapshot.
    fn current_index(&self, planned: usize) -> Option<usize> {
        self.arrangement_ids
            .get(planned)
            .and_then(|&id| index_of_window(id, &self.windows))
    }

    /// Parked windows of the last plan (plan 1d, the parking lot included unless peeked at): never glow or label.
    pub(crate) fn parked_indices(&self) -> BTreeSet<usize> {
        self.arrangement
            .parked
            .iter()
            .filter_map(|&i| self.current_index(i))
            .collect()
    }

    /// Windows on the parking lot in the last plan (parked or tiled while peeking).
    pub(crate) fn lot_indices(&self) -> BTreeSet<usize> {
        self.arrangement
            .lot
            .iter()
            .filter_map(|&i| self.current_index(i))
            .collect()
    }

    /// Only with worker names (`workerNames`): without them nothing changes, Ctrl+Opt+4 stays free (plan).
    pub(crate) fn has_lot(&self) -> bool {
        self.cfg.worker_names.iter().any(|n| !n.is_empty())
    }

    // ---------- Overlays ----------

    /// The session's own Lookout line per session id, for labels, lists and menu lines.
    pub(crate) fn lookout_lines(&self) -> BTreeMap<String, LookoutLine> {
        self.windows
            .iter()
            .flat_map(|w| &w.sessions)
            .filter_map(|s| {
                let line = s.digest.as_ref()?.line(&self.cfg.other_window_names)?;
                Some((s.id.clone(), line))
            })
            .collect()
    }

    /// Glows, labels and lists: the same triggers (tick, focus, app and space switches, after moves).
    pub(crate) fn refresh_overlays(&mut self) {
        if !self.trusted {
            self.glow.hide_all();
            self.labels.hide_all();
            self.lists.hide_all();
            return;
        }
        // One window list for both: all apps (does a stranger's window lie between panel and terminal? `place`), from
        // it the terminal windows (where is what?).
        let stack = ax::on_screen_windows(None);
        let terminals: BTreeSet<Pid> = self.windows.iter().map(|w| w.owner_pid).collect();
        let on_screen: Option<Vec<OnScreenWindow>> = stack.as_ref().map(|s| {
            s.iter()
                .filter(|o| terminals.contains(&o.pid))
                .cloned()
                .collect()
        });
        let parked = self.parked_indices();
        let screens = screens::visible_frames(self.mtm);
        let lookout = self.lookout_lines();
        match self.list_targets(on_screen.as_deref(), &lookout) {
            Some(targets) => self.lists.update(&targets, stack.as_deref()),
            None => self.lists.hide_all(),
        }
        let mut input = OverlayInput::new(
            &self.windows,
            on_screen.as_deref(),
            screens::primary_height(),
            &screens,
        );
        input.focused = self.focused_index();
        input.parked = &parked;

        if self.labels_on {
            let targets: Vec<_> = label_targets(&input, &self.since, &lookout, now())
                .into_iter()
                .filter(|t| !self.moving_ids.contains(&t.id))
                .collect();
            // Only the set, not every text change (the content changes all the time).
            let mut ids: Vec<&str> = targets.iter().map(|t| suffix(&t.id, 8)).collect();
            ids.sort_unstable();
            let line = ids.join(" ");
            if line != self.last_label_log {
                log(&format!(
                    "Schilder: {}",
                    if line.is_empty() { "keine" } else { &line }
                ));
                self.last_label_log = line;
            }
            self.labels.update(&targets, stack.as_deref());
        } else {
            self.labels.hide_all();
        }
        if !self.cfg.glow {
            self.glow.hide_all();
            return;
        }
        // Glows of gliding windows ride along (`follow`): neither moved nor hidden here.
        let targets: Vec<GlowTarget> = glow_targets(&input)
            .into_iter()
            .filter(|t| !self.moving_ids.contains(&t.id))
            .collect();
        // One line per change: the stray frame of 18.09. (design v3, bug 4) can be traced in the log next time.
        let line = targets
            .iter()
            .map(|t| {
                format!(
                    "{}@{},{} {}x{}{}",
                    suffix(&t.id, 8),
                    t.rect.min_x() as i64,
                    t.rect.min_y() as i64,
                    t.rect.width() as i64,
                    t.rect.height() as i64,
                    t.window_number
                        .map_or_else(|| "#?".to_owned(), |n| format!("#{n}"))
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        if line != self.last_glow_log {
            log(&format!(
                "Rahmen: {}",
                if line.is_empty() { "keine" } else { &line }
            ));
            self.last_glow_log = line;
        }
        self.glow
            .update(&targets, &self.moving_ids, stack.as_deref());
    }

    /// Lists at the foot of the sides (plan 1d): only with labels (they show names and steps; off while sharing the
    /// screen), never during own moves, only on the middle's desktop and right above it (GP 6, R2-10). `None` = hide.
    fn list_targets(
        &mut self,
        on_screen: Option<&[OnScreenWindow]>,
        lookout: &BTreeMap<String, LookoutLine>,
    ) -> Option<Vec<lookout_core::overlay::ListTarget>> {
        if !self.labels_on
            || self.self_move
            || !self.moving_ids.is_empty()
            || self.arrangement.lists.is_empty()
        {
            return None;
        }
        let number = if self.peek {
            self.peek_anchor(on_screen?)?
        } else {
            let main = self.arrangement.main.and_then(|i| self.current_index(i))?;
            self.list_anchor(main, on_screen)?
        };
        let n = now();
        Some(
            self.arrangement
                .lists
                .iter()
                .map(|l| {
                    list_target(
                        l,
                        &self.arrangement_ids,
                        &self.windows,
                        &self.since,
                        lookout,
                        n,
                        number,
                    )
                })
                .collect(),
        )
    }

    /// The peek's list lies over the frontmost tile: the tiles were lifted above the middle and the side windows, a list
    /// over the middle would lie under them. `None` = no tile on this desktop.
    fn peek_anchor(&self, on_screen: &[OnScreenWindow]) -> Option<i64> {
        let h = screens::primary_height();
        let tiles: BTreeSet<i64> = self
            .arrangement
            .lot
            .iter()
            .filter(|i| !self.arrangement.parked.contains(i))
            .filter_map(|&i| self.current_index(i))
            .filter_map(
                |i| match locate_on_screen(&self.windows[i], Some(on_screen), h) {
                    OnScreenMatch::Unique(n) => Some(n),
                    _ => None,
                },
            )
            .collect();
        on_screen
            .iter()
            .map(|o| o.number)
            .find(|n| tiles.contains(n))
    }

    /// Window number of the middle (R2-1): unique, or, when parked windows lie congruent behind it, the frontmost of
    /// that app on its frame. Window list unreadable: the last unique one. `None` = the middle is not on this desktop:
    /// no list, instead of one floating above everything.
    fn list_anchor(&mut self, main: usize, on_screen: Option<&[OnScreenWindow]>) -> Option<i64> {
        let w = &self.windows[main];
        let h = screens::primary_height();
        let Some(on_screen) = on_screen else {
            return self
                .main_number
                .filter(|(id, _)| *id == w.id)
                .map(|(_, n)| n);
        };
        match locate_on_screen(w, Some(on_screen), h) {
            OnScreenMatch::Unique(n) => {
                self.main_number = Some((w.id, n));
                Some(n)
            }
            OnScreenMatch::Ambiguous => frontmost_number(w, on_screen, h),
            OnScreenMatch::Absent | OnScreenMatch::Unknown => None,
        }
    }

    // ---------- Target (which screen) ----------

    /// Attached screens with id and name (Kevin 2026-10-01: every screen recognized and selectable).
    pub(crate) fn screen_infos(&self) -> Vec<ScreenInfo> {
        unique_screen_ids(&screens::infos(self.mtm))
    }

    pub(crate) fn current_target(&self) -> Option<Target> {
        resolve_target(&self.choice, &self.screen_infos(), self.pair.as_ref())
    }

    /// Plan for a choice, also for one only HOVERED (preview), hence the parameters. `peek`: with the parking lot tiled.
    pub(crate) fn plan(
        &self,
        c: &TargetChoice,
        m: LayoutMode,
        g: Option<GridSpec>,
        peek: bool,
    ) -> Option<(Target, Arrangement)> {
        let infos = self.screen_infos();
        let target = resolve_target(c, &infos, self.pair.as_ref())?;
        let screens: Vec<Rect> = infos.iter().map(|s| s.visible).collect();
        let hint = self.rest_hint();
        let mut p = PlanInput::new(m, target, &self.windows, &screens, self.side);
        p.focused = self.main_index();
        p.rest_hint = &hint;
        p.grid = g.unwrap_or(self.grid);
        p.excluded = &self.excluded;
        p.hand_parked = &self.hand_parked;
        p.hand_kept = &self.hand_kept;
        p.worker_names = &self.cfg.worker_names;
        p.lead_names = &self.cfg.lead_names;
        p.peek = peek;
        Some((target, plan_arrangement(&p)))
    }

    /// One line per change of target or arrangement, so the behaviour can be checked afterwards.
    fn log_plan(&mut self, target: &Target, arr: &Arrangement) {
        let line = plan_log_line(&self.choice, target, self.mode, arr);
        if line != self.last_plan_log {
            log(&line);
            self.last_plan_log = line;
        }
    }

    // ---------- Arranging ----------

    pub(crate) fn apply_layout(&mut self) {
        // The pause moves nothing, also not on a screen change (review quick 2026-11-06; Swift arranged there too).
        if self.paused {
            return;
        }
        // During an own ride the remembered frames are wrong: note it and arrange afterwards, else a window jumped back
        // to its start and glided anew (GLM + Kimi 2026-09-25).
        if !self.layout_gate.event(self.self_move) {
            return;
        }
        // The open peek stands still (Kevin 2026-11-06 evening): its tiles were placed once, nothing moves until it ends.
        // Are its windows all gone, it ends: else it waited without tiles, focus reports ignored (review quick
        // 2026-11-06).
        if self.peek && self.peek_frozen && self.lot_indices().is_empty() {
            self.end_peek(PeekEnd::Only);
        }
        if !self.trusted || self.windows.is_empty() || (self.peek && self.peek_frozen) {
            return;
        }
        let Some((target, arr)) = self.plan(&self.choice.clone(), self.mode, None, self.peek)
        else {
            return;
        };
        self.log_plan(&target, &arr);
        self.remember_parking(&arr);
        // An empty parking lot ends the peek (plan); the plan is the same with or without it then.
        if self.peek && arr.lot.is_empty() {
            self.end_peek(PeekEnd::Only);
        }
        self.peek_frozen = self.peek;
        self.arrangement = arr;
        self.arrangement_ids = self.windows.iter().map(|w| w.id).collect();
        let arr = &self.arrangement;
        // Dirty check against the MEASURED frame (convention 4), and no new try for a target this window already
        // missed (`needs_move`).
        let mut moves: Vec<WindowMove> = arr
            .frames
            .iter()
            .filter_map(|(&idx, &dest)| {
                let w = self.windows.get(idx)?;
                needs_move(w.frame, dest, self.move_memory.get(&w.id)).then(|| WindowMove {
                    id: w.id,
                    origin: ax_origin(&w.bundle_id),
                    from: w.frame,
                    to: dest,
                })
            })
            .collect();
        // R2-7: a parked window Kevin just excluded rides once to its last visible place, in the same ride (a second one
        // replaced the first). Only while it is excluded: taken back in, it has a planned place; two targets in one
        // ride made it twitch (review quick 2026-09-26, Kimi).
        for (id, frame) in std::mem::take(&mut self.pending_restore) {
            if let Some(idx) = index_of_window(id, &self.windows)
                && !arr.frames.contains_key(&idx)
                && frames_differ(
                    self.windows[idx].frame,
                    frame,
                    lookout_core::motion::FRAME_TOLERANCE,
                )
            {
                let w = &self.windows[idx];
                moves.push(WindowMove {
                    id,
                    origin: ax_origin(&w.bundle_id),
                    from: w.frame,
                    to: frame,
                });
            }
        }
        let focus_back = std::mem::take(&mut self.focus_back);
        let main_id = arr
            .main
            .and_then(|i| self.windows.get(i))
            .map(|w| (w.id, w.owner_pid));
        if moves.is_empty() {
            if focus_back && let Some((id, pid)) = main_id {
                self.focus_window(id, pid);
            }
            return;
        }
        // Lookout only moves, it never sends a window back (GP 2): if this ride parks a window, it may lie IN FRONT of the
        // middle afterwards. So lift the middle in the terminal at the end (AXRaise orders only within one app, R2-6).
        let parks = moves.iter().any(|m| {
            arr.parked
                .iter()
                .any(|&i| self.windows.get(i).is_some_and(|w| w.id == m.id))
        });
        let raise = if parks || focus_back { main_id } else { None };
        // Peek: the tiles come to the front after the ride (plan: "vorn, auch über Chrome").
        let tiles: Vec<(u64, Pid)> = if self.peek {
            arr.lot
                .iter()
                .filter(|i| !arr.parked.contains(i))
                .filter_map(|&i| self.windows.get(i).map(|w| (w.id, w.owner_pid)))
                .collect()
        } else {
            vec![]
        };
        self.moving_ids = moves
            .iter()
            .filter_map(|m| {
                let w = self.windows.iter().find(|w| w.id == m.id)?;
                Some(w.sessions.first()?.id.clone())
            })
            .collect();
        self.refresh_overlays();
        self.self_move = true;
        self.move_epoch += 1;
        let targets: Vec<(u64, Rect)> = moves.iter().map(|m| (m.id, m.to)).collect();
        self.animator.run(
            moves,
            screens::primary_height(),
            |steps| on_main(move || with_app(move |a| a.follow_glide(&steps))),
            move |measured| {
                // AX queue: lift the middle (parking) and the tiles (peek), then give the focus back if asked.
                if let Some((id, _)) = raise {
                    ax::raise(id);
                }
                let mut refused = vec![];
                for &(id, _) in &tiles {
                    ax::raise(id);
                    ax::set_main(id);
                }
                let mut apps: Vec<Pid> = tiles.iter().map(|&(_, pid)| pid).collect();
                apps.dedup();
                if focus_back && let Some((id, pid)) = raise {
                    ax::set_main(id);
                    apps = vec![pid];
                }
                for pid in apps {
                    if !ax::set_app_frontmost(pid) {
                        refused.push(pid);
                    }
                }
                on_main(move || {
                    with_app(move |a| a.move_finished(measured, &targets, &refused, focus_back))
                });
            },
        );
    }

    /// Focus reports wait (`focus_quiet`) from now on. A ride replaced by another never reports its end: they count again
    /// after 3 s at the latest.
    fn start_focus_quiet(&mut self) {
        self.focus_quiet = true;
        self.quiet_gen += 1;
        end_focus_quiet_after(self.quiet_gen, 3.0);
    }

    /// The ride is over: take the MEASURED frames, not the targets (a clamped window would otherwise glow with a wrong
    /// frame and the next dirty check would compare against a lie).
    fn move_finished(
        &mut self,
        measured: Vec<(u64, Rect)>,
        targets: &[(u64, Rect)],
        refused: &[Pid],
        focus_back: bool,
    ) {
        self.self_move = false;
        self.move_epoch += 1;
        if focus_back {
            end_focus_quiet_after(self.quiet_gen, 0.5);
        }
        for (id, frame) in measured {
            if let Some(idx) = index_of_window(id, &self.windows) {
                self.windows[idx].frame = frame;
            }
            self.move_memory.remove(&id);
            if let Some(&(_, target)) = targets.iter().find(|(t, _)| *t == id)
                && frames_differ(frame, target, lookout_core::motion::FRAME_TOLERANCE)
            {
                self.move_memory.insert(
                    id,
                    MoveMemory {
                        target,
                        reached: frame,
                    },
                );
            }
        }
        self.activate_refused(refused);
        self.moving_ids.clear();
        self.refresh_overlays();
        // A click during the ride was noted: handle it now (v4); likewise a noted arranging.
        if self.focus_gate.move_ended() {
            self.handle_focus_change();
        }
        if self.layout_gate.move_ended() {
            self.apply_layout();
        }
    }

    /// `activate` as the fallback where AX could not bring the terminal to the front (since macOS 14 macOS may refuse
    /// a menu bar app to activate another app [Modellwissen, Swift comment]).
    fn activate_refused(&self, refused: &[Pid]) {
        for &pid in refused {
            log("Nach vorn: Terminal per AX nicht nach vorn — activate als Rueckfall");
            if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
                app.activateWithOptions(NSApplicationActivationOptions::empty());
            }
        }
    }

    /// A window to the front with focus, without a ride (the peek ended and nothing had to move).
    fn focus_window(&self, id: u64, pid: Pid) {
        let quiet = self.quiet_gen;
        ax_queue().exec_async(move || {
            ax::raise(id);
            ax::set_main(id);
            let refused = if ax::set_app_frontmost(pid) {
                vec![]
            } else {
                vec![pid]
            };
            on_main(move || with_app(move |a| a.activate_refused(&refused)));
            end_focus_quiet_after(quiet, 0.5);
        });
    }

    /// R2-7: where did a window stand before Lookout parked it? Remembered when it enters the role "parked" (frame
    /// before the move), only if it did not already lie on the middle. Whoever is no longer parked drops out. The
    /// parking lot counts as parked (Kevin 2026-11-06), also while peeked at. Call it BEFORE the new plan is taken
    /// over: `parked_indices` means the old one here.
    fn remember_parking(&mut self, arr: &Arrangement) {
        let was: BTreeSet<usize> = self
            .parked_indices()
            .union(&self.lot_indices())
            .copied()
            .collect();
        let main_rect = arr.main.and_then(|m| arr.frames.get(&m)).copied();
        let old = std::mem::take(&mut self.park_origin);
        for &i in arr.parked.union(&arr.lot) {
            let w = &self.windows[i];
            if let Some(&kept) = old.get(&w.id) {
                self.park_origin.insert(w.id, kept);
            } else if !was.contains(&i)
                && let Some(main_rect) = main_rect
                && frames_differ(w.frame, main_rect, lookout_core::motion::FRAME_TOLERANCE)
            {
                self.park_origin.insert(w.id, w.frame);
            }
        }
    }

    /// Frames of gliding windows on their intermediate place, every frame of the ride (Kevin 2026-09-25).
    fn follow_glide(&mut self, steps: &[(u64, Rect)]) {
        if self.moving_ids.is_empty() {
            return; // the ride ended or was cancelled: nothing to move any more
        }
        let rects: BTreeMap<String, Rect> = steps
            .iter()
            .filter_map(|&(id, rect)| {
                let w = self.windows.iter().find(|w| w.id == id)?;
                Some((w.sessions.first()?.id.clone(), rect))
            })
            .collect();
        self.glow.follow(&rects);
    }

    // ---------- Peek at the parking lot (Kevin 2026-11-06) ----------

    /// Ctrl+Opt+4 or the menu: on shows the lot's windows as tiles on the largest screen, in front, with labels, and
    /// then stands still until it ends (`peek_frozen`); off brings them back behind the middle and Kevin's session to
    /// the front with focus. Not in the pause (it moves windows) and not with an empty lot.
    pub(crate) fn toggle_peek(&mut self) {
        if !self.has_lot() {
            return;
        }
        if self.peek {
            self.end_peek(PeekEnd::Back);
            return;
        }
        let n = self.lot_indices().len();
        if self.paused || n == 0 || !self.trusted {
            return;
        }
        self.peek = true;
        self.peek_frozen = false;
        log(&format!("Blick an ({n})"));
        // Kevin's clicks; Lookout's own raising is no click (it reaches only the focus watcher, ignored meanwhile).
        self.click_monitor = Some(mouse::watch_clicks(
            self.mtm,
            Box::new(|p| with_app(move |a| a.peek_click(p))),
        ));
        self.apply_layout();
    }

    /// Ends the peek. `Back` (shortcut, menu) and `Into` (a click on a tile, Kevin 2026-11-06 evening: "sofort in die
    /// Mitte") arrange again and give the focus to the middle after the ride, the clicked worker being the middle with
    /// `Into`; focus reports wait meanwhile. `Only` (pause, empty lot, a click outside the lot, a session from the menu)
    /// ends the state; the caller arranges.
    fn end_peek(&mut self, how: PeekEnd) {
        if !self.peek {
            return;
        }
        self.peek = false;
        self.peek_frozen = false;
        self.click_monitor = None;
        log("Blick aus");
        if how == PeekEnd::Only {
            return;
        }
        self.focus_back = true;
        self.start_focus_quiet();
        match how {
            // The normal click path: it becomes the main window, the old one swaps out; `apply_layout` follows.
            PeekEnd::Into(id) if self.main != Some(id) => self.on_focused(Some(id), None, None),
            _ => self.apply_layout(),
        }
    }

    /// A row of a list: during the peek like a click on a tile (the worker into the middle), else to the front (R2-4).
    fn list_clicked(&mut self, ids: &[String]) {
        let window = self
            .windows
            .iter()
            .find(|w| w.sessions.iter().any(|s| ids.contains(&s.id)))
            .map(|w| w.id);
        match window {
            Some(id) if self.peek => self.end_peek(PeekEnd::Into(id)),
            _ => self.bring_to_front(&ids.iter().cloned().collect()),
        }
    }

    /// A click during the peek (click watcher, Cocoa screen point): on a tile = that worker into the middle at once; anywhere
    /// else = a click into a window outside the lot, which ends the peek and counts as that click (plan).
    fn peek_click(&mut self, p: Point) {
        if !self.peek {
            return;
        }
        // Another app's window under the click took it, even over a tile: Kevin switched to Chrome (review quick
        // 2026-11-06). Front to back, AX coordinates; Lookout's own panels are click-through.
        let own = std::process::id() as Pid;
        let terminals: BTreeSet<Pid> = self.windows.iter().map(|w| w.owner_pid).collect();
        let y = screens::primary_height() - p.y;
        let elsewhere = ax::on_screen_windows(None)
            .and_then(|list| {
                list.into_iter().find(|w| {
                    w.pid != own
                        && p.x >= w.rect.min_x()
                        && p.x < w.rect.max_x()
                        && y >= w.rect.min_y()
                        && y < w.rect.max_y()
                })
            })
            .is_some_and(|w| !terminals.contains(&w.pid));
        // Terminal windows count by the planned tiles, not their frames: during the ride onto the tiles the frames are
        // still the old ones, and parked windows share the middle's frame (a click there picked some worker).
        let arr = &self.arrangement;
        let tile = arr
            .lot
            .iter()
            .filter(|i| !elsewhere && !arr.parked.contains(i))
            .find(|i| {
                arr.frames.get(i).is_some_and(|r| {
                    p.x >= r.min_x() && p.x < r.max_x() && p.y >= r.min_y() && p.y < r.max_y()
                })
            })
            .and_then(|&i| self.current_index(i));
        match tile {
            Some(i) => self.end_peek(PeekEnd::Into(self.windows[i].id)),
            None => {
                // That click counts: read the focus now (its report may have come while the peek still ignored it, or
                // comes next), so the clicked window becomes the middle. Arranging at once raised the old middle after
                // the ride, and it took the focus back (review quick 2026-11-06). Another app or no AI window: arrange
                // a moment later.
                self.end_peek(PeekEnd::Only);
                self.handle_focus_change();
                on_main_after(0.3, || with_app(App::apply_layout));
            }
        }
    }

    // ---------- Actions ----------

    /// Ctrl+Opt+1…3 = arrangement, Ctrl+Opt+4 = peek, Ctrl+Opt+5 = this window to the parking lot and back. Both come
    /// BEFORE the control-room lock: they work there too.
    fn hotkey(&mut self, idx: usize) {
        if idx == 3 {
            self.toggle_peek();
            return;
        }
        if idx == 4 {
            // The window Kevin is in; not during the peek (focus reports wait then, so that window is not known).
            let ids = self.focused_index().filter(|_| !self.peek).map(|i| {
                self.windows[i]
                    .sessions
                    .iter()
                    .map(|s| s.id.clone())
                    .collect::<Vec<_>>()
            });
            match ids {
                Some(ids) => self.toggle_hand_park(&ids),
                None => log("Parkplatz von Hand: kein KI-Fenster im Fokus"),
            }
            return;
        }
        let Some(&m) = LayoutMode::ALL.get(idx) else {
            return;
        };
        // In the control room the arrangement is fixed (the menu locks it too); else the shortcut changed the stored
        // arrangement invisibly (review quick).
        if matches!(self.current_target(), Some(Target::ControlRoom { .. })) {
            log("Tastenkuerzel ignoriert: im Leitstand ist die Anordnung fest");
            return;
        }
        self.set_mode(m, "Tastenkuerzel");
    }

    pub(crate) fn request_trust(&mut self) {
        ax::trusted(true);
    }

    pub(crate) fn select_target(&mut self, mut t: TargetChoice) {
        // The same screen as the old choice: its id stays ("X#2" instead of the short "X" while the twin is unplugged;
        // review quick 2026-10-01 round 2).
        if let (TargetChoice::Screen(new), TargetChoice::Screen(old)) = (&t, &self.choice)
            && lookout_core::layout::same_screen(new, old, &self.screen_infos())
        {
            t = self.choice.clone();
        }
        prefs::set_string(DEFAULTS_TARGET, &t.raw());
        log(&format!("Ziel gewechselt: {}", t.raw()));
        self.choice = t;
        if !self.paused {
            self.apply_layout();
        }
    }

    pub(crate) fn set_mode(&mut self, m: LayoutMode, via: &str) {
        self.mode = m;
        prefs::set_string(DEFAULTS_MODE, m.raw());
        let grid = if m == LayoutMode::Grid {
            format!(" {}", self.grid.key())
        } else {
            String::new()
        };
        log(&format!("Anordnung gewechselt ({via}): {}{grid}", m.raw()));
        if !self.paused {
            self.apply_layout();
        }
        self.refresh_overlays();
    }

    pub(crate) fn select_grid(&mut self, g: GridSpec) {
        self.grid = g;
        prefs::set_string(DEFAULTS_GRID, &g.key());
        self.set_mode(LayoutMode::Grid, "Menue");
    }

    /// Control-room screen chosen (work or control; the same screen for both roles swaps them) or back to automatic.
    /// Whoever chooses control-room screens wants the control room: "all on …" becomes control room 1.
    pub(crate) fn select_pair(&mut self, key: menu::PairKey) {
        match &key {
            menu::PairKey::Auto => self.pair = None,
            menu::PairKey::Work(id) | menu::PairKey::Control(id) => {
                let infos = self.screen_infos();
                let Some(eff) = self.effective_pair(&infos) else {
                    return;
                };
                // The stored pair, if it applies, keeps its ids (twin "X#2", review quick 2026-10-01 round 2, GLM).
                let mut current = Pair {
                    work: infos[eff.work].id.clone(),
                    control: infos[eff.control].id.clone(),
                };
                if let Some(p) = &self.pair
                    && lookout_core::layout::active_pair(Some(p), &infos).is_some()
                {
                    current = p.clone();
                }
                self.pair = Some(if matches!(key, menu::PairKey::Work(_)) {
                    lookout_core::layout::choose_pair(&current, Some(id), None, &infos)
                } else {
                    lookout_core::layout::choose_pair(&current, None, Some(id), &infos)
                });
                if !matches!(
                    self.choice,
                    TargetChoice::Both | TargetChoice::Both2 | TargetChoice::Both3
                ) {
                    self.choice = TargetChoice::Both;
                    prefs::set_string(DEFAULTS_TARGET, &self.choice.raw());
                }
            }
        }
        match &self.pair {
            Some(p) => {
                prefs::set_string(DEFAULTS_PAIR_WORK, &p.work);
                prefs::set_string(DEFAULTS_PAIR_CONTROL, &p.control);
            }
            None => {
                prefs::remove(DEFAULTS_PAIR_WORK);
                prefs::remove(DEFAULTS_PAIR_CONTROL);
            }
        }
        let which = self.pair.as_ref().map_or_else(
            || "automatisch".to_owned(),
            |p| format!("Arbeit {}, Leitstand {}", p.work, p.control),
        );
        log(&format!(
            "Leitstand-Schirme: {which}, ziel={}",
            self.choice.raw()
        ));
        if !self.paused {
            self.apply_layout();
        }
    }

    /// Control-room screens that apply now (chosen or automatic), for the current control-room variant or control room 1.
    pub(crate) fn effective_pair(&self, infos: &[ScreenInfo]) -> Option<Pair<usize>> {
        let variant = match self.choice {
            TargetChoice::Both | TargetChoice::Both2 | TargetChoice::Both3 => self.choice.clone(),
            _ => TargetChoice::Both,
        };
        match resolve_target(&variant, infos, self.pair.as_ref())? {
            Target::ControlRoom { work, control, .. } => Some(Pair { work, control }),
            Target::Single(_) => None,
        }
    }

    /// Takes a window out of the arrangement or back in, remembered per session id. Its glow stays (Kevin 2026-09-18).
    pub(crate) fn toggle_excluded(&mut self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let was_out = ids.iter().any(|id| self.excluded.contains(id));
        for id in ids {
            if was_out {
                self.excluded.remove(id);
            } else {
                self.excluded.insert(id.clone());
            }
        }
        prefs::set_string_array(
            DEFAULTS_EXCLUDED,
            &self.excluded.iter().cloned().collect::<Vec<_>>(),
        );
        log(&format!(
            "Ausschliessen: {} -> {}",
            ids.join(","),
            if was_out { "wieder drin" } else { "raus" }
        ));
        // R2-7: was it parked (or on the parking lot), it rides once to its last visible place with the next
        // arrangement; was it never visible, it stays (menu: "ausgeschlossen, hinter der Mitte").
        if !was_out
            && let Some(idx) = self
                .windows
                .iter()
                .position(|w| w.sessions.iter().any(|s| ids.contains(&s.id)))
            && (self.parked_indices().contains(&idx) || self.lot_indices().contains(&idx))
            && let Some(&origin) = self.park_origin.get(&self.windows[idx].id)
        {
            self.pending_restore.insert(self.windows[idx].id, origin);
        }
        if !self.paused {
            self.apply_layout();
        }
    }

    /// Is this window meant for the parking lot (the check mark)? A worker by name unless taken off by hand, or sent
    /// there by hand.
    pub(crate) fn lot_member(&self, idx: usize) -> bool {
        lot_member(
            &self.windows[idx],
            &self.cfg.worker_names,
            &self.cfg.lead_names,
            &self.hand_parked,
            &self.hand_kept,
        )
    }

    /// Sends a window to the parking lot by hand or takes it off (Kevin 2026-11-06 evening: Ctrl+Opt+5 in the window or
    /// the menu, for workers too), remembered per session id like the exclusions. On the lot it parks like a worker:
    /// while it runs or watches; done or asking it stands with the waiting ones. Was it the middle, it leaves at once:
    /// the most urgent other window takes its place, with the focus.
    pub(crate) fn toggle_hand_park(&mut self, ids: &[String]) {
        if ids.is_empty() || !self.has_lot() {
            return;
        }
        let idx = self
            .windows
            .iter()
            .position(|w| w.sessions.iter().any(|s| ids.contains(&s.id)));
        let was_in = idx.map_or_else(
            || ids.iter().any(|id| self.hand_parked.contains(id)),
            |i| self.lot_member(i),
        );
        let worker = idx.is_some_and(|i| {
            is_worker(
                &self.windows[i],
                &self.cfg.worker_names,
                &self.cfg.lead_names,
            )
        });
        for id in ids {
            if was_in {
                // Off: whatever put it there, by hand or by name.
                self.hand_parked.remove(id);
                if worker {
                    self.hand_kept.insert(id.clone());
                }
            } else {
                self.hand_kept.remove(id);
                if !worker {
                    self.hand_parked.insert(id.clone());
                }
            }
        }
        prefs::set_string_array(
            DEFAULTS_PARKED,
            &self.hand_parked.iter().cloned().collect::<Vec<_>>(),
        );
        prefs::set_string_array(
            DEFAULTS_KEPT,
            &self.hand_kept.iter().cloned().collect::<Vec<_>>(),
        );
        let name = idx.map_or_else(|| ids.join(","), |i| self.focus_name(i));
        log(&format!(
            "Parkplatz von Hand: {name} -> {}",
            if was_in { "zurück" } else { "rein" }
        ));
        if !was_in
            && let Some(i) = idx
            && self.main_index() == Some(i)
            && let Some(next) = self.next_main(i)
        {
            let id = self.windows[next].id;
            self.main = Some(id);
            self.focused = Some(id);
            self.focus_back = true;
            self.start_focus_quiet();
        }
        if !self.paused {
            self.apply_layout();
        }
    }

    /// Who takes the middle when it goes to the parking lot by hand: the most urgent other arranged window (question,
    /// then done, then running: `attention_order`) that does not park itself.
    fn next_main(&self, leaving: usize) -> Option<usize> {
        let lot = self.lot_indices();
        let others: Vec<usize> = layout_candidates(&self.windows, &self.excluded)
            .into_iter()
            .filter(|&i| i != leaving && !lot.contains(&i))
            .collect();
        let ws: Vec<&TermWindow> = others.iter().map(|&i| &self.windows[i]).collect();
        attention_order(&ws).first().map(|&k| others[k])
    }

    /// A click on a session in the menu or a list (plan 1c/1d, GP 4, R2-4): the window to the front, a minimized one
    /// back, raise, main window, the terminal to the front via AX (`activate` only as fallback). Then read it fresh and
    /// count it like a click into it (`on_focused`): not relying on the focus report, it checks against the snapshot
    /// in which the window is still minimized.
    pub(crate) fn bring_to_front(&mut self, ids: &BTreeSet<String>) {
        if !self.trusted {
            return;
        }
        let Some(idx) = self
            .windows
            .iter()
            .position(|w| w.sessions.iter().any(|s| ids.contains(&s.id)))
        else {
            return;
        };
        let w = self.windows[idx].clone();
        let line = format!("Nach vorn: {}", self.focus_name(idx));
        self.log_focus(&line);
        // A session chosen in the menu or a list during the peek: Kevin wants that window, the peek ends.
        self.end_peek(PeekEnd::Only);
        let h = screens::primary_height();
        ax_queue().exec_async(move || {
            if ax::minimized(w.id) == Some(true) {
                ax::unminimize(w.id);
            }
            ax::raise(w.id);
            ax::set_main(w.id);
            let front = ax::set_app_frontmost(w.owner_pid);
            let fresh = ax::reread_window(&w, h);
            on_main(move || {
                with_app(move |a| {
                    if !front {
                        a.activate_refused(&[w.owner_pid]);
                    }
                    // A tick already running brings the old snapshot (still minimized): raise the epoch like own moves,
                    // then the fresh values hold (`keep_measured_frames`).
                    a.move_epoch += 1;
                    if let Some(i) = index_of_window(w.id, &a.windows) {
                        a.windows[i].frame = fresh.frame;
                        a.windows[i].minimized = fresh.minimized;
                        a.windows[i].fullscreen = fresh.fullscreen;
                    }
                    a.on_focused(Some(w.id), None, None);
                    a.tick();
                })
            });
        });
    }

    pub(crate) fn toggle_labels(&mut self) {
        self.labels_on = !self.labels_on;
        prefs::set_bool(DEFAULTS_LABELS, self.labels_on);
        log(&format!(
            "Schilder: {}",
            if self.labels_on { "an" } else { "aus" }
        ));
        self.refresh_overlays();
    }

    pub(crate) fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        log(&format!("Pause: {}", self.paused));
        if self.paused {
            // The cancelled ride never calls its finish: reset `self_move` here, else the app ignores clicks for good.
            self.animator.cancel();
            self.self_move = false;
            // Likewise: labels come back with the next tick, which reads the real intermediate frames (review quick
            // 2026-09-21, Kimi).
            self.moving_ids.clear();
            self.layout_gate = MoveGate::default(); // pause means: no arranging afterwards
            // The peek moves windows: off in the pause (plan).
            self.end_peek(PeekEnd::Only);
            if self.focus_gate.move_ended() {
                self.handle_focus_change(); // keep a noted click (v4)
            }
        } else {
            // Read fresh first, then arrange: after a cancelled ride the windows stand somewhere in between (review
            // quick 2026-09-25, Kimi).
            self.tick();
        }
    }

    pub(crate) fn toggle_side(&mut self) {
        self.side = match self.side {
            StackSide::Right => StackSide::Left,
            StackSide::Left => StackSide::Right,
        };
        log(&format!("Streifen-Seite: {}", self.side.raw()));
        if !self.paused {
            self.apply_layout();
        }
    }

    pub(crate) fn quit(&mut self) {
        self.animator.cancel();
        hotkeys::remove();
        self.glow.hide_all();
        self.labels.hide_all();
        self.lists.hide_all();
        self.preview.hide();
        self.click_monitor = None;
        log("--- Lookout beendet ---");
        NSApplication::sharedApplication(self.mtm).terminate(None);
    }
}

// ---------- Work queue ----------

/// The tick's work (Swift: the `workQueue.async` block of `tick`): status, processes, windows, matching, ping.
fn tick_work(t: TickInput, engines: &Mutex<Engines>) -> TickOutput {
    // R10: which answer does Kevin have in front of him right now? Focus and title read fresh, BEFORE the decision
    // below, first thing after the front app (review quick 2026-10-04, Codex, GLM, Kimi). Not with a locked screen.
    let mut seen = t.seen;
    if let Some(pid) = t.front_pid
        && t.shown.iter().any(|w| w.owner_pid == pid)
        && !sys::screen_locked()
    {
        // Front check and time belong to the reading: is another app in front by now, it does not count.
        let (focused, title, at) = ax_sync(|| {
            let at = now();
            if ax::focused_app_pid() != Some(pid) {
                return (None, None, at);
            }
            let w = ax::focused_window(pid);
            (w, w.and_then(ax::title), at)
        });
        let on_screen = ax::on_screen_windows(Some(&BTreeSet::from([pid])));
        if let Some(id) = session_in_front(
            &t.shown,
            focused,
            title.as_deref(),
            Some(pid),
            on_screen.as_deref(),
            t.screen_h,
        ) {
            seen.insert(id, at);
        }
    }
    // Processes not readable: the last list holds instead of dropping every match (finding 10).
    let terminals: BTreeSet<Pid> = ax::running_terminals().iter().map(|t| t.0).collect();
    let (procs, rows) = sys::engine_processes(&terminals).unwrap_or((t.known_procs, t.known_rows));
    // Plan 1a: pinged and matched by name is only what runs under Warp/Ghostty. The full list stays for sessions and
    // the log finder, else sessions of other terminals were missing under "without window".
    let terminal_pids: Vec<Pid> = procs
        .iter()
        .filter(|p| p.in_terminal)
        .map(|p| p.pid)
        .collect();
    let raw = if t.refresh {
        let mut guard = engines.lock().unwrap_or_else(|e| e.into_inner());
        let e = &mut *guard;
        let mut raw = e.claude.sessions(now());
        raw.extend(e.finder.sessions(&procs, t.threshold));
        // Transcripts only in the session tick and only for Claude sessions with a terminal.
        for s in raw
            .iter_mut()
            .filter(|s| s.engine == Engine::Claude && s.pid.is_some())
        {
            let sid = s.id.strip_prefix("claude:").unwrap_or(&s.id).to_owned();
            let pid = s.pid;
            // A start prompt (handover from another session) is no message from Kevin.
            s.digest = e.transcripts.digest(&sid).map(|d| {
                launch_corrected(
                    d,
                    || pid.and_then(sys::process_arguments),
                    || pid.and_then(sys::process_start),
                )
            });
        }
        let alive: BTreeSet<String> = raw
            .iter()
            .map(|s| s.id.strip_prefix("claude:").unwrap_or(&s.id).to_owned())
            .collect();
        e.transcripts.forget(&alive);
        // Plan 1e: `busy`, answer finished, background resting since readyThresholdSec -> done.
        let raw = e
            .background
            .adjust(&raw, &rows, now(), t.threshold as f64, &seen);
        // Done, but waits only for another session or an own timer and says "nothing needed".
        let raw = e.standby.adjust(&raw, now(), &seen);
        // Last: asks the session in its Lookout line for something, it waits for Kevin (Kevin 2026-10-12).
        apply_lookout_lines(raw, &t.other_window_names)
    } else {
        t.cached_sessions
    };
    let live = attach_processes(&raw, &procs);

    let mut windows = ax_sync(|| ax::collect_windows(&t.first_seen, now(), t.screen_h));
    // Pairs whose process or window vanished drop out: their process is pinged anew. A settled state stays quiet.
    let mut pid_windows: BTreeMap<Pid, u64> = t
        .pid_windows
        .into_iter()
        .filter(|(pid, id)| {
            procs.iter().any(|p| p.pid == *pid) && windows.iter().any(|w| w.id == *id)
        })
        .collect();
    // v4: windows by name; unique title pairs count at once (also a moved tab). The ping stays the fallback.
    for (pid, idx) in title_matches(&live, &windows, &procs) {
        if pid_windows.get(&pid) != Some(&windows[idx].id) {
            pid_windows.insert(pid, windows[idx].id);
            log(&format!(
                "Zuordnung am Namen: {}",
                prefix(title_glyph(&windows[idx].title).1, 60)
            ));
        }
    }
    // A window the last tick did not know is perhaps a dragged-out tab: then a known pair may be wrong (design v3).
    let new_window = windows.iter().any(|w| !t.first_seen.contains_key(&w.id));
    let n = now();
    let trusted = ax::trusted(false);
    let readable = !windows.is_empty() && !sys::screen_locked();
    let mapped: BTreeSet<Pid> = pid_windows.keys().copied().collect();
    let schedule = ping_schedule(
        &terminal_pids,
        &mapped,
        &t.pending,
        new_window,
        n,
        trusted,
        readable,
    );
    let mut pending = schedule.pending.clone();
    if !schedule.due.is_empty() {
        // The full list: known pairs stay while their process lives, also when `in_terminal` flips once (review quick
        // 2026-09-26, GLM). Pinged are only the due ones anyway.
        let (map, found) = ping_map(&procs, &pid_windows, &schedule.due, t.screen_h);
        pid_windows = map;
        pending = ping_result(&schedule.due, &found, &schedule.pending, n);
        windows = ax_sync(|| ax::collect_windows(&t.first_seen, now(), t.screen_h));
    }
    match_sessions(&mut windows, &live, &pid_windows);
    TickOutput {
        refresh: t.refresh,
        raw,
        live,
        windows,
        seen,
        pid_windows,
        pending,
        procs,
        rows,
        trusted,
        epoch: t.epoch,
    }
}

/// Ends the quiet period `generation` (`focus_quiet`) after `seconds`, unless a newer one started meanwhile (review quick
/// 2026-11-06: an older timer ended a newer period early, and a late focus report of a tile could put a worker into the
/// middle). Any thread.
fn end_focus_quiet_after(generation: u64, seconds: f64) {
    on_main_after(seconds, move || {
        with_app(move |a| {
            if a.quiet_gen == generation {
                a.focus_quiet = false;
            }
        })
    });
}

/// The title ping (Mapper.swift `pingMap`, core `Ping`): a unique marker as OSC-0 title on each due process's tty, then
/// only the titles read every 10 ms for up to 150 ms (a full snapshot took longer than the marker lived), at most 4
/// rounds, titles back at once. Work queue.
fn ping_map(
    procs: &[EngineProc],
    known: &BTreeMap<Pid, u64>,
    due: &BTreeSet<Pid>,
    screen_h: f64,
) -> (BTreeMap<Pid, u64>, BTreeSet<Pid>) {
    let before = ax_sync(|| ax::collect_windows(&BTreeMap::new(), now(), screen_h));
    let mut ping = Ping::start(procs, known, &before, due);
    if ping.open.is_empty() {
        return (ping.map, BTreeSet::new());
    }
    let ids: Vec<u64> = before.iter().map(|w| w.id).collect();
    let write = |writes: Vec<lookout_core::mapping::TitleWrite>| {
        for w in writes {
            sys::write_osc_title(&w.tty, &w.title);
        }
    };
    for _ in 0..PING_ROUNDS {
        write(ping.markers());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(PING_WAIT);
        loop {
            std::thread::sleep(std::time::Duration::from_secs_f64(PING_POLL));
            let titles: Vec<String> = ax_sync(|| {
                ids.iter()
                    .map(|&id| ax::title(id).unwrap_or_default())
                    .collect()
            });
            write(ping.read(&titles));
            if ping.open.is_empty() || std::time::Instant::now() >= deadline {
                break;
            }
        }
        if ping.open.is_empty() {
            break;
        }
    }
    write(ping.finish());
    log(&ping.summary());
    (ping.map, ping.found)
}
