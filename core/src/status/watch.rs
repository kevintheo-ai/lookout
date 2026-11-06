//! "Watching" instead of done (Kevin 2026-09-27: "er überwacht jetzt nur die Sessions"): an answer that is finished
//! but waits for its own background watcher (`BackgroundWatch`, plan 1e+) or only for another session or its own timer
//! (`Standby`) does not need Kevin.

use std::collections::{BTreeMap, BTreeSet};

use super::Report;
use super::words::{
    asks_nothing, opens_with_nothing_needed, promises_report, waits_with_nothing_needed,
};
use crate::mapping::last_path_component;
use crate::model::{Digest, Engine, LineStatus, Pid, PsRow, Session, Time};

/// Did Kevin have the window in front of him after the last answer (Kevin 2026-10-04, screenshot 22:38)? Then he read
/// what came on his message; the answer needs him only if it wants something itself. Without the answer's time: when in
/// doubt not seen.
pub fn answer_seen(d: &Digest, seen_at: Option<Time>) -> bool {
    match (seen_at, d.answer_at) {
        (Some(seen), Some(answered)) => seen >= answered,
        _ => false,
    }
}

// ---------- Waits for another session or a wake-up (Kevin 2026-09-27) ----------

/// Done Claude sessions that only wait for another running session or their own timer and say so explicitly:
/// "watching" instead of done (as the watcher in plan 1e+, same display). Everything else stays as it is.
pub struct Standby {
    other_window_names: Vec<String>,
    report: Report,
    /// Sessions that count as waiting right now (id -> name), for one log line per change.
    quiet: BTreeMap<String, String>,
}

impl Standby {
    /// A timer counts as pending until 5 minutes after its time: firing and the new turn take a moment.
    pub const WAKE_GRACE: Time = 300.0;

    /// `other_window_names` from the configuration.
    pub fn new(other_window_names: Vec<String>, report: Report) -> Standby {
        Standby {
            other_window_names,
            report,
            quiet: BTreeMap::new(),
        }
    }

    /// `seen`: when Kevin last had which session's window in front of him (R10). If Kevin's own message started the
    /// turn, it no longer counts once he has seen the answer (R11 B1, Kevin 2026-10-05), but grey still only with an
    /// explicit "nothing needed" (B1 strict).
    pub fn adjust(
        &mut self,
        sessions: &[Session],
        now: Time,
        seen: &BTreeMap<String, Time>,
    ) -> Vec<Session> {
        // The other session lives when it is among the running Claude sessions (status files), not merely when some
        // process carries the pid (review quick 2026-09-27, GLM). Without pid (own SendMessage) the name counts. A
        // session with that pid that started after the message is not the other session (reused pid, review quick
        // 2026-10-10, Codex).
        let live: Vec<&Session> = sessions
            .iter()
            .filter(|s| s.engine == Engine::Claude && s.pid.is_some())
            .collect();
        let mut started: BTreeMap<Pid, Option<Time>> = BTreeMap::new();
        for s in &live {
            if let Some(pid) = s.pid {
                started.entry(pid).or_insert(s.started_at);
            }
        }
        let peer_alive = |pid: Pid, at: Option<Time>| match started.get(&pid) {
            None => false,
            Some(Some(start)) => at.is_none_or(|at| *start <= at + 2.0),
            Some(None) => true,
        };
        let names: BTreeSet<&str> = live.iter().map(|s| s.name.as_str()).collect();
        let mut out = sessions.to_vec();
        let mut now_quiet = BTreeMap::new();
        for s in &mut out {
            let Some(d) = s.digest.as_ref() else {
                continue;
            };
            if s.engine != Engine::Claude
                || !s.ready
                || s.waiting
                || s.watching
                || !d.turn_ended
                || d.question.is_some()
            {
                continue;
            }
            // The Lookout line goes before the word list (Kevin 2026-10-12): "needs nothing" holds, also on Kevin's
            // message.
            let line = d.line(&self.other_window_names);
            let nothing = match &line {
                Some(l) => l.status == LineStatus::Nothing,
                None => {
                    (d.user_turn != Some(true) || answer_seen(d, seen.get(&s.id).copied()))
                        && d.text.as_deref().is_some_and(waits_with_nothing_needed)
                }
            };
            if !nothing {
                continue;
            }
            // A successor with the same name (new lead session) counts too, even if the checked pid has ended.
            let line_nothing = line
                .as_ref()
                .is_some_and(|l| l.status == LineStatus::Nothing);
            let peer = d.peer_pid.is_some_and(|p| peer_alive(p, d.peer_at))
                || d.peer_name.as_deref().is_some_and(|n| names.contains(n))
                || (line_nothing && d.partners.iter().any(|p| names.contains(p.as_str())));
            let wake = d
                .wakeups
                .values()
                .any(|at| at.is_none_or(|at| now < at + Self::WAKE_GRACE));
            if !(peer || wake) {
                continue;
            }
            let why = if line.is_some() {
                " (Lookout-Zeile)"
            } else if d.user_turn == Some(true) {
                " (Antwort gesehen)"
            } else {
                ""
            };
            s.ready = false;
            s.watching = true;
            now_quiet.insert(s.id.clone(), s.name.clone());
            if !self.quiet.contains_key(&s.id) {
                let what = if peer {
                    "eine andere Session"
                } else {
                    "einen Wecker"
                };
                (self.report)(&format!(
                    "Wartet nur noch auf {what}, nichts nötig{why}: {} gilt als überwacht",
                    s.name
                ));
            }
        }
        for (id, name) in &self.quiet {
            if !now_quiet.contains_key(id) && out.iter().any(|s| &s.id == id && s.ready) {
                (self.report)(&format!("Wartet nicht mehr: {name} gilt als fertig"));
            }
        }
        self.quiet = now_quiet;
        out
    }
}

// ---------- Done despite a background command (plan 1e, Kevin's rule 2026-09-26 evening) ----------
//
// Claude reports `shell` while a background command of the session runs after the finished answer. Kevin: if the
// command works, the session runs; if it is only an observer or something resting, it is done. Kevin 2026-09-27 (lead
// session): waiting for its own watcher (CI run, worker) does not need Kevin: "watching" instead of done. The
// difference: a server listens on a port, a watcher does not.

/// Background jobs of a Claude session: per shell child (zsh/bash/sh) the shell with everything below it. MCP servers
/// and `caffeinate` are no shells and do not count.
pub fn background_tasks(claude: Pid, rows: &[PsRow]) -> Vec<BTreeSet<Pid>> {
    let mut children: BTreeMap<Pid, Vec<&PsRow>> = BTreeMap::new();
    for r in rows {
        children.entry(r.ppid).or_default().push(r);
    }
    let is_shell = |r: &PsRow| {
        let name = last_path_component(&r.comm).trim_start_matches('-');
        matches!(name, "zsh" | "bash" | "sh")
    };
    children
        .get(&claude)
        .into_iter()
        .flatten()
        .filter(|r| is_shell(r))
        .map(|shell| {
            let mut tree = BTreeSet::new();
            let mut todo = vec![shell.pid];
            while let Some(pid) = todo.pop() {
                if !tree.insert(pid) {
                    continue;
                }
                todo.extend(children.get(&pid).into_iter().flatten().map(|r| r.pid));
            }
            tree
        })
        .collect()
}

/// All background processes of a Claude session (union of the jobs).
pub fn background_pids(claude: Pid, rows: &[PsRow]) -> BTreeSet<Pid> {
    background_tasks(claude, rows)
        .into_iter()
        .flatten()
        .collect()
}

/// pids from `lsof -Fp`: per process a line "p<pid>", below it "f<fd>".
pub fn parse_lsof_pids(text: &str) -> BTreeSet<Pid> {
    super::split_lines(text)
        .filter_map(|l| l.strip_prefix('p')?.parse().ok())
        .collect()
}

/// One measurement of the background commands: CPU time (s) and network bytes of open connections, per process.
#[derive(Clone, Debug, PartialEq)]
pub struct UsageSample {
    pub at: Time,
    pub cpu: BTreeMap<Pid, f64>,
    pub net: BTreeMap<Pid, u64>,
}

/// More than this much CPU time (s) in the window means the background works (plan start value).
pub const CPU_LIMIT: f64 = 1.0;
/// More than this many network bytes in the window means the background works: a command waiting for a language model
/// counts as working (plan start value).
pub const NET_LIMIT: u64 = 50_000;

/// Does the background work? Yes, if since the latest sample at least `window` old more than `CPU_LIMIT` s CPU time or
/// `NET_LIMIT` bytes network came, and as long as it has not been watched for `window` yet: only that long a rest means
/// "done". `samples` ascending by time.
pub fn background_busy(samples: &[UsageSample], now: Time, window: Time) -> bool {
    let (Some(last), Some(base)) = (
        samples.last(),
        samples.iter().rev().find(|s| now - s.at >= window),
    ) else {
        return true;
    };
    // Growth per process: a new process counts whole, a vanished one not, nothing goes negative; nettop counts open
    // connections only, a closed one drops out (measured 2026-09-26).
    let cpu: f64 = last
        .cpu
        .iter()
        .map(|(p, v)| (v - base.cpu.get(p).copied().unwrap_or(0.0)).max(0.0))
        .sum();
    let net = last.net.iter().fold(0u64, |sum, (p, v)| {
        sum.saturating_add(v.saturating_sub(base.net.get(p).copied().unwrap_or(0)))
    });
    cpu > CPU_LIMIT || net > NET_LIMIT
}

/// nettop line "name.pid,bytes_in,bytes_out," — the name may contain dots (Claude is called "2.1.283" there).
pub fn parse_nettop(text: &str) -> BTreeMap<Pid, u64> {
    let mut out: BTreeMap<Pid, u64> = BTreeMap::new();
    for line in super::split_lines(text) {
        let f: Vec<&str> = line.split(',').collect();
        let parsed = (|| {
            let (_, pid) = f.first()?.rsplit_once('.')?;
            Some((
                pid.parse::<Pid>().ok()?,
                f.get(1)?.parse::<u64>().ok()?,
                f.get(2)?.parse::<u64>().ok()?,
            ))
        })();
        if let Some((pid, bytes_in, bytes_out)) = parsed {
            let total = out.entry(pid).or_default();
            *total = total.saturating_add(bytes_in.saturating_add(bytes_out));
        }
    }
    out
}

/// Which of these processes listen on a TCP port (servers)? `None` = unknown.
pub type ListeningQuery = Box<dyn FnMut(&[Pid]) -> Option<BTreeSet<Pid>> + Send>;

/// Sample series per Claude process, only while its answer is finished and Claude reports `shell`.
pub struct BackgroundWatch {
    samples: BTreeMap<Pid, Vec<UsageSample>>,
    net_usage: Box<dyn FnMut() -> Option<BTreeMap<Pid, u64>> + Send>,
    listening: ListeningQuery,
    /// Names of sessions that lead other sessions (`lead_names` in the configuration, written exactly so).
    leads: BTreeSet<String>,
    other_window_names: Vec<String>,
    /// One log line per change, so the practice test shows why a session counts as done.
    report: Report,
    /// Sessions whose background rests right now (id -> name, watching or done).
    quiet: BTreeMap<String, (String, bool)>,
    /// Resting sessions whose answer needs Kevin (done instead of watching), only for the reason in the log line.
    for_user: BTreeSet<String>,
}

impl BackgroundWatch {
    /// `net_usage`: network bytes per process (stage 2: `/usr/bin/nettop -P -x -n -L 1 -J bytes_in,bytes_out`, time
    /// limit 3 s, `parse_nettop`); `listening`: which of these processes listen on TCP (stage 2: `/usr/sbin/lsof -nP -a
    /// -p <pids joined by ","> -iTCP -sTCP:LISTEN -Fp`, exit codes 0 and 1 accepted, time limit 3 s, `parse_lsof_pids`;
    /// no pids = empty set without a call). `None` from either = the measurement failed.
    pub fn new(
        net_usage: Box<dyn FnMut() -> Option<BTreeMap<Pid, u64>> + Send>,
        listening: ListeningQuery,
        leads: BTreeSet<String>,
        other_window_names: Vec<String>,
        report: Report,
    ) -> BackgroundWatch {
        BackgroundWatch {
            samples: BTreeMap::new(),
            net_usage,
            listening,
            leads,
            other_window_names,
            report,
            quiet: BTreeMap::new(),
            for_user: BTreeSet::new(),
        }
    }

    /// `shell` + finished answer + background resting since `window` -> watching while a watcher runs; only resting
    /// servers -> done. Everything else stays as reported, also a session without a background shell (something else
    /// keeps it busy, Lookout does not guess). Claude only. `seen`: session id -> when Kevin last had its window in
    /// front (`answer_seen`).
    pub fn adjust(
        &mut self,
        sessions: &[Session],
        rows: &[PsRow],
        now: Time,
        window: Time,
        seen: &BTreeMap<String, Time>,
    ) -> Vec<Session> {
        let out = self.decide(sessions, rows, now, window, seen);
        let mut now_quiet: BTreeMap<String, (String, bool)> = BTreeMap::new();
        for (before, after) in sessions.iter().zip(&out) {
            if after.watching || (after.ready && !before.ready) {
                now_quiet.insert(after.id.clone(), (after.name.clone(), after.watching));
            }
        }
        let seen_answers: BTreeSet<&str> = out
            .iter()
            .filter(|s| {
                s.digest.as_ref().is_some_and(|d| {
                    d.user_turn == Some(true) && answer_seen(d, seen.get(&s.id).copied())
                })
            })
            .map(|s| s.id.as_str())
            .collect();
        for (id, (name, watching)) in &now_quiet {
            if self.quiet.get(id).map(|q| q.1) == Some(*watching) {
                continue;
            }
            let why = match (
                *watching,
                seen_answers.contains(id.as_str()),
                self.for_user.contains(id),
            ) {
                (true, true, _) => "überwacht (Antwort gesehen, Aufpasser läuft)",
                (true, false, _) => "überwacht (Aufpasser läuft)",
                (false, _, true) => "fertig (Antwort an dich)",
                (false, _, false) => "fertig (nur ein Server läuft)",
            };
            (self.report)(&format!(
                "Hintergrund ruht seit {} s: {name} gilt als {why}",
                window as i64
            ));
        }
        // Only when it really runs again: if the background command (idle) or the session ends, nothing works (review
        // quick 2026-09-26, GLM + Kimi).
        let running: BTreeSet<&str> = out
            .iter()
            .filter(|s| !s.ready && !s.watching)
            .map(|s| s.id.as_str())
            .collect();
        for (id, (name, _)) in &self.quiet {
            if !now_quiet.contains_key(id) && running.contains(id.as_str()) {
                (self.report)(&format!("Hintergrund arbeitet wieder: {name}"));
            }
        }
        self.quiet = now_quiet;
        out
    }

    fn decide(
        &mut self,
        sessions: &[Session],
        rows: &[PsRow],
        now: Time,
        window: Time,
        seen: &BTreeMap<String, Time>,
    ) -> Vec<Session> {
        self.for_user.clear();
        let mut watched: BTreeMap<Pid, Vec<BTreeSet<Pid>>> = BTreeMap::new();
        // Turn end in the transcript too: the judgement is on the finished answer, not an interim state.
        for s in sessions {
            if s.engine != Engine::Claude
                || !s.background_only
                || !s.digest.as_ref().is_some_and(|d| d.turn_ended)
            {
                continue;
            }
            let Some(pid) = s.pid else {
                continue;
            };
            let tasks = background_tasks(pid, rows);
            if !tasks.is_empty() {
                watched.insert(pid, tasks);
            }
        }
        self.samples.retain(|pid, _| watched.contains_key(pid));
        // nettop only in this case; if it fails, everything stays as reported (rather "running" than wrongly "done"),
        // only who rested already keeps their state.
        if watched.is_empty() {
            return sessions.to_vec();
        }
        let Some(net) = (self.net_usage)() else {
            let at: Vec<usize> = (0..sessions.len())
                .filter(|&i| sessions[i].pid.is_some_and(|p| watched.contains_key(&p)))
                .collect();
            return self.keep_quiet(sessions.to_vec(), &at, seen);
        };
        let mut cpu: BTreeMap<Pid, f64> = BTreeMap::new();
        for r in rows {
            cpu.entry(r.pid).or_insert(r.cpu);
        }
        let mut out = sessions.to_vec();
        let mut calm: Vec<(usize, &Vec<BTreeSet<Pid>>)> = vec![];
        for (i, s) in out.iter().enumerate() {
            let Some((pid, tasks)) = s.pid.and_then(|p| watched.get_key_value(&p)) else {
                continue;
            };
            let background: BTreeSet<Pid> = tasks.iter().flatten().copied().collect();
            let list = self.samples.entry(*pid).or_default();
            list.push(UsageSample {
                at: now,
                cpu: cpu
                    .iter()
                    .filter(|(p, _)| background.contains(p))
                    .map(|(p, v)| (*p, *v))
                    .collect(),
                net: net
                    .iter()
                    .filter(|(p, _)| background.contains(p))
                    .map(|(p, v)| (*p, *v))
                    .collect(),
            });
            // Keep what the next decision needs: the latest sample before the window and everything after it.
            if let Some(base) = list.iter().rposition(|x| now - x.at >= window) {
                list.drain(..base);
            }
            if !background_busy(list, now, window) {
                calm.push((i, tasks));
            }
        }
        // Server or watcher? One query for all resting sessions; if it fails, the same as with nettop.
        if calm.is_empty() {
            return out;
        }
        let pids: Vec<Pid> = calm
            .iter()
            .flat_map(|(_, tasks)| tasks.iter().flatten().copied())
            .collect();
        let Some(servers) = (self.listening)(&pids) else {
            let at: Vec<usize> = calm.iter().map(|(i, _)| *i).collect();
            return self.keep_quiet(out, &at, seen);
        };
        for (i, tasks) in calm {
            // A job without a listening process is a watcher the session waits for (CI run, worker); only resting
            // servers mean done (Kevin's rule 1e: a preview for him or forgotten).
            if !tasks.iter().any(|t| t.is_disjoint(&servers)) {
                out[i].ready = true;
                continue;
            }
            // Kevin 2026-10-01: if the answer needs Kevin or his own message started the turn, grey hides it: done as
            // before 1e+ (fresh turn ends 2026-09-28 to 10-01: 23 of 23).
            if self.answer_for_user(&out[i], seen) {
                out[i].ready = true;
                self.for_user.insert(out[i].id.clone());
            } else {
                out[i].watching = true;
            }
        }
        out
    }

    /// Measurement failed: who rested in the last tick keeps watching or done; otherwise "works again" would stand in
    /// the log and the label would flicker for a tick (review quick 2026-09-27, Kimi). If the answer needs Kevin by now,
    /// watching becomes done (review quick 2026-10-01, Codex).
    fn keep_quiet(
        &mut self,
        mut sessions: Vec<Session>,
        at: &[usize],
        seen: &BTreeMap<String, Time>,
    ) -> Vec<Session> {
        for &i in at {
            let Some(&(_, watching)) = self.quiet.get(&sessions[i].id) else {
                continue;
            };
            if watching && !self.answer_for_user(&sessions[i], seen) {
                sessions[i].watching = true;
            } else {
                sessions[i].ready = true;
                if watching {
                    self.for_user.insert(sessions[i].id.clone());
                }
            }
        }
        sessions
    }

    /// Does the answer need Kevin (question, request, done message), or did his own message start the turn? Without a
    /// readable answer text, when in doubt yes. On Kevin's message a promise to report back is enough (Kevin 2026-10-03);
    /// once he had the window in front after the answer, his message no longer counts (Kevin 2026-10-04).
    fn answer_for_user(&self, s: &Session, seen: &BTreeMap<String, Time>) -> bool {
        // The Lookout line goes before the word list (Kevin 2026-10-12).
        if let Some(line) = s
            .digest
            .as_ref()
            .and_then(|d| d.line(&self.other_window_names))
        {
            return line.status != LineStatus::Nothing;
        }
        let Some((d, text)) = s
            .digest
            .as_ref()
            .and_then(|d| Some((d, d.text.as_deref()?)))
        else {
            return true;
        };
        if d.user_turn == Some(true) && !answer_seen(d, seen.get(&s.id).copied()) {
            return !(promises_report(text) && asks_nothing(text, false));
        }
        // Kevin 2026-10-09: a lead session reports on OTHER windows ("two windows wait for you"); the word list took that
        // for requests (6 days of a team lead: 211 of 523 turn ends DONE). For it only a question counts. Name from the
        // status file or the one in the menu (own name in the transcript, `/rename`; review quick 2026-10-09).
        if self.leads.contains(&s.name) || d.name.as_ref().is_some_and(|n| self.leads.contains(n)) {
            return text.contains('?');
        }
        !(asks_nothing(text, false) || opens_with_nothing_needed(text))
    }
}
