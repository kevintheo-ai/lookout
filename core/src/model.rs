//! Data shared by layout, session status, window mapping and the overlays.
//!
//! - Time: `Time = f64`, seconds since the UNIX epoch (Swift `Date.timeIntervalSince1970`). Swift's logic only
//!   compares dates and takes differences, which a float keeps exactly.
//! - Windows: Swift's `TermWindow.ax` (AX element) does not exist here. `TermWindow::id` is an opaque window identity
//!   from the platform layer, equal for the same window across ticks (Swift compares AX elements with `CFEqual`).
//!   Layout results refer to windows by their index in the `windows` slice, as in Swift.
//! - Sessions are identified by `Session::id` ("engine:sessionId"), stable across ticks.
//! - Sets and maps are `BTreeSet`/`BTreeMap`: deterministic order, same contents as Swift's `Set`/`Dictionary`.

use std::collections::{BTreeMap, BTreeSet};

use unicode_segmentation::UnicodeSegmentation;

use crate::geom::Rect;
use crate::i18n::tr;

pub type Time = f64;
/// `pid_t`.
pub type Pid = i32;

// ---------- Sessions (Watcher.swift) ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Engine {
    Claude,
    Codex,
    Grok,
    Kimi,
}

impl Engine {
    pub const ALL: [Engine; 4] = [Engine::Claude, Engine::Codex, Engine::Grok, Engine::Kimi];

    /// Name as the CLI binary is called (Swift `rawValue`).
    pub fn raw(self) -> &'static str {
        match self {
            Engine::Claude => "claude",
            Engine::Codex => "codex",
            Engine::Grok => "grok",
            Engine::Kimi => "kimi",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Engine> {
        Engine::ALL.into_iter().find(|e| e.raw() == raw)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    /// engine + ":" + sessionId, stable across ticks.
    pub id: String,
    pub engine: Engine,
    /// Interactive sessions only; background agents have none.
    pub pid: Option<Pid>,
    pub name: String,
    pub cwd: String,
    /// Not working right now: turn ended or waiting for Kevin.
    pub ready: bool,
    /// Asked a QUESTION and cannot go on without an answer (Kevin 2026-09-18: "das hat prio 1"). Also: its Lookout
    /// line asks Kevin for something (`applyLookoutLines`).
    pub waiting: bool,
    pub alive: bool,
    /// Start (from `claude agents --json` only): tells fresh background agents from weeks-old ones.
    pub started_at: Option<Time>,
    /// What Claude's transcript tells about the session (topic, step, answer). Claude only.
    pub digest: Option<Digest>,
    /// Answer done but waits for its own background watcher (CI, worker), another session or its own timer: does not
    /// need Kevin (Kevin 2026-09-27). `ready` stays off meanwhile.
    pub watching: bool,
    /// Claude reports `shell`: turn over, only background commands run. `busy` always means the model works (CLI
    /// 2.1.294/2.1.295, measured 2026-10-09).
    pub background_only: bool,
}

impl Session {
    /// Swift's `Session(id:engine:pid:name:cwd:ready:alive:)`: not waiting, no start, no digest, not watching, not
    /// background-only. Set the rest with struct update syntax.
    pub fn new(
        id: impl Into<String>,
        engine: Engine,
        pid: Option<Pid>,
        name: impl Into<String>,
        cwd: impl Into<String>,
        ready: bool,
        alive: bool,
    ) -> Session {
        Session {
            id: id.into(),
            engine,
            pid,
            name: name.into(),
            cwd: cwd.into(),
            ready,
            waiting: false,
            alive,
            started_at: None,
            digest: None,
            watching: false,
            background_only: false,
        }
    }
}

/// What came last in the transcript; decides what "now" means.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LastBlock {
    #[default]
    None,
    Text,
    Tool,
    Thinking,
}

/// What a Claude transcript tail says about its session. The Swift computed property `line` (the session's own
/// Lookout line, filtered by `otherWindowNames`) belongs to the status logic and is not here.
#[derive(Clone, Debug, PartialEq)]
pub struct Digest {
    /// Topic (`ai-title`, rewritten as the session goes).
    pub title: Option<String>,
    /// Own name ("custom-title": naming hook or /rename, v4 2026-09-21); goes before the topic.
    pub name: Option<String>,
    /// Last tool step in readable form (`stepLabel`).
    pub step: Option<String>,
    /// Last answer text, raw.
    pub text: Option<String>,
    pub last: LastBlock,
    pub answer_at: Option<Time>,
    pub prompt_at: Option<Time>,
    /// Open AskUserQuestion question; closed by ITS answer (tool_result for it) or a new prompt, not by the result of a
    /// tool started in parallel.
    pub question: Option<String>,
    pub question_id: Option<String>,
    /// Answer done (plan 1e): `system`/`turn_duration` came after the last answer, after the stop hooks. Every new
    /// prompt and every later answer clears it.
    pub turn_ended: bool,
    /// Swift `kevinTurn`: the user's own prompt started the running or last turn (Kevin 2026-10-01); then it is his
    /// turn and the session never counts as watching. Messages of other sessions, background notices and timers reset
    /// it. `None`: no trigger in the tail read; the transcript reader keeps the last one seen.
    pub user_turn: Option<bool>,
    /// Swift `kevinPrompt`/`kevinPromptAt`: text and time of the session's first prompt if it started the user's turn
    /// (also `None` for a message mid-turn). Listed in the start arguments shortly after process start, it was a start
    /// prompt, not a message from the user (`launchCorrected`, Kevin 2026-10-01).
    pub user_prompt: Option<String>,
    pub user_prompt_at: Option<Time>,
    /// Other live session talking with this one (Kevin 2026-09-27): a message came from it (`peer_pid` from
    /// `verifiedPeerPid`) or this session wrote to it (`SendMessage`, name only). The user's prompt no longer ends the
    /// conversation (R11, Kevin 2026-10-05).
    pub peer_pid: Option<Pid>,
    pub peer_name: Option<String>,
    /// When the other session was last proven by pid: a session started after that only carries a reused pid.
    pub peer_at: Option<Time>,
    /// All sessions this one talked with in the tail read. A collecting session waits for the next question of ANY of
    /// them (Kevin 2026-10-12).
    pub partners: BTreeSet<String>,
    /// Own timers still pending: "loop" (`ScheduleWakeup`) or cron id (`CronCreate`) -> time, `None` = recurring.
    pub wakeups: BTreeMap<String, Option<Time>>,
    /// No message from the user since the last answer text (prompt or attachment mid-turn).
    pub text_current: bool,
    /// Its "?" names an `ask_until_read_names` place and Kevin has read the answer (`mark_read_asks`, set per tick).
    pub ask_read: bool,
}

impl Default for Digest {
    fn default() -> Digest {
        Digest {
            title: None,
            name: None,
            step: None,
            text: None,
            last: LastBlock::None,
            answer_at: None,
            prompt_at: None,
            question: None,
            question_id: None,
            turn_ended: false,
            user_turn: None,
            user_prompt: None,
            user_prompt_at: None,
            peer_pid: None,
            peer_name: None,
            peer_at: None,
            partners: BTreeSet::new(),
            wakeups: BTreeMap::new(),
            text_current: true,
            ask_read: false,
        }
    }
}

/// Lookout line status: "◌ Lookout: …" needs nothing, "? Lookout: …" asks Kevin, "✓ Lookout: …" done
/// (Kevin 2026-10-12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineStatus {
    Nothing,
    Ask,
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookoutLine {
    pub status: LineStatus,
    /// Short, what for: the bracket content ("Merge #126"), else the text without status word; for the label.
    pub reason: String,
}

// ---------- Processes and the window list (Mapper.swift, Overlay.swift) ----------

/// One process (Swift read them from `ps -Ao pid=,ppid=,tty=,time=,comm=`; the Rust platform layer asks the system
/// directly): every process, not only AI CLIs.
#[derive(Clone, Debug, PartialEq)]
pub struct PsRow {
    pub pid: Pid,
    pub ppid: Pid,
    pub tty: String,
    /// CPU time (user + system) in seconds.
    pub cpu: f64,
    /// The program as started, like `ps -o comm=`: argv[0], a path or a bare name, never the further arguments (see
    /// `mapping::engine_of`). A program that rewrites its argv[0] shows the new text, in ps too.
    pub comm: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EngineProc {
    pub pid: Pid,
    pub engine: Engine,
    /// e.g. "ttys004".
    pub tty: String,
    pub cwd: String,
    /// Runs under Warp or Ghostty (plan 1a, Swift default true)? Only then can the session sit in one of their
    /// windows; only such processes are pinged and matched by name.
    pub in_terminal: bool,
}

/// A window on the VISIBLE space: CGWindowID, owning app and rectangle in AX coordinates (top-left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OnScreenWindow {
    pub number: i64,
    pub pid: Pid,
    pub rect: Rect,
}

// ---------- Title glyph: what does the visible tab show? (Mapper.swift, design 2026-09-21) ----------

/// Claude Code writes the window title itself: spinner (◐ and ◑ alternating) = working, ✳ = done or question, then
/// the topic. Measured 2026-09-21: 12 samples × 8 Warp windows, no other glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleGlyph {
    Busy,
    Idle,
    None,
}

/// Glyph and the title text without it, trimmed.
pub fn title_glyph(title: &str) -> (TitleGlyph, &str) {
    let glyph = match title.chars().next() {
        Some('\u{25D0}' | '\u{25D1}') => TitleGlyph::Busy,
        Some('\u{2733}') => TitleGlyph::Idle,
        _ => return (TitleGlyph::None, trim_ws(title)),
    };
    (glyph, trim_ws(drop_first_character(title)))
}

/// Swift `Character`s: extended grapheme clusters (UAX #29), produced lazily. "✳️" (with variation selector), a
/// family emoji joined by ZWJ or a flag pair are one character each, as Swift counts and cuts them.
pub fn graphemes(s: &str) -> impl Iterator<Item = &str> {
    s.graphemes(true)
}

/// Swift's `dropFirst()` on a string: everything after the first character.
pub fn drop_first_character(s: &str) -> &str {
    graphemes(s).next().map_or(s, |first| &s[first.len()..])
}

/// Texts a Claude session writes into the window title: own name and topic, both, because the title lags behind a
/// rename for a moment. Trimmed, empty ones left out.
pub fn title_texts(s: &Session) -> BTreeSet<String> {
    let Some(d) = &s.digest else {
        return BTreeSet::new();
    };
    [&d.name, &d.title]
        .into_iter()
        .flatten()
        .map(|t| trim_ws(t))
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

// ---------- Text helpers with Foundation's semantics ----------

/// Foundation's `CharacterSet.whitespaces` (tab, the space separators, U+200B), which Swift's
/// `trimmingCharacters(in: .whitespaces)` trims. Unlike `str::trim` it keeps line breaks.
pub fn is_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200B}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

pub fn trim_ws(s: &str) -> &str {
    s.trim_matches(is_whitespace)
}

/// Text for case-insensitive matching (Foundation `.caseInsensitive`): lowercase, "ß" as "ss" (Foundation folds
/// case fully, so "STRASSE" finds "Straße"). ponytail: no full Unicode case folding (ligatures and the like).
pub fn fold_case(s: &str) -> String {
    s.to_lowercase().replace('ß', "ss")
}

// ---------- Terminal windows (Mapper.swift) ----------

#[derive(Clone, Debug, PartialEq)]
pub struct TermWindow {
    /// Opaque window identity from the platform layer (Swift: the AX element).
    pub id: u64,
    pub app: String,
    pub bundle_id: String,
    /// Process of the terminal app (Warp/Ghostty), not of the session.
    pub owner_pid: Pid,
    /// Cocoa coordinates.
    pub frame: Rect,
    pub title: String,
    pub sessions: Vec<Session>,
    pub first_seen: Time,
    /// Since when is this window done? Orders the waiting side: whoever was done first stands on top.
    pub ready_since: Option<Time>,
    pub movable: bool,
    pub minimized: bool,
    pub fullscreen: bool,
}

/// State the window SHOWS (`TermWindow::shown_state`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShownState {
    pub ready: bool,
    pub waiting: bool,
    pub watching: bool,
}

impl TermWindow {
    /// Session in the VISIBLE tab (design 2026-09-21). One session: that one. Several (tabs, typically after
    /// /handoff): the one whose topic equals the window title without glyph; without a unique hit none. Without a
    /// Claude glyph the ONE non-Claude session is visible (Codex/Grok/Kimi write no glyph), and a topic hit only
    /// proves something if every Claude session in the window knows its topic (review quick 2026-09-21).
    pub fn visible_session(&self) -> Option<&Session> {
        let live: Vec<&Session> = self.sessions.iter().filter(|s| s.alive).collect();
        if live.len() <= 1 {
            return live.first().copied();
        }
        let (glyph, text) = title_glyph(&self.title);
        if glyph == TitleGlyph::None {
            let others: Vec<&Session> = live
                .iter()
                .copied()
                .filter(|s| s.engine != Engine::Claude)
                .collect();
            return if others.len() == 1 {
                Some(others[0])
            } else {
                None
            };
        }
        let claude: Vec<&Session> = live
            .iter()
            .copied()
            .filter(|s| s.engine == Engine::Claude)
            .collect();
        let known: Vec<BTreeSet<String>> = claude.iter().map(|s| title_texts(s)).collect();
        if known.iter().any(BTreeSet::is_empty) {
            return None;
        }
        let hits: Vec<usize> = (0..claude.len())
            .filter(|&i| known[i].contains(text))
            .collect();
        if hits.len() == 1 {
            Some(claude[hits[0]])
        } else {
            None
        }
    }

    /// State the window SHOWS. Until 2026-09-21 "done as soon as any tab is done" lit the window after every
    /// /handoff while the visible tab worked (Kevin). Without a recognisable tab the title glyph decides, then the
    /// old rule.
    pub fn shown_state(&self) -> ShownState {
        if let Some(v) = self.visible_session() {
            return ShownState {
                ready: v.ready,
                waiting: v.waiting,
                watching: v.watching,
            };
        }
        let live: Vec<&Session> = self.sessions.iter().filter(|s| s.alive).collect();
        if live.len() <= 1 {
            return ShownState::default();
        }
        let waiting = live.iter().any(|s| s.waiting);
        match title_glyph(&self.title).0 {
            TitleGlyph::Busy => ShownState::default(),
            TitleGlyph::Idle => {
                // ✳ = the visible tab does not work. If no session is done but one watches, it is that one (review
                // quick 2026-09-27, GLM); otherwise a watcher tab would light up as done again.
                let watching_only =
                    !live.iter().any(|s| s.ready) && live.iter().any(|s| s.watching);
                ShownState {
                    ready: !watching_only,
                    waiting,
                    watching: watching_only,
                }
            }
            TitleGlyph::None => ShownState {
                ready: live.iter().any(|s| s.ready),
                waiting,
                watching: false,
            },
        }
    }

    /// Done in the visible tab: frame glow, Focus 2 side and menu bar count depend on it.
    pub fn is_ready(&self) -> bool {
        self.shown_state().ready
    }

    /// Question in the visible tab: highest priority (`attention_order`).
    pub fn is_waiting(&self) -> bool {
        self.shown_state().waiting
    }

    /// Only waits for its own background watcher (Kevin 2026-09-27): no glow, stays with the running ones. Question
    /// and done go first.
    pub fn is_watching(&self) -> bool {
        let s = self.shown_state();
        !s.ready && !s.waiting && s.watching
    }

    /// A session waits for Kevin but the window shows no question: it sits in a HIDDEN tab. Only a hint on the label
    /// (Kevin 2026-09-21: done hidden tabs are ignored).
    pub fn hidden_question(&self) -> bool {
        !self.is_waiting() && self.sessions.iter().any(|s| s.alive && s.waiting)
    }

    /// Layout slots go only to windows that can be moved.
    pub fn layoutable(&self) -> bool {
        self.movable && !self.minimized && !self.fullscreen
    }
}

/// What a label reports; decides symbol, word and colour (Overlay.swift). Here because the layout's list header uses
/// the word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelTone {
    Waiting,
    Ready,
    Running,
    Watching,
}

impl LabelTone {
    pub fn symbol(self) -> &'static str {
        match self {
            LabelTone::Waiting => "?",
            LabelTone::Ready => "✓",
            LabelTone::Running => "●",
            LabelTone::Watching => "◌",
        }
    }

    /// Plain words (Kevin 2026-09-26, label B): "waiting for you" covers question AND approval, "working" instead of
    /// "running"; "watching" (2026-09-27): answer done, waits for its own background watcher, does not need Kevin.
    pub fn word(self) -> &'static str {
        match self {
            LabelTone::Waiting => tr("WARTET AUF DICH", "WAITING FOR YOU"),
            LabelTone::Ready => tr("FERTIG", "DONE"),
            LabelTone::Running => tr("ARBEITET", "WORKING"),
            LabelTone::Watching => tr("ÜBERWACHT", "WATCHING"),
        }
    }
}

// ---------- Screens and targets (Layout.swift) ----------

#[derive(Clone, Debug, PartialEq)]
pub struct ScreenInfo {
    pub visible: Rect,
    pub builtin: bool,
    /// Identity that survives unplugging (display UUID); empty = unknown, then not selectable on its own.
    pub id: String,
    /// Name as macOS calls the screen (external ones); the built-in one is called MacBook or iMac in the menu.
    pub name: String,
    /// Built-in screen of a laptop (MacBook), else of an iMac.
    pub laptop: bool,
}

impl ScreenInfo {
    /// No id, no name, laptop (Swift defaults).
    pub fn new(visible: Rect, builtin: bool) -> ScreenInfo {
        ScreenInfo {
            visible,
            builtin,
            id: String::new(),
            name: String::new(),
            laptop: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackSide {
    Left,
    Right,
}

impl StackSide {
    /// "left" is left, anything else right (Swift `StackSide(_:)`).
    pub fn parse(raw: &str) -> StackSide {
        if raw == "left" {
            StackSide::Left
        } else {
            StackSide::Right
        }
    }

    /// Swift `rawValue`, as the log writes it.
    pub fn raw(self) -> &'static str {
        match self {
            StackSide::Left => "left",
            StackSide::Right => "right",
        }
    }
}

/// The arrangements (design v3, Kevin 2026-09-18; Inbox 2026-11-09). Order = menu; the first three are also the
/// shortcuts Ctrl+Opt+1…3, the inbox is chosen in the menu only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutMode {
    Reading,
    Focus2,
    Grid,
    Inbox,
}

impl LayoutMode {
    /// Swift `allCases`.
    pub const ALL: [LayoutMode; 4] = [
        LayoutMode::Reading,
        LayoutMode::Focus2,
        LayoutMode::Grid,
        LayoutMode::Inbox,
    ];

    pub fn raw(self) -> &'static str {
        match self {
            LayoutMode::Reading => "reading",
            LayoutMode::Focus2 => "focus2",
            LayoutMode::Grid => "grid",
            LayoutMode::Inbox => "inbox",
        }
    }

    pub fn from_raw(raw: &str) -> Option<LayoutMode> {
        LayoutMode::ALL.into_iter().find(|m| m.raw() == raw)
    }

    pub fn label(self) -> &'static str {
        match self {
            LayoutMode::Reading => tr("Fokus", "Focus"),
            LayoutMode::Focus2 => tr("Fokus 2", "Focus 2"),
            LayoutMode::Grid => tr("Raster", "Grid"),
            LayoutMode::Inbox => tr("Eingang", "Inbox"),
        }
    }

    /// Old configuration values still understood instead of falling to the default: `focus` and `reading` were two
    /// menu items with the same picture on narrow screens (Kevin 2026-09-18); a stored stack becomes Focus.
    pub fn parse(raw: &str) -> LayoutMode {
        LayoutMode::from_raw(raw).unwrap_or(match raw {
            "focus" | "stack" => LayoutMode::Reading,
            _ => LayoutMode::Grid,
        })
    }
}

/// Where the windows go (Kevin 2026-09-18, replaces the waiting room). `Single` = all AI windows on one screen.
/// `ControlRoom` (Swift `leitstand`): the active session centred on the work screen, whoever waits for Kevin beside it
/// on `ready_side` (Kevin 2026-09-24: control room left, control room 2 right), everything running as a grid on the
/// other screen. `flipped` (control room 3, Kevin 2026-09-28): the work screen is the MacBook, the running ones stand
/// at the left and right edge of the big screen, its middle stays free. Swift defaults: `ready_side` left, `flipped`
/// false.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Single(usize),
    ControlRoom {
        work: usize,
        control: usize,
        ready_side: StackSide,
        flipped: bool,
    },
}

/// Kevin's choice in the menu. Resolved against the attached screens every tick and kept while its screen is missing
/// (MacBook alone, lid closed). `Screen` = one specific screen by id (Kevin 2026-10-01: every screen selectable);
/// `External` (largest external) and `Builtin` are older stored values, still understood but no longer in the menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetChoice {
    External,
    Builtin,
    Both,
    Both2,
    Both3,
    Screen(String),
}

impl TargetChoice {
    /// As stored in the defaults and menu keys: "both3", "screen:<id>" and so on.
    pub fn from_raw(raw: &str) -> Option<TargetChoice> {
        if let Some(id) = raw.strip_prefix("screen:") {
            return (!id.is_empty()).then(|| TargetChoice::Screen(id.to_owned()));
        }
        Some(match raw {
            "external" => TargetChoice::External,
            "builtin" => TargetChoice::Builtin,
            "both" => TargetChoice::Both,
            "both2" => TargetChoice::Both2,
            "both3" => TargetChoice::Both3,
            _ => return None,
        })
    }

    pub fn raw(&self) -> String {
        match self {
            TargetChoice::External => "external".into(),
            TargetChoice::Builtin => "builtin".into(),
            TargetChoice::Both => "both".into(),
            TargetChoice::Both2 => "both2".into(),
            TargetChoice::Both3 => "both3".into(),
            TargetChoice::Screen(id) => format!("screen:{id}"),
        }
    }
}

/// The two control-room roles: screen ids (`Pair<String>`, the menu choice) or screen indices (`Pair<usize>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pair<T> {
    pub work: T,
    pub control: T,
}

/// A fixed grid, columns × rows. "keine Auswahl verschiedener Raster" was Kevin's finding about v2, where the
/// automatic computed the columns itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSpec {
    pub cols: usize,
    pub rows: usize,
}

impl GridSpec {
    pub const fn new(cols: usize, rows: usize) -> GridSpec {
        GridSpec { cols, rows }
    }

    /// "3x2" = 3 columns, 2 rows, as the choice is stored. Unreadable parts between the "x" are skipped, as in Swift.
    pub fn parse(key: &str) -> Option<GridSpec> {
        let parts: Vec<i64> = key
            .split('x')
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse().ok())
            .collect();
        match parts[..] {
            [c, r] if c > 0 && r > 0 => Some(GridSpec::new(
                usize::try_from(c).ok()?,
                usize::try_from(r).ok()?,
            )),
            _ => None,
        }
    }

    pub fn key(self) -> String {
        format!("{}x{}", self.cols, self.rows)
    }

    pub fn label(self) -> String {
        format!("{} × {}", self.cols, self.rows)
    }
}

// ---------- Plan results (Layout.swift, plan 1d) ----------

/// Split of one side: the first `cards` as windows, `rows` as list rows, `more` without a row of their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RailSplit {
    pub cards: usize,
    pub rows: usize,
    pub more: usize,
}

/// The buttons on an open card of the inbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardButton {
    ReadNext,
    Park,
    Unpark,
}

/// List entry: header, window (index in `windows`) or "+N more (in the menu)". `Working` and `Lot` are windows for the
/// inbox's second tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListEntry {
    Header(String),
    Row(usize),
    More(usize),
    Working(usize),
    Lot(usize),
}

/// List at the foot of a side (Cocoa rectangle) with its entries from top to bottom. `inbox`: the inbox window, whose
/// entries are drawn as cards (`overlay::inbox_target`).
#[derive(Clone, Debug, PartialEq)]
pub struct SideList {
    pub rect: Rect,
    pub entries: Vec<ListEntry>,
    pub inbox: bool,
}

impl SideList {
    /// Parked windows of this list: rows plus "+N more".
    pub fn count(&self) -> usize {
        self.entries
            .iter()
            .map(|e| match e {
                ListEntry::Row(_) | ListEntry::Working(_) | ListEntry::Lot(_) => 1,
                ListEntry::More(m) => *m,
                ListEntry::Header(_) => 0,
            })
            .sum()
    }
}

/// Plan result: frame per window, which ones are parked, the lists and the middle (GP 2: "parked" comes from the plan,
/// not from comparing with the screen, which counts congruent windows as visible). Window keys are indices into the
/// planned `windows`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arrangement {
    pub frames: BTreeMap<usize, Rect>,
    /// Behind the main window (frame = main frame): list overflow and the parking lot.
    pub parked: BTreeSet<usize>,
    pub lists: Vec<SideList>,
    pub main: Option<usize>,
    /// Parking lot (Kevin 2026-11-06): running or watching worker windows. Parked behind the main window without a
    /// list row, or tiled on the largest screen while peeking (then not in `parked`).
    pub lot: BTreeSet<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::attention_order;
    use crate::testutil::*;

    /// Claude writes the window title itself: spinner = working, ✳ = done/question (measured 2026-09-21).
    #[test]
    fn test_title_glyph() {
        let busy = title_glyph("◐ Handoff spawn 2026-09-21 beta-pr1-build");
        assert!(
            busy.0 == TitleGlyph::Busy && busy.1 == "Handoff spawn 2026-09-21 beta-pr1-build",
            "spinner = working, text without glyph"
        );
        assert_eq!(
            title_glyph("◑ Muse code erkunden").0,
            TitleGlyph::Busy,
            "both spinner halves"
        );
        let idle = title_glyph("✳ Rechnungen prüfen");
        assert!(idle.0 == TitleGlyph::Idle && idle.1 == "Rechnungen prüfen");
        let folder = title_glyph("shop+feature-pr0");
        assert!(
            folder.0 == TitleGlyph::None && folder.1 == "shop+feature-pr0",
            "folder name after ping: no glyph"
        );
        assert_eq!(
            title_glyph("wg-map-4711").0,
            TitleGlyph::None,
            "ping marker is no status"
        );
        assert_eq!(title_glyph("").0, TitleGlyph::None);
    }

    /// Swift's `dropFirst` drops a whole character; measured in Swift: all three give " Foo" before trimming.
    #[test]
    fn title_glyph_drops_the_whole_first_character() {
        assert_eq!(title_glyph("✳\u{FE0F} Foo"), (TitleGlyph::Idle, "Foo"));
        assert_eq!(title_glyph("✳\u{20E3} Foo"), (TitleGlyph::Idle, "Foo"));
        assert_eq!(title_glyph("✳\u{200D}🔥 Foo"), (TitleGlyph::Idle, "Foo"));
        assert_eq!(title_glyph("✳Foo"), (TitleGlyph::Idle, "Foo"));
        assert_eq!(
            trim_ws("\u{A0} a\n\u{200B}\u{3000}"),
            "a\n",
            "Foundation's whitespace set keeps line breaks"
        );
    }

    /// Kevin 2026-09-21: "the top window glows green but it works". /handoff opens the new tab in the same window; the
    /// old tab (beta-52) was done, the visible one (beta-e5) worked.
    #[test]
    fn test_visible_tab_decides() {
        let alt = tab_session("beta-52", true, false, Some("Handoff spawn loop closing"));
        let neu = tab_session(
            "beta-e5",
            false,
            false,
            Some("Handoff spawn 2026-09-21 beta-pr1-build"),
        );
        let w = tab_window(
            vec![alt.clone(), neu.clone()],
            "◐ Handoff spawn 2026-09-21 beta-pr1-build",
        );
        assert_eq!(
            w.visible_session().map(|s| &s.id),
            Some(&neu.id),
            "the visible tab is found by its topic"
        );
        assert!(!w.is_ready(), "visible tab works -> window works");
        // (Swift's glowTargets assert "no glow" belongs to wave B.)
        let laeuft = fake_window(false, 500.0);
        assert_eq!(
            attention_order(&[&w, &laeuft]),
            vec![1, 0],
            "no jumping the queue of the waiting"
        );
        let mut zurueck = w.clone();
        zurueck.title = "✳ Handoff spawn loop closing".into();
        assert!(
            zurueck.visible_session().map(|s| &s.id) == Some(&alt.id) && zurueck.is_ready(),
            "Kevin switches to the old tab: done"
        );
    }

    #[test]
    fn test_visible_tab_fallbacks() {
        let a = tab_session("a", true, false, Some("Thema A"));
        let b = tab_session("b", false, false, Some("Thema B"));
        let unbekannt = tab_window(vec![a.clone(), b.clone()], "◐ Neues Thema");
        assert!(
            unbekannt.visible_session().is_none() && !unbekannt.is_ready(),
            "spinner -> working"
        );
        let ruhig = tab_window(vec![a.clone(), b.clone()], "✳ Neues Thema");
        assert!(ruhig.is_ready() && !ruhig.is_waiting(), "✳ -> done");
        assert!(
            tab_window(vec![a, b.clone()], "Beta").is_ready(),
            "no clue: done as soon as one tab is done"
        );
        assert!(
            !tab_window(vec![b], "✳ irgendwas").is_ready(),
            "one session decides itself"
        );
        assert!(
            !tab_window(vec![], "✳ irgendwas").is_ready(),
            "a window without session is never done"
        );
    }

    /// review quick 2026-09-21: a unique topic hit only proves something if every Claude session knows its topic
    /// (GLM); without a Claude glyph the ONE non-Claude session is visible.
    #[test]
    fn test_visible_tab_robust() {
        let codex = Session::new(
            "codex:1",
            Engine::Codex,
            Some(2),
            "Alpha",
            "/Users/test/Projects/Alpha",
            false,
            true,
        );
        let w = tab_window(
            vec![
                tab_session("alt", true, false, Some("Alpha")),
                codex.clone(),
            ],
            "Alpha",
        );
        assert!(
            w.visible_session().map(|s| &s.id) == Some(&codex.id) && !w.is_ready(),
            "without a Claude glyph the Codex session is visible"
        );
        let handoff = tab_window(
            vec![
                tab_session("alt2", true, false, Some("Alte Aufgabe")),
                codex,
            ],
            "Alpha",
        );
        assert!(
            !handoff.is_ready(),
            "handoff after Codex: the working Codex tab decides"
        );
        let ohne_thema = Session::new(
            "claude:neu",
            Engine::Claude,
            Some(3),
            "neu",
            "",
            false,
            true,
        );
        let gleich = tab_window(
            vec![
                tab_session("alt3", true, false, Some("Handoff spawn 2026-09-21")),
                ohne_thema,
            ],
            "◐ Handoff spawn 2026-09-21",
        );
        assert!(
            gleich.visible_session().is_none() && !gleich.is_ready(),
            "hit not conclusive -> the glyph decides"
        );
        let a = tab_session("a", true, false, Some("Thema A "));
        let b = tab_session("b", false, false, Some("Thema B"));
        assert_eq!(
            tab_window(vec![a.clone(), b], "✳ Thema A")
                .visible_session()
                .map(|s| &s.id),
            Some(&a.id),
            "topic with trailing space"
        );
    }

    #[test]
    fn test_hidden_question() {
        let fragt = tab_session("alt", true, true, Some("Alte Aufgabe"));
        let arbeitet = tab_session("neu", false, false, Some("Neue Aufgabe"));
        let w = tab_window(vec![fragt.clone(), arbeitet.clone()], "◐ Neue Aufgabe");
        assert!(
            !w.is_waiting() && w.hidden_question(),
            "question in a hidden tab: only a hint"
        );
        let sichtbar = tab_window(vec![fragt, arbeitet.clone()], "✳ Alte Aufgabe");
        assert!(
            sichtbar.is_waiting() && !sichtbar.hidden_question(),
            "visible question: glow, no extra hint"
        );
        let fertig = tab_session("f", true, false, Some("Fertig"));
        assert!(
            !tab_window(vec![fertig, arbeitet], "◐ Neue Aufgabe").hidden_question(),
            "done hidden tabs are ignored (Kevin 2026-09-21)"
        );
    }

    #[test]
    fn watching_needs_no_question_and_no_done() {
        let mut s = tab_session("w", false, false, Some("Aufpasser"));
        s.watching = true;
        let w = tab_window(vec![s], "✳ Aufpasser");
        assert!(w.is_watching() && !w.is_ready() && !w.is_waiting());
        let idle_tab = tab_window(
            vec![
                Session {
                    watching: true,
                    ..tab_session("a", false, false, None)
                },
                tab_session("b", false, false, None),
            ],
            "✳ ?",
        );
        assert_eq!(
            idle_tab.shown_state(),
            ShownState {
                ready: false,
                waiting: false,
                watching: true
            },
            "✳ with only a watcher: watching, not done (review quick 2026-09-27)"
        );
    }

    #[test]
    fn raw_values_round_trip() {
        for e in Engine::ALL {
            assert_eq!(Engine::from_raw(e.raw()), Some(e));
        }
        assert_eq!(Engine::from_raw(" claude"), None);
        for m in LayoutMode::ALL {
            assert_eq!(LayoutMode::from_raw(m.raw()), Some(m));
        }
        assert!(
            GridSpec::parse("3xax2") == Some(GridSpec::new(3, 2))
                && GridSpec::parse("-3x2").is_none()
        );
        assert!(
            GridSpec::parse("x3x2x") == Some(GridSpec::new(3, 2))
                && GridSpec::parse("3x2x1").is_none()
        );
        assert_eq!(fold_case("STRASSE"), fold_case("Straße"));
    }
}
