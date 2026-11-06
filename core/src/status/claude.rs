//! Claude sessions. Claude reports its status officially (`claude agents --json`); sessions of a SECOND configuration
//! appear there only with CLAUDE_CONFIG_DIR set, so it is asked once per configuration folder (spike 2026-09-18). Since
//! 2026-09-27 Lookout reads the status files `claude agents` takes its interactive sessions from directly; the query
//! itself runs every 5 minutes only (crash diagnosis: 13.4 starts per minute).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{Obj, Report, as_double, as_int, objects};
use crate::mapping::last_path_component;
use crate::model::{Engine, Pid, Session, Time};

/// Background agents of the last 24 h are listed "without window"; older ones only count.
pub const MAX_AGENT_AGE: Time = 86_400.0;

/// "Without window" in the menu (Kevin 2026-09-18: "Nur Echtes"): running sessions without a visible window (say
/// background tabs) and background agents of the last 24 h. Ended Codex/Kimi/Grok runs are not in `live` at all;
/// older agents only count. Returns the shown sessions and the number of stale agents.
pub fn windowless_sessions(
    live: &[Session],
    in_windows: &BTreeSet<String>,
    now: Time,
) -> (Vec<Session>, usize) {
    let mut shown = vec![];
    let mut stale = 0;
    for s in live.iter().filter(|s| !in_windows.contains(&s.id)) {
        // Without pid = background agent: no terminal, so never a window.
        if s.pid.is_none() && !s.started_at.is_some_and(|t| now - t <= MAX_AGENT_AGE) {
            stale += 1;
        } else {
            shown.push(s.clone());
        }
    }
    (shown, stale)
}

/// The Claude CLI, first executable of the usual places.
pub fn claude_binary(home: &str) -> Option<String> {
    [
        format!("{home}/.local/bin/claude"),
        "/opt/homebrew/bin/claude".into(),
        "/usr/local/bin/claude".into(),
    ]
    .into_iter()
    .find(|p| is_executable(p))
}

/// `isExecutableFile(atPath:)`. ponytail: any execute bit, not the access check for this user.
#[cfg(unix)]
fn is_executable(path: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &str) -> bool {
    Path::new(path).is_file()
}

/// The output of `claude agents --json` as sessions.
pub fn parse_agents_json(data: &[u8]) -> Vec<Session> {
    serde_json::from_slice::<Value>(data)
        .ok()
        .as_ref()
        .and_then(objects)
        .map(|entries| entries.into_iter().filter_map(parse_agent).collect())
        .unwrap_or_default()
}

/// An entry of `claude agents --json` or a status file (`read_session_files`), same fields. Entries without
/// `sessionId` are dropped: better one session less than a guessed one.
pub fn parse_agent(obj: &Obj) -> Option<Session> {
    let s = |k: &str| obj.get(k).and_then(Value::as_str);
    let sid = s("sessionId").filter(|sid| !sid.is_empty())?;
    let kind = s("kind").unwrap_or("");
    let status = s("status").unwrap_or("");
    let (ready, waiting) = if kind == "background" {
        // "blocked" = the agent hangs on an approval or question: Kevin's turn.
        let waiting = s("state") == Some("blocked");
        (waiting, waiting)
    } else {
        // Measured live 2026-09-18 with an open question: interactive sessions report their own status `waiting`.
        // `idle` = turn ended, `waiting` = waits for Kevin's answer. Both mean: not working.
        let waiting = status == "waiting";
        (status == "idle" || waiting, waiting)
    };
    // Background agents have no terminal and no window: they run without pid, even if `claude agents` reports one
    // (review quick 2026-09-27, Kimi).
    let pid = if kind == "background" {
        None
    } else {
        obj.get("pid")
            .and_then(as_int)
            .and_then(|p| Pid::try_from(p).ok())
    };
    Some(Session {
        waiting,
        started_at: obj
            .get("startedAt")
            .and_then(as_double)
            .map(|ms| ms / 1000.0),
        background_only: kind != "background" && status == "shell",
        ..Session::new(
            format!("{}:{sid}", Engine::Claude.raw()),
            Engine::Claude,
            pid,
            s("name").unwrap_or(sid),
            s("cwd").unwrap_or(""),
            ready,
            true,
        )
    })
}

/// Status files of a configuration folder: `<dir>/sessions/<pid>.json`, one per running session, written by Claude
/// (fields as in `claude agents --json`, `status` busy/shell/idle/waiting; checked 2026-09-27: 5/5 identical).
/// Background agents are NOT in them. `None` = folder missing, or files there but none readable (format changed?);
/// then the old query applies. `start_of`: start of a process (stage 2: `sysctl KERN_PROC_PID`), `None` = no such
/// process.
pub fn read_session_files(
    dir: &str,
    start_of: &dyn Fn(Pid) -> Option<Time>,
) -> Option<Vec<Session>> {
    let folder = format!("{dir}/sessions");
    let files: Vec<String> = std::fs::read_dir(&folder)
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".json"))
        .collect();
    let parse = |name: &str| -> Option<Session> {
        let data = std::fs::read(format!("{folder}/{name}")).ok()?;
        let Ok(Value::Object(obj)) = serde_json::from_slice::<Value>(&data) else {
            return None;
        };
        obj.get("pid").and_then(as_int)?;
        parse_agent(&obj)
    };
    let mut parsed = 0;
    let mut sessions = vec![];
    for name in &files {
        // Claude rewrites the file in place (same inode, measured 2026-09-27): who reads exactly between emptying and
        // writing sees nothing whole; a second try milliseconds later sees the finished file.
        let Some(s) = parse(name).or_else(|| {
            thread::sleep(Duration::from_millis(10));
            parse(name)
        }) else {
            continue;
        };
        parsed += 1;
        // Left-over file (session crashed, pid given to a newer process since, also a root process): the process is
        // younger than the entry. 2 s slack for the clocks.
        if let (Some(pid), Some(started)) = (s.pid, s.started_at) {
            if start_of(pid).is_some_and(|p| p - started > 2.0) {
                continue;
            }
        }
        sessions.push(s);
    }
    (files.is_empty() || parsed > 0).then_some(sessions)
}

/// Runs a command with a hard time limit (seconds). `None` = failure or timeout. `accept`: exit codes that are no
/// failure (lsof ends with 1 when nothing matches). The environment is inherited; `env` sets, `unset` removes.
/// Differences to Swift: stdin is /dev/null, the time limit kills (SIGKILL, Swift: SIGTERM), and a grandchild that keeps
/// the output pipe open cannot hold the caller past the limit.
pub fn run_command(
    path: &str,
    args: &[&str],
    env: &BTreeMap<String, String>,
    unset: &[String],
    timeout: f64,
    accept: &[i32],
    log: &mut dyn FnMut(&str),
) -> Option<Vec<u8>> {
    let mut cmd = Command::new(path);
    // stderr to /dev/null: a pipe nobody reads fills at 64 KB and blocks the child.
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for k in unset {
        cmd.env_remove(k);
    }
    cmd.envs(env);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log(&format!("Watcher: {path} nicht startbar — {e}"));
            return None;
        }
    };
    // The time limit MUST hang on the process, not on the reading: reading blocks until the child closes its pipe.
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    let (tx, rx) = mpsc::channel();
    if let Some(mut out) = child.stdout.take() {
        thread::spawn(move || {
            let mut data = vec![];
            let _ = out.read_to_end(&mut data);
            let _ = tx.send(data);
        });
    }
    let data = rx
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .ok();
    let status = data.as_ref().and_then(|_| {
        loop {
            match child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                _ => break None,
            }
        }
    });
    let Some(status) = status else {
        let _ = child.kill();
        let _ = child.wait();
        log(&format!(
            "Watcher: {path} {} ueberschritt {}s — abgebrochen",
            args.join(" "),
            timeout as i64
        ));
        return None;
    };
    // Ended by a signal: never a result, even if the signal number were in `accept`.
    status.code().filter(|c| accept.contains(c)).and(data)
}

/// The official query for ONE configuration folder (starts `claude`, ~0.25 s CPU). `None` = not possible.
pub fn fetch_agents(dir: &str, home: &str, log: &mut dyn FnMut(&str)) -> Option<Vec<Session>> {
    let Some(bin) = claude_binary(home) else {
        log("Watcher: kein claude-Binary gefunden");
        return None;
    };
    let (set, unset) = agents_environment(dir, home);
    run_command(&bin, &["agents", "--json"], &set, &unset, 5.0, &[0], log)
        .and_then(|data| agents_reply(&data))
}

/// Environment for `claude agents` per configuration folder (variables to set, variables to remove): the default
/// folder WITHOUT CLAUDE_CONFIG_DIR, with it claude created a second configuration file there. Removed actively, Lookout
/// inherits its starter's environment. Compared after resolving links: a symlink to `~/.claude` is the default folder.
pub fn agents_environment(dir: &str, home: &str) -> (BTreeMap<String, String>, Vec<String>) {
    let real = |p: &str| {
        std::fs::canonicalize(p).map_or_else(|_| p.to_owned(), |r| r.to_string_lossy().into_owned())
    };
    if real(dir) == real(&format!("{home}/.claude")) {
        (BTreeMap::new(), vec!["CLAUDE_CONFIG_DIR".into()])
    } else {
        (
            BTreeMap::from([("CLAUDE_CONFIG_DIR".to_owned(), dir.to_owned())]),
            vec![],
        )
    }
}

/// Answer of `claude agents --json`: `None` instead of an empty list when it is no use (no JSON array, or entries
/// there but none readable). Otherwise the cross-check would take every file session for unknown.
pub fn agents_reply(data: &[u8]) -> Option<Vec<Session>> {
    let Ok(Value::Array(entries)) = serde_json::from_slice::<Value>(data) else {
        return None;
    };
    let sessions = parse_agents_json(data);
    (entries.is_empty() || !sessions.is_empty()).then_some(sessions)
}

/// Swift's `"\(names)"` for an array of strings: `["a", "b\'s"]`, escaped like `String.debugDescription` (measured
/// 2026-11-06: backslash, both quotes, \0 \t \n \r, other ASCII controls as \u{XX}, a combining mark at the start).
fn swift_list<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let items: Vec<String> = names
        .map(|name| {
            let mut out = String::from("\"");
            for (i, c) in name.chars().enumerate() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '"' => out.push_str("\\\""),
                    '\'' => out.push_str("\\'"),
                    '\0' => out.push_str("\\0"),
                    '\t' => out.push_str("\\t"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    c if c.is_ascii_control()
                        || (i == 0 && matches!(c as u32, 0x0300..=0x036F)) =>
                    {
                        let hex = format!("{:X}", c as u32);
                        let pad = if hex.len() % 2 == 1 { "0" } else { "" };
                        out.push_str(&format!("\\u{{{pad}{hex}}}"));
                    }
                    c => out.push(c),
                }
            }
            out.push('"');
            out
        })
        .collect();
    format!("[{}]", items.join(", "))
}

/// The official query for one configuration folder (`fetch_agents`).
pub type AgentsQuery = Box<dyn FnMut(&str) -> Option<Vec<Session>> + Send>;

/// Claude sessions of all configurations without constant program starts (Kevin 2026-09-27). Interactive sessions come
/// from the status files (`read_session_files`), the official query runs every 5 minutes only, for the background
/// agents ("without window") and as a cross-check with the files. If a folder's files are missing or no use, Lookout
/// asks there as before in every session tick, until the next cross-check fits again.
pub struct ClaudeStatus {
    dirs: Vec<String>,
    agents: AgentsQuery,
    alive: Box<dyn Fn(Pid) -> bool + Send>,
    start_of: Box<dyn Fn(Pid) -> Option<Time> + Send>,
    report: Report,
    last_official: Option<Time>,
    /// Background agents per folder from the last query (failed = empty, else they would stand forever).
    background: BTreeMap<String, Vec<Session>>,
    /// Folders whose files did not fit at the cross-check: the old query applies there.
    distrusted: BTreeSet<String>,
    /// Folders where a state differed at the last cross-check.
    differed: BTreeSet<String>,
    last_note: BTreeMap<String, String>,
}

impl ClaudeStatus {
    pub const OFFICIAL_EVERY: Time = 300.0;
    /// File sessions that started shortly before the query do not count in the reverse check: they may have begun
    /// after it.
    pub const YOUNG_FILE: Time = 60.0;

    /// `agents`: the official query per folder (`fetch_agents`); `alive`: does a pid live (stage 2: `kill(pid, 0)`,
    /// EPERM counts as alive, pid ≤ 0 never); `start_of`: process start (stage 2: `sysctl KERN_PROC_PID`).
    pub fn new(
        dirs: Vec<String>,
        agents: AgentsQuery,
        alive: Box<dyn Fn(Pid) -> bool + Send>,
        start_of: Box<dyn Fn(Pid) -> Option<Time> + Send>,
        report: Report,
    ) -> ClaudeStatus {
        ClaudeStatus {
            dirs,
            agents,
            alive,
            start_of,
            report,
            last_official: None,
            background: BTreeMap::new(),
            distrusted: BTreeSet::new(),
            differed: BTreeSet::new(),
            last_note: BTreeMap::new(),
        }
    }

    pub fn sessions(&mut self, now: Time) -> Vec<Session> {
        let official = self
            .last_official
            .is_none_or(|t| now - t >= Self::OFFICIAL_EVERY);
        if official {
            self.last_official = Some(now);
        }
        let mut seen = BTreeSet::new();
        let mut result = vec![];
        for dir in self.dirs.clone() {
            if !Path::new(&dir).exists() {
                continue;
            }
            // First the query, then the files: a session ending in between is missing only in the files; its pid is
            // dead then and does not count in the cross-check.
            let mut asked = if official { (self.agents)(&dir) } else { None };
            let mut files = read_session_files(&dir, &*self.start_of);
            if let (Some(a), Some(f)) = (&asked, &files) {
                self.compare(&dir, a, f, now);
            }
            let problem = if files.is_none() {
                Some("Status-Dateien fehlen oder unlesbar")
            } else if self.distrusted.contains(&dir) {
                Some("Status-Dateien weichen von claude agents ab")
            } else {
                None
            };
            let text = problem.map_or_else(
                || "Status aus den Dateien".to_owned(),
                |p| format!("{p} — alte Abfrage in jedem Takt"),
            );
            self.note(&dir, &text);
            if problem.is_some() {
                files = None;
                if asked.is_none() {
                    asked = (self.agents)(&dir);
                }
            }
            if official {
                let agents = asked.iter().flatten().filter(|s| s.pid.is_none()).cloned();
                self.background.insert(dir.clone(), agents.collect());
            }
            let live = files.unwrap_or_else(|| {
                asked
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|s| s.pid.is_some())
                    .collect()
            });
            let agents = self.background.get(&dir).cloned().unwrap_or_default();
            for s in live.into_iter().chain(agents) {
                if seen.insert(s.id.clone()) {
                    result.push(s);
                }
            }
        }
        result
    }

    /// Cross-check query against files, both ways: a session missing in the files whose process still lives, a living
    /// one in the files that `claude agents` does not know (Kevin 2026-09-28), or a state that differs in two
    /// cross-checks in a row (a renamed status field would leave every session "working"; review quick 2026-09-27,
    /// GLM): then the old query applies for this folder until the next cross-check fits. A single state difference only
    /// goes to the log: the two reads are fractions of a second apart.
    fn compare(&mut self, dir: &str, asked: &[Session], files: &[Session], asked_at: Time) {
        let mut by_id: BTreeMap<&str, &Session> = BTreeMap::new();
        for f in files {
            by_id.entry(f.id.as_str()).or_insert(f);
        }
        let listed: BTreeSet<&str> = asked.iter().map(|s| s.id.as_str()).collect();
        let alive = |s: &Session| s.pid.is_some_and(|p| (self.alive)(p));
        let running: Vec<&Session> = asked.iter().filter(|s| alive(s)).collect();
        let missing: Vec<&Session> = running
            .iter()
            .copied()
            .filter(|s| !by_id.contains_key(s.id.as_str()))
            .collect();
        let extra: Vec<&Session> = files
            .iter()
            .filter(|f| {
                !listed.contains(f.id.as_str())
                    && alive(f)
                    && f.started_at.is_none_or(|t| t < asked_at - Self::YOUNG_FILE)
            })
            .collect();
        let differs: Vec<&Session> = running
            .iter()
            .copied()
            .filter(|a| {
                by_id
                    .get(a.id.as_str())
                    .is_some_and(|f| f.ready != a.ready || f.waiting != a.waiting)
            })
            .collect();
        let persistent = !differs.is_empty() && self.differed.contains(dir);
        if differs.is_empty() {
            self.differed.remove(dir);
        } else {
            self.differed.insert(dir.to_owned());
        }
        if missing.is_empty() && extra.is_empty() && !persistent {
            self.distrusted.remove(dir);
        } else {
            self.distrusted.insert(dir.to_owned());
        }
        if !missing.is_empty() || !extra.is_empty() || !differs.is_empty() {
            let names = |v: &[&Session]| swift_list(v.iter().map(|s| s.name.as_str()));
            let line = format!(
                "Abgleich {}: fehlt in den Dateien {}, nur in den Dateien {}, Zustand weicht ab {}",
                last_path_component(dir),
                names(&missing),
                names(&extra),
                names(&differs)
            );
            (self.report)(&line);
        }
    }

    /// One line per change (and at the start), not per tick.
    fn note(&mut self, dir: &str, text: &str) {
        if self.last_note.get(dir).map(String::as_str) == Some(text) {
            return;
        }
        self.last_note.insert(dir.to_owned(), text.to_owned());
        let line = format!("Claude-Status {}: {text}", last_path_component(dir));
        (self.report)(&line);
    }
}
