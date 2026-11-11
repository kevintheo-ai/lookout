//! Codex, Grok, Kimi: turn end from the session logs. Spike 2026-09-18: all three log the start AND end of a turn
//! explicitly, so the last turn event is read instead of guessing whether "nothing written for a while" means done.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use serde_json::Value;

use super::transcript::{TAIL_BYTES, first_json_line, local, tail_string};
use super::{dir_names, now, percent_decode, secs, split_lines};
use crate::mapping::last_path_component;
use crate::model::{Engine, EngineProc, Pid, Session, Time};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnState {
    Working,
    Done,
    Waiting,
    /// Broken off (Esc): Codex writes `turn_aborted` (reason "interrupted", seen 2026-11-10).
    Aborted,
    Unknown,
}

/// The event that says the whole turn was broken off. Only where seen: Codex `turn_aborted`. Grok and Kimi were not
/// seen breaking off a turn; a pattern would also catch a request broken off and retried inside a working turn
/// (`llm.request.aborted`, review quick 2026-11-10, Codex).
fn aborted_event(engine: Engine) -> Option<&'static str> {
    match engine {
        Engine::Codex => Some("turn_aborted"),
        _ => None,
    }
}

/// Events with which an engine reports a QUESTION. Measured 2026-09-18: Grok writes `permission_requested` /
/// `permission_resolved`; Codex and Kimi name their approval events similarly but were not observed (their runs were
/// auto-approved). So patterns instead of fixed names.
fn waiting_marker(kind: &str) -> Option<bool> {
    let t = kind.to_lowercase();
    if !(t.contains("permission") || t.contains("approval") || t.contains("confirmation")) {
        return None;
    }
    if t.contains("request") {
        return Some(true);
    }
    ["resolved", "response", "granted", "denied", "answer"]
        .iter()
        .any(|w| t.contains(w))
        .then_some(false)
}

/// Start and end events of a turn.
fn turn_markers(engine: Engine) -> Option<(&'static str, &'static str)> {
    match engine {
        Engine::Codex => Some(("task_started", "task_complete")),
        Engine::Grok => Some(("turn_started", "turn_ended")),
        Engine::Kimi => Some(("agent.turn.started", "agent.turn.ended")),
        Engine::Claude => None,
    }
}

/// Last turn event in a JSONL tail. Read from the back, broken lines are skipped (the tail usually starts mid-line).
pub fn last_turn_state(text: &str, engine: Engine) -> TurnState {
    let Some((start, end)) = turn_markers(engine) else {
        return TurnState::Unknown;
    };
    for line in split_lines(text).collect::<Vec<_>>().into_iter().rev() {
        let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let mut types: Vec<&str> = vec![];
        if let Some(t) = obj.get("type").and_then(Value::as_str) {
            types.push(t);
        }
        if let Some(t) = obj
            .get("payload")
            .and_then(Value::as_object)
            .and_then(|p| p.get("type"))
            .and_then(Value::as_str)
        {
            types.push(t);
        }
        // Questions first: they beat every other state.
        if let Some(request) = types.iter().find_map(|t| waiting_marker(t)) {
            return if request {
                TurnState::Waiting
            } else {
                TurnState::Working
            };
        }
        if aborted_event(engine).is_some_and(|a| types.contains(&a)) {
            return TurnState::Aborted;
        }
        if types.contains(&end) {
            return TurnState::Done;
        }
        if types.contains(&start) {
            return TurnState::Working;
        }
    }
    TurnState::Unknown
}

/// ponytail: reads only the file's end; rollout files grow several MB and the last turn event is always at the back.
/// Without one there, the time rule applies.
pub fn last_turn_state_in_file(path: &str, engine: Engine, tail_bytes: u64) -> TurnState {
    tail_string(path, tail_bytes).map_or(TurnState::Unknown, |t| last_turn_state(&t, engine))
}

/// Last resort when no turn event is found (plan task 4): quiet beyond the threshold AND no process signal. Known
/// limit: silent thinking can fake done.
pub fn classify_log_ready(
    last_write: Time,
    now: Time,
    threshold_sec: u64,
    proc_active: bool,
) -> bool {
    !proc_active && now - last_write >= threshold_sec as f64
}

/// `Date.distantPast`, for files without a modification time.
const DISTANT_PAST: Time = -62_135_769_600.0;

/// Modification time like `attributesOfItem` (a symbolic link's own time).
fn modified(path: &str) -> Time {
    fs::symlink_metadata(path)
        .and_then(|m| m.modified())
        .map_or(DISTANT_PAST, secs)
}

fn subdirectories(path: &str) -> Vec<String> {
    dir_names(path)
        .into_iter()
        .map(|n| format!("{path}/{n}"))
        .filter(|p| fs::metadata(p).is_ok_and(|m| m.is_dir()))
        .collect()
}

fn session(
    engine: Engine,
    id: &str,
    pid: Pid,
    cwd: &str,
    path: &str,
    threshold_sec: u64,
) -> Session {
    let turn = last_turn_state_in_file(path, engine, TAIL_BYTES);
    let (ready, waiting) = match turn {
        TurnState::Done | TurnState::Aborted => (true, false),
        TurnState::Waiting => (true, true),
        TurnState::Working => (false, false),
        TurnState::Unknown => (
            classify_log_ready(modified(path), now(), threshold_sec, false),
            false,
        ),
    };
    Session {
        waiting,
        stopped: turn == TurnState::Aborted,
        ..Session::new(
            format!("{}:{id}", engine.raw()),
            engine,
            Some(pid),
            last_path_component(cwd),
            cwd,
            ready,
            true,
        )
    }
}

struct LogFile {
    path: String,
    id: String,
    cwd: String,
    modified: Time,
}

impl LogFile {
    /// Creation of the session: it comes with its process start (pairing when several processes share a folder).
    fn created(&self) -> Time {
        fs::symlink_metadata(&self.path)
            .and_then(|m| m.created())
            .map_or(self.modified, secs)
    }
}

/// Codex/Grok/Kimi sessions of the RUNNING processes: per engine and working folder the newest log files, as many as
/// processes run there, without an age limit (until 2026-09-25 "file younger than 6 h" let a turn finished in the
/// evening fall out overnight, GLM + Kimi). Without a matching process nothing is read.
pub struct LogSessionFinder {
    home: String,
    start_of: Box<dyn Fn(Pid) -> Option<Time> + Send>,
    /// Working folder + session id per header file (Codex: first line, Kimi: state.json); they never change.
    /// ponytail: grows with every file ever seen (~800 short entries); thin out if it ever matters.
    meta: BTreeMap<String, (String, String)>,
}

impl LogSessionFinder {
    /// `start_of`: process start (stage 2: `sysctl KERN_PROC_PID`).
    pub fn new(
        home: String,
        start_of: Box<dyn Fn(Pid) -> Option<Time> + Send>,
    ) -> LogSessionFinder {
        LogSessionFinder {
            home,
            start_of,
            meta: BTreeMap::new(),
        }
    }

    pub fn sessions(&mut self, procs: &[EngineProc], threshold_sec: u64) -> Vec<Session> {
        let mut out = vec![];
        for engine in [Engine::Codex, Engine::Grok, Engine::Kimi] {
            let live: Vec<&EngineProc> = procs
                .iter()
                .filter(|p| p.engine == engine && !p.cwd.is_empty())
                .collect();
            if live.is_empty() {
                continue;
            }
            let mut started: BTreeMap<Pid, Option<Time>> = BTreeMap::new();
            for p in &live {
                started
                    .entry(p.pid)
                    .or_insert_with(|| (self.start_of)(p.pid));
            }
            let mut per_cwd: BTreeMap<&str, Vec<&EngineProc>> = BTreeMap::new();
            for p in &live {
                per_cwd.entry(p.cwd.as_str()).or_default().push(p);
            }
            let since = started.values().flatten().copied().reduce(f64::min);
            let cwds: BTreeSet<String> = per_cwd.keys().map(|c| c.to_string()).collect();
            let files = match engine {
                Engine::Codex => self.codex_files(&cwds, since),
                Engine::Grok => grok_files(&self.home, &cwds),
                Engine::Kimi => self.kimi_files(&cwds),
                Engine::Claude => vec![],
            };
            for (cwd, here) in per_cwd {
                // One session per process, paired by age: the file created first belongs to the process started first
                // (review quick 2026-09-25, GLM: otherwise two terminals in one folder swapped their state).
                let mut newest: Vec<&LogFile> = files.iter().filter(|f| f.cwd == cwd).collect();
                newest.sort_by(|a, b| b.modified.total_cmp(&a.modified));
                newest.truncate(here.len());
                newest.sort_by(|a, b| a.created().total_cmp(&b.created()));
                let mut by_start = here.clone();
                // Unknown start = `distantPast`, before every known one.
                let key = |p: &EngineProc| started[&p.pid].unwrap_or(DISTANT_PAST);
                by_start.sort_by(|a, b| key(a).total_cmp(&key(b)));
                for (f, p) in newest.into_iter().zip(by_start) {
                    out.push(session(engine, &f.id, p.pid, cwd, &f.path, threshold_sec));
                }
            }
        }
        out
    }

    /// Header data read once and remembered, only with a working folder: a freshly created, still empty file is read
    /// again next time instead of counting as "without folder" forever.
    fn cached_meta(
        &mut self,
        path: &str,
        read: impl FnOnce() -> (String, String),
    ) -> (String, String) {
        if let Some(m) = self.meta.get(path) {
            return m.clone();
        }
        let m = read();
        if !m.0.is_empty() {
            self.meta.insert(path.to_owned(), m.clone());
        }
        m
    }

    /// Rollouts lie by start day in `sessions/YYYY/MM/DD/` (local date): searched from the day before the oldest process
    /// start (time zone edge) to today, at most 60 days back; without a known start today and yesterday.
    fn codex_files(&mut self, cwds: &BTreeSet<String>, since: Option<Time>) -> Vec<LogFile> {
        let now = now();
        let back = (((now - since.unwrap_or(now)) / 86_400.0) as i64 + 1).clamp(1, 60);
        let mut out = vec![];
        for day in 0..=back {
            let Some(date) = local(now - day as f64 * 86_400.0) else {
                continue;
            };
            let dir = format!("{}/.codex/sessions/{}", self.home, date.format("%Y/%m/%d"));
            for name in dir_names(&dir)
                .into_iter()
                .filter(|n| n.ends_with(".jsonl"))
            {
                let path = format!("{dir}/{name}");
                let (cwd, id) = self.cached_meta(&path, || {
                    let payload = first_json_line(&path, 65_536)
                        .and_then(|l| l.get("payload")?.as_object().cloned())
                        .unwrap_or_default();
                    let s = |k: &str| payload.get(k).and_then(Value::as_str);
                    (
                        s("cwd").unwrap_or("").to_owned(),
                        s("session_id").unwrap_or(&name).to_owned(),
                    )
                });
                if cwds.contains(&cwd) {
                    out.push(LogFile {
                        modified: modified(&path),
                        path,
                        id,
                        cwd,
                    });
                }
            }
        }
        out
    }

    /// One `wd_…` folder per working folder, one session per subfolder; folder and id in `state.json`, the turn events in
    /// `agents/main/wire.jsonl`.
    fn kimi_files(&mut self, cwds: &BTreeSet<String>) -> Vec<LogFile> {
        let mut out = vec![];
        let sessions: Vec<String> = subdirectories(&format!("{}/.kimi-code/sessions", self.home))
            .iter()
            .flat_map(|d| subdirectories(d))
            .collect();
        for s in sessions {
            let state_path = format!("{s}/state.json");
            let (cwd, id) = self.cached_meta(&state_path, || {
                let obj = fs::read(&state_path)
                    .ok()
                    .and_then(|data| match serde_json::from_slice::<Value>(&data) {
                        Ok(Value::Object(o)) => Some(o),
                        _ => None,
                    })
                    .unwrap_or_default();
                let str_of = |k: &str| obj.get(k).and_then(Value::as_str);
                (
                    str_of("cwd").unwrap_or("").to_owned(),
                    str_of("id").unwrap_or(last_path_component(&s)).to_owned(),
                )
            });
            if !cwds.contains(&cwd) {
                continue;
            }
            let wire = format!("{s}/agents/main/wire.jsonl");
            let path = if fs::metadata(&wire).is_ok() {
                wire
            } else {
                state_path.clone()
            };
            out.push(LogFile {
                modified: modified(&state_path).max(modified(&path)),
                path,
                id,
                cwd,
            });
        }
        out
    }
}

/// Folder name = percent-encoded working folder, one session per subfolder.
fn grok_files(home: &str, cwds: &BTreeSet<String>) -> Vec<LogFile> {
    let mut out = vec![];
    for dir in subdirectories(&format!("{home}/.grok/sessions")) {
        let cwd = percent_decode(last_path_component(&dir)).unwrap_or_default();
        if !cwds.contains(&cwd) {
            continue;
        }
        for s in subdirectories(&dir) {
            let path = format!("{s}/events.jsonl");
            if fs::metadata(&path).is_err() {
                continue;
            }
            out.push(LogFile {
                modified: modified(&path),
                id: last_path_component(&s).to_owned(),
                cwd: cwd.clone(),
                path,
            });
        }
    }
    out
}
