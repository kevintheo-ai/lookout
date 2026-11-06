//! Window mapping (wave B2): process tree, ping schedule, matching sessions to windows by name or title marker.
//! Ported from Mapper.swift.
//!
//! Matching titles alone does not hold (spike 2026-09-18): Claude writes its task into the title, Codex the folder
//! name, Kimi the prompt, unused shells "zsh". What holds is the title ping: a unique marker as OSC-0 title on the
//! session's tty, read the window titles, write the title back. The terminal handles the sequence itself, the running
//! program never sees it (live: 8 of 9 sessions in one pass, the ninth sat in a background tab). Since v4 a Claude
//! session is matched by the name in its window title first (`title_matches`), the ping stays the fallback.
//!
//! Platform work stays outside and comes in as plain data: windows and their titles (AX), process rows and working
//! folders (libproc), writing titles to ttys. Windows are named by `TermWindow::id`.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    Engine, EngineProc, OnScreenWindow, Pid, PsRow, Session, TermWindow, Time, TitleGlyph,
    title_glyph, title_texts, trim_ws,
};
use crate::overlay::{OnScreenMatch, locate_on_screen};

// ---------- The session in front of the user ----------

/// Claude session the user has in front of him (Kevin 2026-10-04, `answerSeen`): the visible tab of the focused window,
/// only if its terminal is the frontmost app and the window lies on the visible desktop (minimized or another space
/// does not count, review quick 2026-10-04, GLM). `title`: the window title read just now; without it nothing counts
/// (the remembered one could show the tab of 3 s ago). Even with ONE session the title must prove it, Claude glyph and
/// topic: else a plain shell tab (Codex) or the tab of an ended session with an old "✳ …" (GLM) would stand in for it.
/// `None` = nothing counts as seen.
pub fn session_in_front(
    windows: &[TermWindow],
    focused: Option<u64>,
    title: Option<&str>,
    front_pid: Option<Pid>,
    on_screen: Option<&[OnScreenWindow]>,
    screen_h: f64,
) -> Option<String> {
    let (focused, title) = (focused?, title?);
    let found = windows.iter().find(|w| w.id == focused)?;
    if Some(found.owner_pid) != front_pid {
        return None;
    }
    match locate_on_screen(found, on_screen, screen_h) {
        OnScreenMatch::Unique(_) | OnScreenMatch::Ambiguous => {}
        OnScreenMatch::Absent | OnScreenMatch::Unknown => return None,
    }
    let mut w = found.clone();
    w.title = title.to_owned();
    let s = w.visible_session().filter(|s| s.engine == Engine::Claude)?;
    let (glyph, text) = title_glyph(&w.title);
    (glyph != TitleGlyph::None && title_texts(s).contains(text)).then(|| s.id.clone())
}

// ---------- Matching by name (v4, Kevin 2026-09-21) ----------

/// Windows recognised by name (v4, Kevin 2026-09-21: a new window came to the middle late). Working sessions paint
/// their spinner over the ping marker ("ohne Marker" four times in a row in the log), but their title gives them away.
/// Only unique pairs: a Claude session with known name or topic and exactly one window with a Claude glyph whose title
/// text fits (no hit: the fixed tab title fallback below). Result: session pid -> window index. Anything ambiguous is
/// left to the ping.
pub fn title_matches(
    sessions: &[Session],
    windows: &[TermWindow],
    all: &[EngineProc],
) -> BTreeMap<Pid, usize> {
    // Only processes under Warp/Ghostty and their sessions (plan R2-8): a session in another terminal never stands in
    // these windows and would block every name match without a title. Filtered here, not by the caller, so an
    // unfiltered list cannot bring the bug back (review quick 2026-09-26, Kimi).
    let procs: Vec<&EngineProc> = all.iter().filter(|p| p.in_terminal).collect();
    let pids: BTreeSet<Pid> = procs.iter().map(|p| p.pid).collect();
    let candidates: Vec<&Session> = sessions
        .iter()
        .filter(|s| {
            s.engine == Engine::Claude && s.alive && s.pid.is_some_and(|p| pids.contains(&p))
        })
        .collect();
    // A title is unique only if EVERY Claude session in a terminal is known: a fresh one (not asked yet or without
    // transcript) may already carry the same start name. A wrong pair would stick, mapped pids are never pinged again
    // (review quick v4).
    let known: BTreeSet<Pid> = candidates.iter().filter_map(|s| s.pid).collect();
    let texts: Vec<BTreeSet<String>> = candidates.iter().map(|s| title_texts(s)).collect();
    if !procs
        .iter()
        .all(|p| p.engine != Engine::Claude || known.contains(&p.pid))
        || texts.iter().any(BTreeSet::is_empty)
    {
        return BTreeMap::new();
    }
    let mut out = BTreeMap::new();
    for (k, s) in candidates.iter().enumerate() {
        let mine = &texts[k];
        // A second session with the same name: the title proves nothing.
        let twin = candidates
            .iter()
            .zip(&texts)
            .any(|(o, t)| o.id != s.id && !t.is_disjoint(mine));
        if twin {
            continue;
        }
        let mut hits: Vec<usize> = (0..windows.len())
            .filter(|&i| {
                let (glyph, text) = title_glyph(&windows[i].title);
                glyph != TitleGlyph::None && mine.contains(text)
            })
            .collect();
        // Fallback (Kevin 2026-10-24): a fixed tab title (Warp launch configuration with `title:`) never shows a Claude
        // glyph and swallows the ping marker. Then the session's own name counts as the WHOLE title; the topic does
        // not, it resembles a folder name too easily (Codex).
        let name = s
            .digest
            .as_ref()
            .and_then(|d| d.name.as_deref())
            .map(trim_ws)
            .filter(|n| !n.is_empty());
        if let (true, Some(name)) = (hits.is_empty(), name) {
            hits = (0..windows.len())
                .filter(|&i| title_glyph(&windows[i].title) == (TitleGlyph::None, name))
                .collect();
        }
        if let ([hit], Some(pid)) = (hits.as_slice(), s.pid) {
            out.insert(pid, *hit);
        }
    }
    out
}

// ---------- Processes ----------

/// `NSString.lastPathComponent`: the part after the last "/", trailing slashes ignored; "/" stays "/".
pub fn last_path_component(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.is_empty() { "" } else { "/" };
    }
    trimmed.rfind('/').map_or(trimmed, |i| &trimmed[i + 1..])
}

/// AI CLI with a real terminal? Only such a process can stand in a terminal window. The name is the last path part of
/// the command as ps shows it (bare name, path or whole command line).
pub fn engine_of(row: &PsRow) -> Option<Engine> {
    if row.tty.starts_with("ttys") {
        Engine::from_raw(last_path_component(&row.comm))
    } else {
        None
    }
}

/// Does one of `ancestors` lie above `pid` (the process itself does not count)? At most 32 steps, so a loop in the
/// parent map cannot run forever.
pub fn has_ancestor(pid: Pid, ancestors: &BTreeSet<Pid>, parents: &BTreeMap<Pid, Pid>) -> bool {
    let mut current = pid;
    for _ in 0..32 {
        match parents.get(&current) {
            Some(&parent) if parent != current => {
                if ancestors.contains(&parent) {
                    return true;
                }
                current = parent;
            }
            _ => return false,
        }
    }
    false
}

/// Process list (plan 1a): first the parent map from ALL rows, then the AI CLIs with a terminal, each with
/// `in_terminal`: does a running Warp or Ghostty process (`terminal_pids`) lie in its ancestry? Live 2026-09-26:
/// `claude ← -zsh ← Warp helper ← Warp ← launchd`. Swift parsed `ps` text and answered nil for unreadable text; the rows
/// now come from the platform layer, which answers "unknown" itself when it cannot read them (an empty list would drop
/// every window match, review 2026-09-25 finding 10).
pub fn process_list(
    rows: &[PsRow],
    terminal_pids: &BTreeSet<Pid>,
    cwd: impl Fn(Pid) -> String,
) -> Vec<EngineProc> {
    let mut parents = BTreeMap::new();
    for r in rows {
        // The first row of a pid wins, like Swift's `uniquingKeysWith: { first, _ in first }`.
        parents.entry(r.pid).or_insert(r.ppid);
    }
    rows.iter()
        .filter_map(|r| {
            let engine = engine_of(r)?;
            Some(EngineProc {
                pid: r.pid,
                engine,
                tty: r.tty.clone(),
                cwd: cwd(r.pid),
                in_terminal: has_ancestor(r.pid, terminal_pids, &parents),
            })
        })
        .collect()
}

// ---------- Windows ----------

pub fn index_of_window(id: u64, windows: &[TermWindow]) -> Option<usize> {
    windows.iter().position(|w| w.id == id)
}

/// Snapshot taken while Lookout moved windows itself: its frames are intermediate. The last measured ones (`known`)
/// stay, else a window that had just arrived jumped back and glided again (2026-09-25). Same for "minimized": bringing a
/// window to the front fetches it from the Dock, a tick already running still knows it minimized (plan R2-4). If the
/// known state is wrong, the next tick heals it.
pub fn keep_measured_frames(fresh: Vec<TermWindow>, known: &[TermWindow]) -> Vec<TermWindow> {
    fresh
        .into_iter()
        .map(|mut w| {
            if let Some(old) = known.iter().find(|k| k.id == w.id) {
                w.frame = old.frame;
                w.minimized = old.minimized;
            }
            w
        })
        .collect()
}

// ---------- When to ping (design v3, bug 1) ----------

/// Swift `Date.distantPast`: every retry interval since then has passed.
const DISTANT_PAST: Time = f64::NEG_INFINITY;

/// Ping bookkeeping per process.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PingState {
    pub attempts: u32,
    pub last_try: Time,
}

impl Default for PingState {
    fn default() -> PingState {
        PingState {
            attempts: 0,
            last_try: DISTANT_PAST,
        }
    }
}

/// Gap to the next try: the first four every 15 s, then every 2 minutes, never giving up for good. v2 gave up after 4
/// misses and so lost every session whose spinner just overwrote the marker. The long gap spares background tabs: their
/// tab title flashes briefly on every try.
pub fn ping_retry_interval(attempts: u32) -> f64 {
    if attempts < 4 { 15.0 } else { 120.0 }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PingSchedule {
    pub due: BTreeSet<Pid>,
    pub pending: BTreeMap<Pid, PingState>,
}

/// Who is pinged now? Processes without a window, until it works. When a NEW window appears, everyone, at once: a tab
/// pulled out leaves its old window standing and the old match held forever (Kevin 2026-09-18, window #10192 stayed
/// unarranged). The number of misses stays: a background tab falls back to its long gap instead of flashing every 15 s
/// again. Vanished processes drop out. Without Accessibility access (`trusted` false) no title is readable, so it only
/// notes them: every try would write markers and folder names into the user's tab titles. Same when AX gives not a
/// single terminal window (`windows_readable` false, locked screen): until 2026-09-25 Lookout then pinged every session
/// every 2 min and set a tab title to the folder name 15,696 times.
pub fn ping_schedule(
    pids: &[Pid],
    mapped: &BTreeSet<Pid>,
    pending: &BTreeMap<Pid, PingState>,
    new_window: bool,
    now: Time,
    trusted: bool,
    windows_readable: bool,
) -> PingSchedule {
    let live: BTreeSet<Pid> = pids.iter().copied().collect();
    let mut next: BTreeMap<Pid, PingState> = pending
        .iter()
        .filter(|(pid, _)| live.contains(pid))
        .map(|(pid, s)| (*pid, *s))
        .collect();
    for &pid in pids {
        if new_window || (!mapped.contains(&pid) && !next.contains_key(&pid)) {
            let attempts = next.get(&pid).map_or(0, |s| s.attempts);
            next.insert(
                pid,
                PingState {
                    attempts,
                    last_try: DISTANT_PAST,
                },
            );
        }
    }
    if !trusted || !windows_readable {
        return PingSchedule {
            due: BTreeSet::new(),
            pending: next,
        };
    }
    let due = next
        .iter()
        .filter(|(_, s)| now - s.last_try >= ping_retry_interval(s.attempts))
        .map(|(pid, _)| *pid)
        .collect();
    PingSchedule { due, pending: next }
}

/// After a ping round: found -> rest, otherwise one miss more.
pub fn ping_result(
    due: &BTreeSet<Pid>,
    found: &BTreeSet<Pid>,
    pending: &BTreeMap<Pid, PingState>,
    now: Time,
) -> BTreeMap<Pid, PingState> {
    let mut next = pending.clone();
    for &pid in due {
        if found.contains(&pid) {
            next.remove(&pid);
        } else {
            let attempts = pending
                .get(&pid)
                .map_or(0, |s| s.attempts)
                .saturating_add(1);
            next.insert(
                pid,
                PingState {
                    attempts,
                    last_try: now,
                },
            );
        }
    }
    next
}

// ---------- Title ping (`pingMap` without AX and tty writes) ----------
//
// Lessons from the live runs (2026-09-18): a round that hits only part of the processes must NOT replace the whole map,
// else the matching jumps between 12/12 and 1/12. Known pairs stay valid as long as process AND window exist; a known
// pair checked again whose marker is not seen stays as a fallback. A working Claude session repaints its spinner title
// all the time and overwrites the marker, so the marker is written again in EVERY round and then only the titles are
// read, every 10 ms for up to 150 ms: a full snapshot (7 AX reads per window) took longer than the marker lived.

const PING_MARKER: &str = "wg-map-";
/// Rounds of "write markers, read titles" per ping.
pub const PING_ROUNDS: usize = 4;
/// How long a round reads titles, and how often.
pub const PING_WAIT: f64 = 0.15;
pub const PING_POLL: f64 = 0.01;

/// Title marker for a process.
pub fn ping_marker(pid: Pid) -> String {
    format!("{PING_MARKER}{pid}")
}

/// A title to write to a tty as OSC 0 (`ESC ] 0 ; title BEL`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleWrite {
    pub tty: String,
    pub title: String,
}

/// Known pairs whose process still runs and whose window still exists; the others are pinged again.
pub fn kept_pairs(
    known: &BTreeMap<Pid, u64>,
    procs: &[EngineProc],
    windows: &[TermWindow],
) -> BTreeMap<Pid, u64> {
    known
        .iter()
        .filter(|(pid, id)| {
            procs.iter().any(|p| p.pid == **pid) && windows.iter().any(|w| w.id == **id)
        })
        .map(|(pid, id)| (*pid, *id))
        .collect()
}

/// One ping (`pingMap`). The caller, for at most `PING_ROUNDS` rounds while `open` is not empty: writes `markers()`,
/// then every `PING_POLL` s for up to `PING_WAIT` s reads the titles of the windows of `before` and hands them to
/// `read`, writing what it returns at once. At the end it writes `finish()` and logs `summary()`. Only `due` processes
/// are pinged: a settled matching leaves the titles alone.
#[derive(Clone, Debug, PartialEq)]
pub struct Ping {
    /// Process -> window id, known pairs carried over.
    pub map: BTreeMap<Pid, u64>,
    pub found: BTreeSet<Pid>,
    /// Pinged processes whose marker was not seen yet.
    pub open: Vec<EngineProc>,
    pinged: usize,
    in_terminal: usize,
    /// Window id and title of the snapshot before the ping.
    before: Vec<(u64, String)>,
}

impl Ping {
    pub fn start(
        procs: &[EngineProc],
        known: &BTreeMap<Pid, u64>,
        before: &[TermWindow],
        due: &BTreeSet<Pid>,
    ) -> Ping {
        let open: Vec<EngineProc> = procs
            .iter()
            .filter(|p| due.contains(&p.pid))
            .cloned()
            .collect();
        Ping {
            map: kept_pairs(known, procs, before),
            found: BTreeSet::new(),
            pinged: open.len(),
            open,
            in_terminal: procs.iter().filter(|p| p.in_terminal).count(),
            before: before.iter().map(|w| (w.id, w.title.clone())).collect(),
        }
    }

    pub fn markers(&self) -> Vec<TitleWrite> {
        self.open
            .iter()
            .map(|p| TitleWrite {
                tty: p.tty.clone(),
                title: ping_marker(p.pid),
            })
            .collect()
    }

    /// One reading: the fresh title of each window of `before`, in its order ("" if unreadable). Returns the titles to
    /// write back right away, not after all rounds: else a visible window shows the marker as long as any background tab
    /// is searched (review quick).
    pub fn read(&mut self, fresh: &[String]) -> Vec<TitleWrite> {
        let mut back = vec![];
        for ((id, original), title) in self.before.iter().zip(fresh) {
            let Some(pid) = title
                .strip_prefix(PING_MARKER)
                .and_then(|n| n.parse::<Pid>().ok())
            else {
                continue;
            };
            let Some(p) = self.open.iter().find(|p| p.pid == pid) else {
                continue;
            };
            if !self.found.insert(pid) {
                continue;
            }
            self.map.insert(pid, *id);
            let original = (!original.starts_with(PING_MARKER)).then_some(original.as_str());
            back.push(restore_title(p, original));
        }
        self.open.retain(|p| !self.found.contains(&p.pid));
        back
    }

    /// Titles of the processes whose marker never showed.
    pub fn finish(&self) -> Vec<TitleWrite> {
        self.open.iter().map(|p| restore_title(p, None)).collect()
    }

    /// Log line (the log stays German). Only processes under Warp/Ghostty can be matched (plan 1a).
    pub fn summary(&self) -> String {
        let mut line = format!(
            "Mapper: {}/{} gepingt, {}/{} Sessions zugeordnet",
            self.found.len(),
            self.pinged,
            self.map.len(),
            self.in_terminal
        );
        if !self.open.is_empty() {
            line += &format!(
                " ({} ohne Marker — Hintergrund-Tab oder Spinner, neuer Versuch folgt)",
                self.open.len()
            );
        }
        line
    }
}

/// Title back. Without a known original (background tab) the title falls back to the folder name: the usual terminal
/// naming and never an empty tab.
fn restore_title(p: &EngineProc, original: Option<&str>) -> TitleWrite {
    let fallback = if p.cwd.is_empty() {
        p.engine.raw()
    } else {
        last_path_component(&p.cwd)
    };
    TitleWrite {
        tty: p.tty.clone(),
        title: original.unwrap_or(fallback).to_owned(),
    }
}

// ---------- Joining ----------

/// Attaches each session to its process (Claude by pid, the others by working folder) and drops sessions without a
/// running process: their log file just lies around (spike: 11 log sessions, most long ended).
pub fn attach_processes(sessions: &[Session], procs: &[EngineProc]) -> Vec<Session> {
    let mut used = BTreeSet::new();
    sessions
        .iter()
        .filter_map(|s| {
            if let Some(pid) = s.pid {
                if !procs.iter().any(|p| p.pid == pid) {
                    return None;
                }
                used.insert(pid);
                return Some(s.clone());
            }
            // Claude background agents have no terminal and no window. Hanging them on some process by folder would be
            // a guess: they stay in the list without pid and so outside every window matching.
            if s.engine == Engine::Claude {
                return Some(s.clone());
            }
            if s.cwd.is_empty() {
                return None;
            }
            let m = procs
                .iter()
                .find(|p| p.engine == s.engine && p.cwd == s.cwd && !used.contains(&p.pid))?;
            used.insert(m.pid);
            // As in Swift a fresh session: start, digest, watching and background-only go back to their defaults.
            Some(Session {
                waiting: s.waiting,
                ..Session::new(
                    s.id.clone(),
                    s.engine,
                    Some(m.pid),
                    s.name.clone(),
                    s.cwd.clone(),
                    s.ready,
                    true,
                )
            })
        })
        .collect()
}

/// Puts the sessions into their windows. Windows without a session stay untouched (design: only AI windows move).
pub fn match_sessions(
    windows: &mut [TermWindow],
    sessions: &[Session],
    pid_windows: &BTreeMap<Pid, u64>,
) {
    for w in windows.iter_mut() {
        w.sessions.clear();
    }
    for s in sessions {
        let idx = s
            .pid
            .and_then(|pid| pid_windows.get(&pid))
            .and_then(|&id| index_of_window(id, windows));
        if let Some(idx) = idx {
            windows[idx].sessions.push(s.clone());
        }
    }
}

#[cfg(test)]
mod tests;
