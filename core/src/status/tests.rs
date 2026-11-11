//! Status tests, ported assert for assert from LayoutTests.swift (functions of Watcher.swift). Swift's test runner set
//! `greetingName = "Kevin"` and German; both are the per-thread test defaults here (`overlay::greeting_name`,
//! `i18n`). macOS sampling (process start, `kill`, start arguments) comes in as closures with fixed answers.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Timelike};
use serde_json::{Value, json};

use super::*;
use crate::i18n::set_german;
use crate::model::{
    Digest, Engine, EngineProc, LastBlock, LineStatus, LookoutLine, Pid, PsRow, Session, Time,
};
use crate::overlay::set_greeting_name;

// ---------- Helpers ----------

/// A fresh folder name under the system temp folder (not created), removed with everything in it when dropped.
struct TempDir(String);

impl TempDir {
    fn new() -> TempDir {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir =
            std::env::temp_dir().join(format!("lookout-test-{}-{n}-{nanos}", std::process::id()));
        TempDir(dir.to_string_lossy().into_owned())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn append(path: &str, text: &str) {
    let mut f = fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

fn write_file(path: &str, text: &str) {
    fs::create_dir_all(Path::new(path).parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn system_time(t: Time) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs_f64(t)
}

type Lines = Arc<Mutex<Vec<String>>>;

/// A `report` that keeps its lines.
fn recorder() -> (Lines, Report) {
    let lines: Lines = Arc::default();
    let sink = lines.clone();
    (
        lines,
        Box::new(move |l: &str| sink.lock().unwrap().push(l.to_owned())),
    )
}

fn said(lines: &Lines) -> Vec<String> {
    lines.lock().unwrap().clone()
}

fn silent() -> Report {
    Box::new(|_| {})
}

fn lines(l: &[&str]) -> String {
    l.join("\n")
}

fn obj(v: Value) -> Obj {
    match v {
        Value::Object(o) => o,
        _ => panic!("test input must be an object"),
    }
}

fn me() -> Pid {
    Pid::try_from(std::process::id()).unwrap()
}

fn claude(id: &str, pid: Option<Pid>, name: &str, ready: bool) -> Session {
    Session::new(id, Engine::Claude, pid, name, "", ready, true)
}

fn find<'a>(s: &'a [Session], id: &str) -> Option<&'a Session> {
    s.iter().find(|x| x.id == id)
}

// ---------- Task 3: claude agents --json ----------

const AGENTS_FIXTURE: &str = r#"[
  {"id":"6a9d0a2e","cwd":"/Users/test","kind":"background","startedAt":1782917161034,
   "sessionId":"6a9d0a2e-7f03-46d9-9fa1-9d2f471d2c1a","name":"Review drift alert","state":"blocked"},
  {"pid":99492,"cwd":"/Users/test/Projects/Alpha/Tools/Viewer","kind":"interactive",
   "startedAt":1789671613103,"sessionId":"1d53b4a7-34b1-4cf6-92bf-5425b62cedb5",
   "name":"warpgrid-0c","status":"busy"},
  {"pid":45880,"cwd":"/Users/test/Projects/Alpha","kind":"interactive","startedAt":1789720001713,
   "sessionId":"74d07d7b-5d79-4774-a049-e48fcd031fb2","name":"alpha-6b","status":"idle"},
  {"kind":"interactive","name":"ohne-session-id","status":"idle"}
]"#;

#[test]
fn test_parse_agents_json() {
    let s = parse_agents_json(AGENTS_FIXTURE.as_bytes());
    assert_eq!(
        s.len(),
        3,
        "an entry without sessionId is dropped, not guessed"
    );

    let background = &s[0];
    assert_eq!(
        background.id, "claude:6a9d0a2e-7f03-46d9-9fa1-9d2f471d2c1a",
        "id = engine:sessionId"
    );
    assert_eq!(background.pid, None, "background sessions have no pid");
    assert!(background.ready, "state=blocked waits for Kevin -> ready");
    assert!(background.alive);
    assert_eq!(
        background.started_at,
        Some(1_782_917_161.034),
        "start from startedAt (ms)"
    );

    let busy = &s[1];
    assert_eq!(busy.pid, Some(99492));
    assert!(!busy.ready, "status=busy is not ready");
    assert_eq!(busy.name, "warpgrid-0c");
    assert_eq!(busy.engine, Engine::Claude);

    let idle = &s[2];
    assert_eq!(idle.pid, Some(45880));
    assert!(idle.ready, "status=idle is ready");

    // Kevin 2026-10-09: `shell` = turn over, only background commands run; `busy` = the model works.
    let shell = parse_agent(&obj(
        json!({"sessionId": "s", "pid": 7, "kind": "interactive", "status": "shell"}),
    ));
    assert!(
        shell
            .as_ref()
            .is_some_and(|s| s.background_only && !s.ready),
        "shell: not done, background only"
    );
    assert!(
        !busy.background_only && !idle.background_only && !background.background_only,
        "otherwise never background only"
    );
}

#[test]
fn test_parse_agents_json_robust() {
    assert!(
        parse_agents_json(b"kein json").is_empty(),
        "garbage -> empty list, no crash"
    );
    assert!(
        parse_agents_json(b"{}").is_empty(),
        "object instead of list -> empty"
    );
    assert!(parse_agents_json(b"[]").is_empty());
}

// ---------- Status-Dateien statt Dauer-Abfrage (2026-09-27, crash diagnosis) ----------

/// Writes a status file as Claude creates it (`<dir>/sessions/<pid>.json`).
fn write_session_file(dir: &str, pid: i64, sid: &str, status: &str, started_at: Time) {
    let json = format!(
        r#"{{"pid":{pid},"sessionId":"{sid}","cwd":"/tmp","startedAt":{},"kind":"interactive","entrypoint":"cli","name":"{sid}","status":"{status}"}}"#,
        (started_at * 1000.0) as i64
    );
    write_file(&format!("{dir}/sessions/{pid}.json"), &json);
}

/// Process starts as `sysctl` would answer: this test process started 100 s ago, launchd (pid 1) a day ago, other pids
/// do not exist.
fn fake_start(now: Time) -> impl Fn(Pid) -> Option<Time> {
    let me = me();
    move |pid| match pid {
        p if p == me => Some(now - 100.0),
        1 => Some(now - 86_400.0),
        _ => None,
    }
}

#[test]
fn test_read_session_files() {
    let root = TempDir::new();
    let now = super::now();
    let start = fake_start(now);
    assert!(
        read_session_files(&root.0, &start).is_none(),
        "folder missing -> None (old query)"
    );

    let me = i64::from(me());
    write_session_file(&root.0, me, "wartet", "waiting", now);
    write_session_file(&root.0, 99_999_990, "fertig", "idle", now); // no such process: stays
    write_session_file(&root.0, 99_999_991, "laeuft", "shell", now);
    write_file(&format!("{}/sessions/.syncthing.1.json.tmp", root.0), "{}");
    write_file(&format!("{}/sessions/99999992.json", root.0), ""); // half written
    let s = read_session_files(&root.0, &start).unwrap_or_default();
    assert_eq!(
        s.len(),
        3,
        "three valid files, .tmp and the empty file do not count"
    );
    let by_name: BTreeMap<&str, &Session> = s.iter().map(|x| (x.name.as_str(), x)).collect();
    assert!(
        by_name["wartet"].waiting
            && by_name["wartet"].ready
            && by_name["wartet"].pid.map(i64::from) == Some(me)
    );
    assert!(by_name["fertig"].ready && !by_name["fertig"].waiting);
    assert!(
        !by_name["laeuft"].ready,
        "shell = background command runs = works (like busy)"
    );
    assert_eq!(
        by_name["wartet"].id, "claude:wartet",
        "the same id as from claude agents"
    );

    // Left over: same pid, but the entry is older than the process (crash, pid given anew).
    write_session_file(&root.0, me, "alt", "idle", 1_000.0);
    assert!(
        !read_session_files(&root.0, &start)
            .unwrap_or_default()
            .iter()
            .any(|x| x.name == "alt"),
        "old entry on a new pid drops out"
    );
    // Also when the pid belongs to a root process now (launchd): libproc gave no start time for it (2026-09-28).
    write_session_file(&root.0, 1, "root", "idle", 1_000.0);
    assert!(
        !read_session_files(&root.0, &start)
            .unwrap_or_default()
            .iter()
            .any(|x| x.name == "root"),
        "old entry on a root pid drops out"
    );

    let broken = TempDir::new();
    fs::create_dir_all(format!("{}/sessions", broken.0)).unwrap();
    assert!(
        read_session_files(&broken.0, &start).is_some_and(|s| s.is_empty()),
        "empty folder = no session, NO fallback"
    );
    write_file(
        &format!("{}/sessions/1.json", broken.0),
        r#"{"sessionId":"x"}"#,
    );
    assert!(
        read_session_files(&broken.0, &start).is_none(),
        "files there, none readable (format changed?) -> None"
    );
}

/// `ClaudeStatus` with a scripted `claude agents`: this process lives (`kill`), nothing else; starts from `fake_start`.
fn claude_status(dirs: Vec<String>, agents: AgentsQuery, now: Time) -> ClaudeStatus {
    let me = me();
    ClaudeStatus::new(
        dirs,
        agents,
        Box::new(move |p| p == me),
        Box::new(fake_start(now)),
        silent(),
    )
}

/// Kevin 2026-09-27: status from the files, `claude agents` only every 5 minutes, then also the background agents.
#[test]
fn test_claude_status_asks_rarely() {
    let root = TempDir::new();
    let (files, no_files) = (format!("{}/main", root.0), format!("{}/work", root.0));
    fs::create_dir_all(&no_files).unwrap();
    let me = me();
    let wall = super::now();
    write_session_file(&files, i64::from(me), "a", "busy", wall);
    let agent = Session {
        waiting: true,
        ..claude("claude:bg", None, "bg", true)
    };
    let calls: Arc<Mutex<Vec<String>>> = Arc::default();
    let official: Arc<Mutex<BTreeMap<String, Vec<Session>>>> =
        Arc::new(Mutex::new(BTreeMap::from([
            (
                files.clone(),
                vec![agent.clone(), claude("claude:a", Some(me), "a", false)],
            ),
            (
                no_files.clone(),
                vec![claude("claude:u", Some(me), "u", true)],
            ),
        ])));
    let (c, o) = (calls.clone(), official.clone());
    let mut status = claude_status(
        vec![files.clone(), no_files.clone()],
        Box::new(move |dir| {
            c.lock().unwrap().push(dir.to_owned());
            o.lock().unwrap().get(dir).cloned()
        }),
        wall,
    );
    let take = |calls: &Arc<Mutex<Vec<String>>>| std::mem::take(&mut *calls.lock().unwrap());
    let t0 = 1_000_000.0;
    let every = ClaudeStatus::OFFICIAL_EVERY;

    let s = status.sessions(t0);
    assert_eq!(
        take(&calls),
        [files.as_str(), no_files.as_str()],
        "first tick: official query per folder"
    );
    let ids: BTreeSet<&str> = s.iter().map(|x| x.id.as_str()).collect();
    assert_eq!(
        ids,
        BTreeSet::from(["claude:a", "claude:bg", "claude:u"]),
        "files + background + fallback folder"
    );

    write_session_file(&files, i64::from(me), "a", "idle", wall);
    // The query reads the same file: from now on it says "done" too.
    official.lock().unwrap().get_mut(&files).unwrap()[1] = claude("claude:a", Some(me), "a", true);
    let s = status.sessions(t0 + 6.0);
    assert_eq!(
        take(&calls),
        [no_files.as_str()],
        "session tick: only the folder without files still asks"
    );
    assert!(
        find(&s, "claude:a").is_some_and(|x| x.ready),
        "the state change comes from the file"
    );
    assert!(
        find(&s, "claude:bg").is_some(),
        "background agent stays from the last query"
    );

    status.sessions(t0 + every - 1.0);
    assert_eq!(
        take(&calls),
        [no_files.as_str()],
        "before 5 minutes no official query for the folder with files"
    );
    status.sessions(t0 + every);
    assert_eq!(
        take(&calls),
        [files.as_str(), no_files.as_str()],
        "after 5 minutes official again"
    );

    // Cross-check: the query knows a running session missing in the files -> the old query per tick there.
    official
        .lock()
        .unwrap()
        .get_mut(&files)
        .unwrap()
        .push(claude("claude:neu", Some(me), "neu", false));
    let s = status.sessions(t0 + 2.0 * every);
    assert!(
        find(&s, "claude:neu").is_some(),
        "the missing session comes from the query"
    );
    take(&calls);
    let s = status.sessions(t0 + 2.0 * every + 6.0);
    assert!(
        take(&calls).contains(&files) && find(&s, "claude:neu").is_some(),
        "ask every tick until the next cross-check"
    );
    // Files and query agree again: the file holds again.
    write_session_file(&files, 99_999_993, "neu", "busy", wall);
    {
        let mut o = official.lock().unwrap();
        let list = o.get_mut(&files).unwrap();
        list.pop();
        list.push(claude("claude:neu", Some(99_999_993), "neu", false));
    }
    status.sessions(t0 + 3.0 * every);
    take(&calls);
    status.sessions(t0 + 3.0 * every + 6.0);
    assert_eq!(
        take(&calls),
        [no_files.as_str()],
        "cross-check fits again -> no query for this folder any more"
    );
}

/// review quick 2026-09-27 (GLM): a renamed status field would leave every session "working": a state that differs in
/// two cross-checks in a row means the old query; a single difference (change between the reads) does not. And: a
/// failed query drops the background agents instead of keeping them forever.
#[test]
fn test_claude_status_distrusts_wrong_states() {
    let dir = TempDir::new();
    let me = me();
    let wall = super::now();
    write_session_file(&dir.0, i64::from(me), "a", "busy", wall);
    let agent = Session {
        waiting: true,
        ..claude("claude:bg", None, "bg", true)
    };
    let answer: Arc<Mutex<Option<Vec<Session>>>> = Arc::new(Mutex::new(Some(vec![
        agent,
        claude("claude:a", Some(me), "a", true),
    ])));
    let calls = Arc::new(AtomicUsize::new(0));
    let (a, c) = (answer.clone(), calls.clone());
    let mut status = claude_status(
        vec![dir.0.clone()],
        Box::new(move |_| {
            c.fetch_add(1, Ordering::Relaxed);
            a.lock().unwrap().clone()
        }),
        wall,
    );
    let t0 = 2_000_000.0;
    let at = |minutes: f64| t0 + minutes * 60.0;
    let reset = || calls.swap(0, Ordering::Relaxed);

    status.sessions(at(0.0)); // first cross-check: file "busy", query "idle"
    reset();
    let s = status.sessions(at(0.1));
    assert!(
        reset() == 0 && find(&s, "claude:a").is_some_and(|x| !x.ready),
        "one difference: still from the file"
    );
    status.sessions(at(5.0)); // second cross-check: differs again
    reset();
    let s = status.sessions(at(5.1));
    assert_eq!(reset(), 1, "differs twice in a row -> old query per tick");
    assert!(
        find(&s, "claude:a").is_some_and(|x| x.ready),
        "the state comes from the query now"
    );

    write_session_file(&dir.0, i64::from(me), "a", "idle", wall); // fits again
    status.sessions(at(10.0));
    reset();
    let s = status.sessions(at(10.1));
    assert!(
        reset() == 0 && find(&s, "claude:a").is_some_and(|x| x.ready),
        "fits again -> back to the files"
    );
    assert!(
        find(&s, "claude:bg").is_some(),
        "background agent from the last query"
    );

    *answer.lock().unwrap() = None; // query fails (binary gone, time limit)
    let s = status.sessions(at(15.0));
    assert!(
        find(&s, "claude:bg").is_none(),
        "failed query: no stale background agents"
    );
    assert!(find(&s, "claude:a").is_some(), "the files carry on");
}

/// review quick 2026-09-27 (Kimi): a background agent WITH pid from `claude agents` must neither count as a session
/// missing in the files (constant fallback) nor drop out of "without window".
#[test]
fn test_background_agent_with_pid() {
    let me = me();
    let json = format!(
        r#"[{{"kind":"background","sessionId":"b2","pid":{me},"state":"blocked"}},{{"kind":"interactive","sessionId":"a","pid":{me},"status":"idle"}}]"#
    );
    let parsed = parse_agents_json(json.as_bytes());
    assert_eq!(
        find(&parsed, "claude:b2").map(|s| s.pid),
        Some(None),
        "background agent: its pid does not count"
    );
    assert_eq!(
        find(&parsed, "claude:a").map(|s| s.pid),
        Some(Some(me)),
        "interactive session keeps its pid"
    );

    let dir = TempDir::new();
    let wall = super::now();
    write_session_file(&dir.0, i64::from(me), "a", "idle", wall);
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let mut status = claude_status(
        vec![dir.0.clone()],
        Box::new(move |_| {
            c.fetch_add(1, Ordering::Relaxed);
            Some(parse_agents_json(json.as_bytes()))
        }),
        wall,
    );
    let t0 = 3_000_000.0;
    status.sessions(t0);
    calls.store(0, Ordering::Relaxed);
    let s = status.sessions(t0 + 6.0);
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "no fallback because of the background agent"
    );
    assert!(
        s.iter().any(|x| x.id == "claude:b2" && x.waiting),
        "blocked background agent stands under without window"
    );
}

/// Kevin 2026-09-28: the 5-minute cross-check also checks the reverse: a living file session that `claude agents` does
/// not know means not trusting the files until the next cross-check fits. Sessions started shortly before the query do
/// not count (they may have begun after it), ended ones neither.
#[test]
fn test_claude_status_checks_both_ways() {
    let dir = TempDir::new();
    let me = me();
    let started = 1_790_000_000.0; // the process start `fake_start`-style: passes the reused-pid check
    write_session_file(&dir.0, i64::from(me), "a", "idle", started);
    write_session_file(&dir.0, 99_999_994, "tot", "idle", 1_000.0);
    let answer: Arc<Mutex<Vec<Session>>> = Arc::default(); // the query knows neither
    let calls = Arc::new(AtomicUsize::new(0));
    let (a, c) = (answer.clone(), calls.clone());
    let mut status = ClaudeStatus::new(
        vec![dir.0.clone()],
        Box::new(move |_| {
            c.fetch_add(1, Ordering::Relaxed);
            Some(a.lock().unwrap().clone())
        }),
        Box::new(move |p| p == me),
        Box::new(move |p| (p == me).then_some(started)),
        silent(),
    );
    let at = |seconds: f64| started + seconds;
    let every = ClaudeStatus::OFFICIAL_EVERY;
    let reset = || calls.swap(0, Ordering::Relaxed);

    status.sessions(at(30.0)); // 30 s after the start: "a" too young, "tot" does not run
    reset();
    let s = status.sessions(at(36.0));
    assert!(
        reset() == 0 && find(&s, "claude:a").is_some(),
        "young session: still from the files"
    );

    status.sessions(at(30.0 + every)); // next cross-check: "a" old enough
    reset();
    let s = status.sessions(at(36.0 + every));
    assert_eq!(
        reset(),
        1,
        "living file session unknown to claude agents -> old query per tick"
    );
    assert!(find(&s, "claude:a").is_none(), "the query holds");

    *answer.lock().unwrap() = vec![claude("claude:a", Some(me), "a", true)];
    status.sessions(at(30.0 + 2.0 * every));
    reset();
    let s = status.sessions(at(36.0 + 2.0 * every));
    assert!(
        reset() == 0 && find(&s, "claude:a").is_some(),
        "fits again -> back to the files"
    );
}

/// An unusable answer of `claude agents` counts as failed (None), not as an empty list: otherwise the reverse check
/// would take every file session for unknown and Lookout would show no Claude session at all.
#[test]
fn test_agents_reply() {
    assert!(agents_reply(b"kein json").is_none(), "garbage -> None");
    assert!(
        agents_reply(b"{}").is_none(),
        "object instead of list -> None"
    );
    assert!(
        agents_reply(b"[]").is_some_and(|s| s.is_empty()),
        "empty list stays an empty list"
    );
    assert!(
        agents_reply(br#"[{"kind":"interactive","status":"idle"}]"#).is_none(),
        "entries there, none readable (format changed?) -> None"
    );
    assert_eq!(
        agents_reply(AGENTS_FIXTURE.as_bytes()).map(|s| s.len()),
        Some(3),
        "an unreadable entry among readable ones only drops out"
    );
}

// ---------- Task 4: Zug-Ende aus Session-Logs ----------

/// Real event vocabularies, collected from Kevin's sessions 2026-09-18.
const CODEX_WORKING: &str = r#"{"type":"event_msg","payload":{"type":"task_complete","completed_at":1}}
{"type":"event_msg","payload":{"type":"task_started"}}
{"type":"response_item","payload":{"type":"reasoning"}}"#;
const CODEX_DONE: &str = r#"{"type":"event_msg","payload":{"type":"task_started"}}
{"type":"response_item","payload":{"type":"message"}}
{"type":"event_msg","payload":{"type":"task_complete","completed_at":1789722036}}
{"type":"token_usage_record"}"#;
const GROK_DONE: &str = r#"{"ts":"2026-09-18T09:00:00Z","type":"turn_started","turn_number":0}
{"ts":"2026-09-18T09:00:48Z","type":"phase_changed","phase":"streaming_text"}
{"ts":"2026-09-18T09:00:48Z","type":"turn_ended"}"#;
const KIMI_WORKING: &str = r#"{"type":"agent.turn.ended","agentId":"main"}
{"type":"agent.turn.started","agentId":"main"}
{"type":"llm.request"}"#;

/// Esc in Codex writes `turn_aborted` (reason "interrupted", Kevin's logs 2026-11-10): stopped until the next turn starts.
#[test]
fn test_turn_aborted_is_stopped() {
    let aborted = r#"{"type":"event_msg","payload":{"type":"task_started"}}
{"type":"event_msg","payload":{"type":"turn_aborted","turn_id":"t1","reason":"interrupted"}}"#;
    assert_eq!(last_turn_state(aborted, Engine::Codex), TurnState::Aborted);
    let again = format!(
        "{aborted}\n{}",
        r#"{"type":"event_msg","payload":{"type":"task_started"}}"#
    );
    assert_eq!(last_turn_state(&again, Engine::Codex), TurnState::Working);
    let done_after = format!(
        "{again}\n{}",
        r#"{"type":"event_msg","payload":{"type":"task_complete","completed_at":2}}"#
    );
    assert_eq!(
        last_turn_state(&done_after, Engine::Codex),
        TurnState::Done,
        "an older abort does not hide a later finished turn (review quick 2026-11-10, Muse)"
    );
    let retried = r#"{"type":"agent.turn.started","agentId":"main"}
{"type":"llm.request.aborted"}
{"type":"llm.request"}"#;
    assert_eq!(
        last_turn_state(retried, Engine::Kimi),
        TurnState::Working,
        "a request broken off and retried inside the turn is no stop (review quick 2026-11-10, Codex)"
    );
}

/// Esc in Claude Code: "[Request interrupted by user]" (also "… for tool use]" beside a tool result) after the answer
/// marks the turn broken off; the next message or answer clears it.
#[test]
fn test_claude_interrupt_in_the_transcript() {
    let prompt = r#"{"type":"user","message":{"role":"user","content":"Bau das"}}"#;
    let answer = r#"{"type":"assistant","message":{"model":"claude-opus-5-5","content":[{"type":"text","text":"Ich baue."}]}}"#;
    let esc = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#;
    let esc_tool = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"x","content":"no"},{"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#;
    let d = digest_transcript(&[prompt, answer, esc].join("\n"));
    assert!(d.interrupted);
    assert_eq!(d.model.as_deref(), Some("claude-opus-5-5"));
    assert!(digest_transcript(&[prompt, answer, esc_tool].join("\n")).interrupted);
    assert!(!digest_transcript(&[prompt, answer, esc, prompt].join("\n")).interrupted);
    assert!(!digest_transcript(&[prompt, answer].join("\n")).interrupted);
}

#[test]
fn test_turn_state_from_tail() {
    assert_eq!(
        last_turn_state(CODEX_DONE, Engine::Codex),
        TurnState::Done,
        "the last turn event is task_complete"
    );
    assert_eq!(
        last_turn_state(CODEX_WORKING, Engine::Codex),
        TurnState::Working,
        "task_started AFTER task_complete means: works again"
    );
    assert_eq!(last_turn_state(GROK_DONE, Engine::Grok), TurnState::Done);
    assert_eq!(
        last_turn_state(KIMI_WORKING, Engine::Kimi),
        TurnState::Working
    );
    assert_eq!(
        last_turn_state("", Engine::Codex),
        TurnState::Unknown,
        "empty -> unknown, not 'done'"
    );
    assert_eq!(
        last_turn_state("{kaputt\n{\"type\":\"turn_ended\"}", Engine::Grok),
        TurnState::Done,
        "a broken line must not tip the evaluation"
    );
}

#[test]
fn test_log_ready_classification() {
    let now = super::now();
    assert!(
        classify_log_ready(now - 120.0, now, 90, false),
        "quiet beyond the threshold and no process signal -> done"
    );
    assert!(
        !classify_log_ready(now - 30.0, now, 90, false),
        "no 90 s of quiet yet -> not done"
    );
    assert!(
        !classify_log_ready(now - 600.0, now, 90, true),
        "process computes -> not done, however long the file rests"
    );
}

// ---------- Plan 1a: fremde Terminals in Ruhe lassen (2026-09-26) ----------

/// Kevin 2026-09-18 on "Ohne Fenster (35)": "Nur Echtes". Shown is what runs without a window, plus fresh background
/// agents; ended log runs are gone already, old agents only count.
#[test]
fn test_windowless_sessions() {
    let now = 2_000_000.0;
    let s = |id: &str, pid: Option<Pid>, started: Option<Time>| Session {
        started_at: started.map(|t| now - t),
        ..claude(id, pid, id, true)
    };
    let live = [
        s("tab", Some(10), None),
        s("imFenster", Some(11), None),
        s("frisch", None, Some(3600.0)),
        s("alt", None, Some(30.0 * 86_400.0)),
        s("ohneZeit", None, None),
    ];
    let (shown, stale) = windowless_sessions(&live, &BTreeSet::from(["imFenster".to_owned()]), now);
    let ids: Vec<&str> = shown.iter().map(|x| x.id.as_str()).collect();
    assert_eq!(ids, ["tab", "frisch"]);
    assert_eq!(stale, 2, "old and timeless agents only count");
}

// ---------- Reihenfolge "wer wartet auf Kevin" (the Watcher part) ----------

#[test]
fn test_waiting_from_permission_events() {
    // Grok writes permission_requested/_resolved (measured live 2026-09-18).
    let offen = lines(&[
        r#"{"type":"turn_started"}"#,
        r#"{"type":"permission_requested","tool":"bash"}"#,
    ]);
    let beantwortet = lines(&[
        r#"{"type":"permission_requested","tool":"bash"}"#,
        r#"{"type":"permission_resolved","granted":true}"#,
    ]);
    assert_eq!(
        last_turn_state(&offen, Engine::Grok),
        TurnState::Waiting,
        "open question"
    );
    assert_eq!(
        last_turn_state(&beantwortet, Engine::Grok),
        TurnState::Working,
        "answered -> works on"
    );

    // Patterns instead of fixed names: Codex-like names match too.
    let codex = lines(&[
        r#"{"type":"event_msg","payload":{"type":"task_started"}}"#,
        r#"{"type":"event_msg","payload":{"type":"exec_approval_request"}}"#,
    ]);
    assert_eq!(last_turn_state(&codex, Engine::Codex), TurnState::Waiting);

    // A background agent with state=blocked is an open question too.
    let blocked =
        parse_agents_json(br#"[{"kind":"background","sessionId":"b1","state":"blocked"}]"#);
    assert!(blocked.first().is_some_and(|s| s.waiting && s.ready));
    let busy = parse_agents_json(br#"[{"kind":"interactive","sessionId":"i1","status":"busy"}]"#);
    assert!(busy.first().is_some_and(|s| !s.waiting));

    // Interactive session with an open question: its own status `waiting` (measured live 2026-09-18).
    let fragt = parse_agents_json(
        br#"[{"kind":"interactive","sessionId":"i2","pid":1,"status":"waiting"}]"#,
    );
    assert!(
        fragt.first().is_some_and(|s| s.waiting),
        "status=waiting is an open question"
    );
    assert!(
        fragt.first().is_some_and(|s| s.ready),
        "who waits does not work, so counts as ready"
    );
    let idle =
        parse_agents_json(br#"[{"kind":"interactive","sessionId":"i3","pid":1,"status":"idle"}]"#);
    assert!(idle.first().is_some_and(|s| s.ready && !s.waiting));
}

// ---------- Schilder + Grün-Fix (design 2026-09-21) ----------

/// Real entry forms from Claude transcripts (sample 2026-09-21).
fn transcript_fixture() -> String {
    lines(&[
        r#"e","aiTitle":"abgeschnittene erste Zeile"}"#,
        r#"{"type":"ai-title","aiTitle":"Altes Thema"}"#,
        r#"{"type":"user","timestamp":"2026-09-21T12:40:00.000Z","message":{"role":"user","content":"Bau die Karten"}}"#,
        r#"{"type":"assistant","timestamp":"2026-09-21T12:40:05.000Z","message":{"content":[{"type":"thinking","thinking":""}]}}"#,
        r#"{"type":"assistant","timestamp":"2026-09-21T12:40:06.000Z","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"npm test","description":"Tests der Karten laufen lassen"}}]}}"#,
        r#"{"type":"user","timestamp":"2026-09-21T12:40:30.000Z","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#,
        r#"{"type":"user","isMeta":true,"timestamp":"2026-09-21T12:40:31.000Z","message":{"content":"<system-reminder>"}}"#,
        r#"{"type":"ai-title","aiTitle":"Build queue cards"}"#,
    ])
}

/// 2026-09-21T12:40:00Z (Swift: `ISO8601DateFormatter().date(from:)`, measured).
const PROMPT_AT: Time = 1_789_994_400.0;

#[test]
fn test_digest_transcript() {
    let fixture = transcript_fixture();
    let d = digest_transcript(&fixture);
    assert_eq!(
        d.title.as_deref(),
        Some("Build queue cards"),
        "the last ai-title holds"
    );
    assert!(
        d.step.as_deref() == Some("Tests der Karten laufen lassen") && d.last == LastBlock::Tool,
        "last step = Bash description"
    );
    assert_eq!(
        d.prompt_at,
        Some(PROMPT_AT),
        "turn start = Kevin's prompt, not a tool result or isMeta"
    );
    assert_eq!(d.answer_at, Some(PROMPT_AT + 6.0));
    assert!(d.text.is_none() && d.question.is_none());

    let fertig = format!(
        "{fixture}\n{}",
        r#"{"type":"assistant","timestamp":"2026-09-21T12:41:00.000Z","message":{"content":[{"type":"text","text":"Kevin, die Karten sind gebaut."}]}}"#
    );
    let f = digest_transcript(&fertig);
    assert!(
        f.last == LastBlock::Text && f.text.as_deref() == Some("Kevin, die Karten sind gebaut.")
    );

    let frage = format!(
        "{fertig}\n{}",
        r#"{"type":"assistant","timestamp":"2026-09-21T12:42:00.000Z","message":{"content":[{"type":"tool_use","id":"toolu_frage","name":"AskUserQuestion","input":{"questions":[{"question":"Welche Variante?","header":"V","options":[]}]}}]}}"#
    );
    assert_eq!(
        digest_transcript(&frage).question.as_deref(),
        Some("Welche Variante?"),
        "open question recognised"
    );
    let beantwortet = format!(
        "{frage}\n{}",
        r#"{"type":"user","timestamp":"2026-09-21T12:43:00.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_frage","content":"A"}]}}"#
    );
    let b = digest_transcript(&beantwortet);
    assert!(
        b.question.is_none(),
        "the answer to THIS question closes it"
    );
    assert_eq!(b.prompt_at, d.prompt_at, "a tool result is no new prompt");

    // review quick 2026-09-21 (Kimi): the result of a tool started IN PARALLEL does not end the question (1 of 211).
    let parallel = format!(
        "{fertig}\n{}\n{}",
        r#"{"type":"assistant","timestamp":"2026-09-21T12:42:00.000Z","message":{"content":[{"type":"tool_use","id":"toolu_frage","name":"AskUserQuestion","input":{"questions":[{"question":"Welche Variante?"}]}},{"type":"tool_use","id":"toolu_ls","name":"Bash","input":{"command":"ls","description":"Ordner ansehen"}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_ls","content":"a b"}]}}"#
    );
    assert_eq!(
        digest_transcript(&parallel).question.as_deref(),
        Some("Welche Variante?"),
        "another tool's result leaves the question open"
    );
    let meta = format!(
        "{frage}\n{}",
        r#"{"type":"user","isMeta":true,"message":{"content":"<local-command-stdout>"}}"#
    );
    assert_eq!(
        digest_transcript(&meta).question.as_deref(),
        Some("Welche Variante?"),
        "a system insert leaves the question open"
    );
    let neuer_prompt = format!(
        "{frage}\n{}",
        r#"{"type":"user","message":{"content":"Mach weiter"}}"#
    );
    assert!(
        digest_transcript(&neuer_prompt).question.is_none(),
        "a new prompt closes the question"
    );

    // U+2028 inside the JSON text must not split the line (spike: Python's splitlines did).
    let trenner = "{\"type\":\"ai-title\",\"aiTitle\":\"Zeile\u{2028}zwei\"}";
    assert_eq!(
        digest_transcript(trenner).title.as_deref(),
        Some("Zeile\u{2028}zwei")
    );
    assert!(
        digest_transcript("").title.is_none(),
        "empty -> empty digest, no crash"
    );
}

#[test]
fn test_step_label() {
    let label = |tool: &str, input: Value| step_label(tool, &obj(input));
    assert_eq!(
        label(
            "Bash",
            json!({"command": "git status", "description": "Zeigt den Stand"})
        ),
        "Zeigt den Stand"
    );
    assert_eq!(
        label("Bash", json!({"command": "git status\n--short"})),
        "git status",
        "without description: first command line"
    );
    assert_eq!(
        label("Edit", json!({"file_path": "/a/b/Layout.swift"})),
        "ändert Layout.swift"
    );
    assert_eq!(
        label("Agent", json!({"description": "Review Shop"})),
        "Agent: Review Shop"
    );
    assert_eq!(
        label("mcp__mail__listEmails", json!({})),
        "mail: listEmails"
    );
    assert_eq!(
        label("WebFetch", json!({"url": "https://docs.warp.dev/x"})),
        "liest Webseite docs.warp.dev"
    );
    assert_eq!(
        label(
            "AskUserQuestion",
            json!({"questions": [{"question": "Welche?"}]})
        ),
        "Frage: Welche?"
    );
}

#[test]
fn test_transcript_reader() {
    let root = TempDir::new();
    let path = format!("{}/projects/-Users-test-Projects-Beta/sid-1.jsonl", root.0);
    write_file(
        &path,
        "{\"type\":\"ai-title\",\"aiTitle\":\"Erstes Thema\"}\n",
    );

    let mut reader = TranscriptReader::new(vec![root.0.clone()], TAIL_BYTES);
    assert_eq!(
        reader.digest("sid-1").and_then(|d| d.title),
        Some("Erstes Thema".into()),
        "found in any project folder"
    );
    assert!(
        reader.digest("fehlt").is_none(),
        "without transcript: None, no crash"
    );
    append(
        &path,
        "{\"type\":\"ai-title\",\"aiTitle\":\"Zweites Thema\"}\n",
    );
    assert_eq!(
        reader.digest("sid-1").and_then(|d| d.title),
        Some("Zweites Thema".into()),
        "a grown file is read again"
    );
    reader.forget(&BTreeSet::new());
    assert_eq!(
        reader.digest("sid-1").and_then(|d| d.title),
        Some("Zweites Thema".into()),
        "searched again after forgetting"
    );

    // review quick 2026-10-10 (GLM): the other session's message falls out of the tail after a long turn.
    let mut small = TranscriptReader::new(vec![root.0.clone()], 400);
    let peer = r#"{"type":"user","isMeta":true,"origin":{"kind":"peer","from":"uds:/tmp/cc-socks/55848.sock","verifiedPeerPid":55848,"name":"Lead"},"message":{"content":"go"}}"#;
    append(&path, &format!("{peer}\n"));
    assert_eq!(
        small.digest("sid-1").and_then(|d| d.peer_pid),
        Some(55848),
        "message in the tail"
    );
    append(&path, &"{\"type\":\"progress\"}\n".repeat(40));
    let later = small.digest("sid-1");
    assert!(
        later.is_some_and(|d| d.peer_pid == Some(55848) && d.peer_name.as_deref() == Some("Lead")),
        "fallen out of the tail: remembered"
    );

    // review quick 2026-11-10 (GLM): a break-off stays when its marker falls out of the tail without a message or
    // answer after it.
    let esc = r#"{"type":"user","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#;
    append(&path, &format!("{esc}\n"));
    assert!(small.digest("sid-1").is_some_and(|d| d.interrupted));
    append(&path, &"{\"type\":\"progress\"}\n".repeat(40));
    assert!(
        small.digest("sid-1").is_some_and(|d| d.interrupted),
        "fallen out of the tail: still stopped"
    );
    let answer = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Weiter."}]}}"#;
    append(&path, &format!("{answer}\n"));
    assert!(
        small.digest("sid-1").is_some_and(|d| !d.interrupted),
        "an answer ends it"
    );
}

// ---------- v4: saubere Namen, Fenster am Namen, verworfene Klicks (design 2026-09-21) ----------

/// The naming hook sets `custom-title` in the transcript (probe 2026-09-21); it wins over the topic.
#[test]
fn test_digest_custom_title() {
    let t = lines(&[
        r#"{"type":"ai-title","aiTitle":"Handoff spawn 2026-09-21 Gamma Umzug"}"#,
        r#"{"type":"custom-title","customTitle":"Gamma","sessionId":"s"}"#,
        r#"{"type":"custom-title","customTitle":"Gamma · Buchhaltung weiterführen","sessionId":"s"}"#,
    ]);
    let d = digest_transcript(&t);
    assert_eq!(
        d.name.as_deref(),
        Some("Gamma · Buchhaltung weiterführen"),
        "the last own name holds"
    );
    assert_eq!(
        d.title.as_deref(),
        Some("Handoff spawn 2026-09-21 Gamma Umzug"),
        "the topic stays beside it"
    );
}

/// Name and topic are written only with each message: after a long turn they lie up to ~5.5 MB before the end
/// (measured 2026-09-21), far before the tail (review quick v4).
#[test]
fn test_transcript_titles_far_back() {
    let root = TempDir::new();
    let path = format!("{}/projects/-Users-test-Projects-Alpha/sid-2.jsonl", root.0);
    let filler = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"weiter\"}]}}\n".repeat(60);
    write_file(
        &path,
        &format!(
            "{}\n{}\n{filler}",
            r#"{"type":"custom-title","customTitle":"Gamma · Buchhaltung weiterführen"}"#,
            r#"{"type":"ai-title","aiTitle":"Ledger books"}"#
        ),
    );

    let mut reader = TranscriptReader::new(vec![root.0.clone()], 1024);
    let names = |d: Option<Digest>| d.map(|d| (d.name, d.title));
    let full = (
        Some("Gamma · Buchhaltung weiterführen".to_owned()),
        Some("Ledger books".to_owned()),
    );
    assert_eq!(
        names(reader.digest("sid-2")),
        Some(full.clone()),
        "the first read searches backwards"
    );
    append(&path, &filler);
    assert_eq!(
        names(reader.digest("sid-2")),
        Some(full),
        "grown without a new name: the last one seen stays"
    );
    append(
        &path,
        "{\"type\":\"custom-title\",\"customTitle\":\"Neu\"}\n",
    );
    assert_eq!(
        reader.digest("sid-2").and_then(|d| d.name),
        Some("Neu".into()),
        "a new name wins"
    );
    // review quick 2026-10-01 (Codex): after a long turn Kevin's prompt is no longer in the tail; the trigger seen last
    // stays until a new one comes.
    append(
        &path,
        "{\"type\":\"user\",\"origin\":{\"kind\":\"human\"},\"turnPosition\":{\"promptIndex\":1,\"turnIndex\":1},\"message\":{\"content\":\"und jetzt CI?\"}}\n",
    );
    let turn = |r: &mut TranscriptReader| r.digest("sid-2").and_then(|d| d.user_turn);
    assert_eq!(turn(&mut reader), Some(true), "Kevin's prompt in the tail");
    // Out of the tail step by step (each read grows less than the tail): its text stays too.
    let wenig = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"weiter\"}]}}\n".repeat(8);
    for _ in 0..3 {
        append(&path, &wenig);
        reader.digest("sid-2");
    }
    assert_eq!(
        turn(&mut reader),
        Some(true),
        "prompt fallen out of the tail: stays Kevin's turn"
    );
    assert_eq!(
        reader.digest("sid-2").and_then(|d| d.user_prompt),
        Some("und jetzt CI?".into()),
        "its text stays too (start prompt check)"
    );
    // review quick 2026-10-01 (Codex): grew the file by more than the tail between two reads, Kevin's message mid-turn
    // may lie there: the remembered start prompt no longer holds, when in doubt Kevin's turn.
    append(&path, &filler);
    assert_eq!(turn(&mut reader), Some(true), "jump: stays Kevin's turn");
    assert_eq!(
        reader.digest("sid-2").and_then(|d| d.user_prompt),
        None,
        "jump over the tail: never a start prompt"
    );
    append(
        &path,
        "{\"type\":\"user\",\"isMeta\":true,\"origin\":{\"kind\":\"peer\",\"from\":\"uds:/tmp/x.sock\",\"verifiedPeerPid\":7,\"name\":\"Lead\"},\"message\":{\"content\":\"go\"}}\n",
    );
    assert_eq!(
        turn(&mut reader),
        Some(false),
        "the other session starts the next turn"
    );

    // A name line across a block border is not lost; the search goes only to the limit.
    for chunk in [16, 37, 100, 4096] {
        let t = last_titles(&path, chunk, 1 << 20);
        assert!(
            t.0.as_deref() == Some("Neu") && t.1.as_deref() == Some("Ledger books"),
            "block {chunk}: {t:?}"
        );
    }
    assert!(
        last_titles(&path, 100, 2048).1.is_none(),
        "only up to the limit"
    );
    let fehlt = last_titles(
        &format!("{}/fehlt.jsonl", root.0),
        LAST_TITLES_CHUNK,
        LAST_TITLES_MAX,
    );
    assert!(
        fehlt.0.is_none() && fehlt.1.is_none(),
        "missing file: nothing, no crash"
    );
}

// ---------- Durchsicht 2026-09-25 (Kevin's screenshot: green frame without reason) ----------

fn write_aged(path: &str, text: &str, age: f64, now: Time) {
    write_file(path, text);
    let f = fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(system_time(now - age)).unwrap();
}

/// Sets the creation date like `setAttributes([.creationDate: …])`; only macOS can.
#[cfg(target_os = "macos")]
fn set_created(path: &str, at: Time) {
    use std::os::macos::fs::FileTimesExt;
    let f = fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_times(fs::FileTimes::new().set_created(system_time(at)))
        .unwrap();
}

fn proc(pid: Pid, engine: Engine, tty: &str, cwd: &str) -> EngineProc {
    EngineProc {
        pid,
        engine,
        tty: tty.into(),
        cwd: cwd.into(),
        in_terminal: true,
    }
}

/// Codex/Grok/Kimi (GLM + Kimi 2026-09-25): a turn finished more than 6 h ago, or a Codex session from the day before
/// yesterday, fell out with its window although the process still ran. Now searched from the living process: per working
/// folder the newest log file, without an age limit; without a process not at all.
#[test]
fn test_log_sessions_follow_live_processes() {
    let home = TempDir::new();
    let h = &home.0;
    let now = super::now();
    let started = now - 3.0 * 86_400.0;
    let day = transcript::local(started).unwrap().format("%Y/%m/%d");
    // Codex: started three days ago, done for 10 h.
    write_aged(
        &format!("{h}/.codex/sessions/{day}/rollout-a.jsonl"),
        &format!(
            "{}\n{CODEX_DONE}",
            r#"{"type":"session_meta","payload":{"session_id":"cx-1","cwd":"/w/codex"}}"#
        ),
        10.0 * 3600.0,
        now,
    );
    // Grok: two sessions in one folder; the newer (done, 8 h ago) holds, not the older.
    write_aged(
        &format!("{h}/.grok/sessions/%2Fw%2Fgrok/g-alt/events.jsonl"),
        r#"{"type":"turn_started"}"#,
        30.0 * 3600.0,
        now,
    );
    write_aged(
        &format!("{h}/.grok/sessions/%2Fw%2Fgrok/g-neu/events.jsonl"),
        GROK_DONE,
        8.0 * 3600.0,
        now,
    );
    // Kimi: done for 7 h.
    write_aged(
        &format!("{h}/.kimi-code/sessions/wd_kimi_0123456789ab/session_k/state.json"),
        r#"{"id":"session_k","cwd":"/w/kimi"}"#,
        7.0 * 3600.0,
        now,
    );
    write_aged(
        &format!("{h}/.kimi-code/sessions/wd_kimi_0123456789ab/session_k/agents/main/wire.jsonl"),
        r#"{"type":"agent.turn.ended"}"#,
        7.0 * 3600.0,
        now,
    );
    let procs = [
        proc(101, Engine::Codex, "ttys001", "/w/codex"),
        proc(102, Engine::Grok, "ttys002", "/w/grok"),
        proc(103, Engine::Kimi, "ttys003", "/w/kimi"),
        proc(104, Engine::Codex, "ttys004", "/w/ohne-log"),
    ];
    let mut finder = LogSessionFinder::new(h.clone(), Box::new(move |_| Some(started)));
    let found: BTreeMap<String, Session> = finder
        .sessions(&procs, 90)
        .into_iter()
        .map(|s| (s.id.clone(), s))
        .collect();
    assert!(
        found.get("codex:cx-1").is_some_and(|s| s.ready),
        "Codex from the day before yesterday, done for 10 h, got {:?}",
        found.keys()
    );
    assert!(
        found.get("grok:g-neu").is_some_and(|s| s.ready) && !found.contains_key("grok:g-alt"),
        "per process the newest Grok session"
    );
    assert!(
        found.get("kimi:session_k").is_some_and(|s| s.ready),
        "Kimi done for 7 h"
    );
    assert_eq!(
        found.len(),
        3,
        "without a log no session, got {:?}",
        found.keys()
    );
    assert!(
        finder.sessions(&[], 90).is_empty(),
        "without a process nothing is read"
    );

    // Two Grok terminals in one folder (review quick 2026-09-25, GLM): each session belongs to the process started
    // before it, not the last written file to the first process of the list; otherwise both windows swapped their
    // state. Needs creation dates, which only macOS lets a test set.
    #[cfg(target_os = "macos")]
    {
        let alt = format!("{h}/.grok/sessions/%2Fw%2Fzwei/z-alt/events.jsonl");
        let neu = format!("{h}/.grok/sessions/%2Fw%2Fzwei/z-neu/events.jsonl");
        write_aged(&alt, r#"{"type":"turn_started"}"#, 60.0, now); // older process works right now (written last)
        write_aged(&neu, GROK_DONE, 1800.0, now); // younger process done for 30 min
        set_created(&alt, now - 5.0 * 3600.0);
        set_created(&neu, now - 3600.0);
        let starts: BTreeMap<Pid, Time> =
            BTreeMap::from([(301, now - 5.0 * 3600.0 - 60.0), (302, now - 3600.0 - 60.0)]);
        let zwei = LogSessionFinder::new(h.clone(), Box::new(move |p| starts.get(&p).copied()))
            .sessions(
                &[
                    proc(301, Engine::Grok, "ttys005", "/w/zwei"),
                    proc(302, Engine::Grok, "ttys006", "/w/zwei"),
                ],
                90,
            );
        let pids: BTreeMap<&str, Option<Pid>> =
            zwei.iter().map(|s| (s.id.as_str(), s.pid)).collect();
        assert!(
            pids.get("grok:z-alt") == Some(&Some(301))
                && pids.get("grok:z-neu") == Some(&Some(302)),
            "session per process by start time, got {pids:?}"
        );
    }
}

// ---------- Plan 1e: fertig trotz Hintergrund-Befehl, wenn der nur wartet (Kevin 2026-09-26 evening) ----------

/// `claude agents` reports `busy` while a background command runs, also after the finished answer. The turn end is in
/// the transcript: `system`/`turn_duration`, after the last answer and the stop hooks.
#[test]
fn test_turn_ended() {
    let antwort = r#"{"type":"assistant","timestamp":"2026-09-26T19:16:40.000Z","message":{"content":[{"type":"text","text":"Fertig."}]}}"#;
    let hook = r#"{"type":"system","subtype":"stop_hook_summary","hookCount":6}"#;
    let ende = r#"{"type":"system","subtype":"turn_duration","durationMs":102576,"timestamp":"2026-09-26T19:16:42.562Z","isMeta":false}"#;
    assert!(
        digest_transcript(&lines(&[antwort, hook, ende])).turn_ended,
        "turn_duration ends the turn"
    );
    assert!(
        !digest_transcript(&lines(&[antwort, hook])).turn_ended,
        "without turn_duration: turn open"
    );
    let prompt = r#"{"type":"user","message":{"content":"Mach weiter"}}"#;
    assert!(
        !digest_transcript(&lines(&[antwort, ende, prompt])).turn_ended,
        "a new prompt lifts it"
    );
    // Background notices come as a prompt (live: origin task-notification, without isMeta) and start a turn.
    let meldung = r#"{"type":"user","origin":{"kind":"task-notification"},"message":{"content":"<task-notification>fertig</task-notification>"}}"#;
    assert!(
        !digest_transcript(&lines(&[antwort, ende, meldung])).turn_ended,
        "background notice"
    );
    assert!(
        !digest_transcript(&lines(&[ende, antwort])).turn_ended,
        "a later answer lifts it"
    );
    let meta = r#"{"type":"user","isMeta":true,"message":{"content":"<system-reminder>"}}"#;
    assert!(
        digest_transcript(&lines(&[antwort, ende, meta])).turn_ended,
        "a system insert is no prompt"
    );
    // Kevin 2026-10-10 (23:27, two waiting windows DONE after /compact): the summary is no prompt.
    let peer = r#"{"type":"user","isMeta":true,"origin":{"kind":"peer","from":"uds:/tmp/cc-socks/55848.sock","verifiedPeerPid":55848,"name":"Lead"},"message":{"content":"go"}}"#;
    let grenze = r#"{"type":"system","subtype":"compact_boundary","isMeta":false}"#;
    let summary = r#"{"type":"user","isCompactSummary":true,"isVisibleInTranscriptOnly":true,"message":{"content":"This session is being continued from a previous conversation that ran out of context."}}"#;
    let d = digest_transcript(&lines(&[peer, antwort, ende, grenze, summary]));
    assert!(
        d.turn_ended && d.user_turn == Some(false) && d.peer_pid == Some(55848),
        "compaction changes nothing about the turn"
    );
}

fn row(pid: Pid, ppid: Pid, tty: &str, cpu: f64, comm: &str) -> PsRow {
    PsRow {
        pid,
        ppid,
        tty: tty.into(),
        cpu,
        comm: comm.into(),
    }
}

/// Live 2026-09-26: a session with `zsh → fnm → node` in the background (0 % CPU), beside it the MCP servers as direct
/// children and `caffeinate`. Swift's `backgroundFixture` as `parsePsLine` reads it (CPU "3:05.50" = 185.5 s).
fn background_fixture() -> Vec<PsRow> {
    vec![
        row(530, 520, "ttys001", 185.5, "/Users/test/.local/bin/claude"),
        row(540, 530, "??", 0.0, "/bin/zsh"),
        row(541, 540, "??", 0.01, "/opt/homebrew/bin/fnm"),
        row(542, 541, "??", 2.0, "node"),
        row(
            550,
            530,
            "ttys001",
            12.0,
            "/Library/Frameworks/Python.framework/Versions/3.13/Resources/Python.app/Contents/MacOS/Python",
        ),
        row(
            551,
            530,
            "ttys001",
            0.4,
            "npm exec @upstash/context7-mcp@3.2.0",
        ),
        row(552, 551, "ttys001", 5.0, "node"),
        row(560, 530, "ttys001", 0.0, "caffeinate"),
        row(570, 530, "??", 0.01, "-bash"),
        row(580, 999, "??", 0.01, "/bin/zsh"),
    ]
}

fn pids(p: &[Pid]) -> BTreeSet<Pid> {
    p.iter().copied().collect()
}

type Net = Arc<Mutex<Option<BTreeMap<Pid, u64>>>>;

/// A watch with scripted nettop (`net`, None = fails) and lsof (`servers`, None = fails).
fn watch_with(
    net: Net,
    servers: Arc<Mutex<Option<BTreeSet<Pid>>>>,
    leads: &[&str],
    report: Report,
) -> BackgroundWatch {
    BackgroundWatch::new(
        Box::new(move || net.lock().unwrap().clone()),
        Box::new(move |_| servers.lock().unwrap().clone()),
        leads.iter().map(|s| s.to_string()).collect(),
        vec![],
        report,
    )
}

fn fixed<T: Clone + Send + 'static>(v: T) -> Arc<Mutex<T>> {
    Arc::new(Mutex::new(v))
}

/// Kevin's rule: if the background command works, the session runs; an observer or something resting means done.
/// "Works" = in the last 90 s more than 1 s CPU time or more than 50 KB network (start values).
#[test]
fn test_background_activity() {
    let rows = background_fixture();
    assert_eq!(
        background_pids(530, &rows),
        pids(&[540, 541, 542, 570]),
        "shell children with everything below; MCP servers and caffeinate do not count"
    );
    assert!(background_pids(999, &rows) == pids(&[580]) && background_pids(1, &rows).is_empty());

    let t0 = 50_000.0;
    let at = |dt: f64, cpu: f64, net: u64| UsageSample {
        at: t0 + dt,
        cpu: BTreeMap::from([(542, cpu)]),
        net: BTreeMap::from([(542, net)]),
    };
    let t96 = t0 + 96.0;
    assert!(
        background_busy(&[at(0.0, 2.0, 0), at(60.0, 2.3, 0)], t0 + 60.0, 90.0),
        "watched only 60 s: still runs"
    );
    assert!(
        !background_busy(
            &[at(0.0, 2.0, 0), at(60.0, 2.3, 0), at(96.0, 2.5, 0)],
            t96,
            90.0
        ),
        "90 s with 0.5 s CPU and no network: done"
    );
    assert!(
        background_busy(&[at(0.0, 2.0, 0), at(96.0, 3.5, 0)], t96, 90.0),
        "1.5 s CPU: works"
    );
    assert!(
        background_busy(&[at(0.0, 2.0, 1_000), at(96.0, 2.0, 80_000)], t96, 90.0),
        "network grows (waits for a language model): works"
    );
    let new_process = UsageSample {
        at: t96,
        cpu: BTreeMap::from([(543, 4.0)]),
        net: BTreeMap::new(),
    };
    assert!(
        background_busy(&[at(0.0, 2.0, 0), new_process], t96, 90.0),
        "a new process counts with its whole CPU time"
    );
    let gone = UsageSample {
        at: t96,
        cpu: BTreeMap::new(),
        net: BTreeMap::new(),
    };
    assert!(
        !background_busy(&[at(0.0, 2.0, 90_000), gone], t96, 90.0),
        "ended processes and closed connections subtract nothing"
    );

    let net = parse_nettop(
        ",bytes_in,bytes_out,\nlaunchd.1,0,0,\n2.1.283.530,1107283,4290220,\nnode.542,6546,1797507,\nkaputt\n",
    );
    assert_eq!(
        net,
        BTreeMap::from([(1, 0), (530, 5_397_503), (542, 1_804_053)]),
        "the name may contain dots"
    );

    let tasks: BTreeSet<BTreeSet<Pid>> = background_tasks(530, &rows).into_iter().collect();
    assert_eq!(
        tasks,
        BTreeSet::from([pids(&[540, 541, 542]), pids(&[570])]),
        "per background shell one job with everything below"
    );

    // The series over several session ticks (network and listening ports from scripted sources instead of nettop/lsof).
    // Fixture: shell 540 leads a server (node 542 listens), shell 570 is a watcher (does not listen).
    let net_now: Net = fixed(Some(BTreeMap::new()));
    let server = fixed(Some(pids(&[542])));
    let (log, report) = recorder();
    let mut watch = watch_with(net_now.clone(), server.clone(), &[], report);
    let busy = Session {
        background_only: true,
        digest: Some(Digest {
            text: Some("Waiting for the workers. Nothing needed from you.".into()),
            last: LastBlock::Text,
            turn_ended: true,
            ..Digest::default()
        }),
        ..Session::new(
            "claude:b",
            Engine::Claude,
            Some(530),
            "lookout-b",
            "",
            false,
            true,
        )
    };
    let tick = |w: &mut BackgroundWatch, dt: f64, s: &Session, r: &[PsRow]| {
        w.adjust(std::slice::from_ref(s), r, t0 + dt, 90.0, &BTreeMap::new())[0].clone()
    };
    assert!(
        !tick(&mut watch, 0.0, &busy, &rows).ready && !tick(&mut watch, 50.0, &busy, &rows).ready,
        "before 90 s of quiet: runs"
    );
    // Kevin 2026-09-27 (lead session): who waits for its own watcher does not need Kevin; no done alarm.
    let wacht = tick(&mut watch, 96.0, &busy, &rows);
    assert!(
        wacht.watching && !wacht.ready,
        "watcher resting for 90 s: watching, not done"
    );
    assert_eq!(
        said(&log),
        ["Hintergrund ruht seit 90 s: lookout-b gilt als überwacht (Aufpasser läuft)"]
    );
    assert!(
        tick(&mut watch, 99.0, &busy, &rows).watching && said(&log).len() == 1,
        "same state: no new line"
    );
    let rechnet: Vec<PsRow> = rows
        .iter()
        .map(|r| {
            if r.pid == 542 {
                row(542, 541, "??", 9.0, "node")
            } else {
                r.clone()
            }
        })
        .collect();
    let wieder = tick(&mut watch, 102.0, &busy, &rechnet);
    assert!(
        !wieder.ready && !wieder.watching,
        "CPU time grows: runs again"
    );
    assert_eq!(
        said(&log).last().map(String::as_str),
        Some("Hintergrund arbeitet wieder: lookout-b")
    );
    // Only a resting server (a preview for Kevin or forgotten): done as before (plan 1e).
    let nur_server: Vec<PsRow> = rows.iter().filter(|r| r.pid != 570).cloned().collect();
    let (server_log, report) = recorder();
    let mut server_watch = watch_with(fixed(Some(BTreeMap::new())), server.clone(), &[], report);
    for dt in [0.0, 96.0] {
        tick(&mut server_watch, dt, &busy, &nur_server);
    }
    let fertig = tick(&mut server_watch, 99.0, &busy, &nur_server);
    assert!(
        fertig.ready && !fertig.watching,
        "only a resting server: done"
    );
    assert_eq!(
        said(&server_log).last().map(String::as_str),
        Some("Hintergrund ruht seit 90 s: lookout-b gilt als fertig (nur ein Server läuft)")
    );
    // If the port query fails, Lookout does not guess: as reported (runs).
    let mut blind = watch_with(fixed(Some(BTreeMap::new())), fixed(None), &[], silent());
    for dt in [0.0, 96.0] {
        tick(&mut blind, dt, &busy, &rows);
    }
    let unklar = tick(&mut blind, 99.0, &busy, &rows);
    assert!(!unklar.ready && !unklar.watching, "lsof fails: as reported");
    // review quick 2026-09-27 (Kimi): if a measurement fails for a session that rested already, it keeps its state;
    // otherwise "works again" in the log and a WORKING tick although the background rests unchanged. For lsof and (same
    // cause, since plan 1e) nettop.
    let (net_ok, lsof_ok) = (fixed(Some(BTreeMap::new())), fixed(Some(pids(&[542]))));
    let (wackel_log, report) = recorder();
    let mut wackel = watch_with(net_ok.clone(), lsof_ok.clone(), &[], report);
    for dt in [0.0, 96.0] {
        tick(&mut wackel, dt, &busy, &rows);
    }
    *lsof_ok.lock().unwrap() = None;
    let haelt = tick(&mut wackel, 102.0, &busy, &rows);
    assert!(
        haelt.watching && !haelt.ready && said(&wackel_log).len() == 1,
        "lsof fails: stays watching, got {:?}",
        said(&wackel_log)
    );
    *lsof_ok.lock().unwrap() = Some(pids(&[542]));
    *net_ok.lock().unwrap() = None;
    let haelt_netz = tick(&mut wackel, 108.0, &busy, &rows);
    assert!(
        haelt_netz.watching && said(&wackel_log).len() == 1,
        "nettop fails: stays watching"
    );
    let net_ok2 = fixed(Some(BTreeMap::new()));
    let mut nur_server_wackel = watch_with(net_ok2.clone(), server.clone(), &[], silent());
    for dt in [0.0, 96.0] {
        tick(&mut nur_server_wackel, dt, &busy, &nur_server);
    }
    *net_ok2.lock().unwrap() = None;
    let haelt_fertig = tick(&mut nur_server_wackel, 102.0, &busy, &nur_server);
    assert!(
        haelt_fertig.ready && !haelt_fertig.watching,
        "nettop fails: done stays done"
    );
    // review quick 2026-09-26 (GLM + Kimi): when the background command (claude agents reports idle) or the session ends,
    // nothing works again; no wrong log line.
    let (quiet_log, report) = recorder();
    let mut quiet_watch = watch_with(
        fixed(Some(BTreeMap::new())),
        fixed(Some(BTreeSet::new())),
        &[],
        report,
    );
    for dt in [0.0, 96.0] {
        tick(&mut quiet_watch, dt, &busy, &rows);
    }
    let before = said(&quiet_log).len();
    let idle = Session::new(
        busy.id.clone(),
        Engine::Claude,
        Some(530),
        "lookout-b",
        "",
        true,
        true,
    );
    tick(&mut quiet_watch, 102.0, &idle, &rows);
    assert_eq!(
        said(&quiet_log).len(),
        before,
        "background ended, session normally done: no line"
    );
    for dt in [110.0, 206.0] {
        tick(&mut quiet_watch, dt, &busy, &rows);
    }
    let flipped = said(&quiet_log).len();
    quiet_watch.adjust(&[], &rows, t0 + 212.0, 90.0, &BTreeMap::new());
    assert_eq!(said(&quiet_log).len(), flipped, "session ended: no line");
    *net_now.lock().unwrap() = Some(BTreeMap::from([(542, 500_000)]));
    let netz = tick(&mut watch, 300.0, &busy, &rechnet);
    assert!(!netz.ready && !netz.watching, "network grows: runs");
    let offen = Session {
        digest: Some(Digest {
            turn_ended: false,
            ..Digest::default()
        }),
        ..busy.clone()
    };
    let offen_jetzt = tick(&mut watch, 600.0, &offen, &rows);
    assert!(
        !offen_jetzt.ready && !offen_jetzt.watching,
        "answer not finished: stays as claude agents reports it"
    );
    // Without a background shell something else keeps the session busy (say a background agent); Lookout does not guess.
    let ohne_shell: Vec<PsRow> = rows
        .iter()
        .filter(|r| ![540, 541, 542, 570].contains(&r.pid))
        .cloned()
        .collect();
    for dt in [700.0, 760.0, 820.0] {
        let s = tick(&mut watch, dt, &busy, &ohne_shell);
        assert!(
            !s.ready && !s.watching,
            "without a background command: as reported"
        );
    }
    // Kevin 2026-10-09 (team lead 12:18:46): another session's message starts a turn without a prompt line; the
    // transcript still shows the old turn end until the first answer. Claude reports `busy` at once: works.
    let (lead_log, report) = recorder();
    let mut lead_watch = watch_with(
        fixed(Some(BTreeMap::new())),
        fixed(Some(BTreeSet::new())),
        &[],
        report,
    );
    for dt in [0.0, 96.0] {
        tick(&mut lead_watch, dt, &busy, &rows);
    }
    let geweckt = Session {
        digest: busy.digest.clone(),
        ..Session::new(
            busy.id.clone(),
            Engine::Claude,
            Some(530),
            "lookout-b",
            "",
            false,
            true,
        )
    };
    let arbeitet = tick(&mut lead_watch, 99.0, &geweckt, &rows);
    assert!(
        !arbeitet.ready && !arbeitet.watching,
        "busy despite an old turn end in the transcript: works"
    );
    assert_eq!(
        said(&lead_log),
        [
            "Hintergrund ruht seit 90 s: lookout-b gilt als überwacht (Aufpasser läuft)",
            "Hintergrund arbeitet wieder: lookout-b"
        ]
    );
    let frage = Session {
        waiting: true,
        ..Session::new("claude:q", Engine::Claude, Some(530), "q", "", true, true)
    };
    assert!(
        tick(&mut watch, 0.0, &frage, &rows).waiting,
        "a question stays a question"
    );
    let codex = Session::new("codex:c", Engine::Codex, Some(530), "c", "", false, true);
    assert!(
        !tick(&mut watch, 2000.0, &codex, &rows).ready,
        "Claude only"
    );
}

/// A session with a resting watcher: `kevin` = Kevin's own message started the turn.
fn background_session(id: &str, name: &str, text: &str, kevin: bool) -> Session {
    Session {
        background_only: true,
        digest: Some(Digest {
            text: Some(text.into()),
            last: LastBlock::Text,
            turn_ended: true,
            user_turn: Some(kevin),
            ..Digest::default()
        }),
        ..Session::new(id, Engine::Claude, Some(530), name, "", false, true)
    }
}

/// Kevin 2026-10-01: if a session waits for its watcher but needs Kevin (question, request, done message, or his own
/// message started the turn), grey hid it (fresh turn ends 2026-09-28 to 10-01: 23 of 23). Then DONE as before 1e+.
#[test]
fn test_watcher_needs_kevin() {
    let rows = background_fixture();
    let t0 = 60_000.0;
    let (log, report) = recorder();
    let mut watch = watch_with(
        fixed(Some(BTreeMap::new())),
        fixed(Some(pids(&[542]))),
        &[],
        report,
    );
    let session =
        |text: &str, kevin: bool| background_session("claude:q", "lookout-q", text, kevin);
    let judge = |w: &mut BackgroundWatch, s: Session, dt: f64| {
        w.adjust(&[s], &rows, t0 + dt, 90.0, &BTreeMap::new())[0].clone()
    };
    let frage = session("CI is red on one flaky test. Retry it or skip it?", false);
    for dt in [0.0, 96.0] {
        judge(&mut watch, frage.clone(), dt);
    }
    let out = judge(&mut watch, frage, 99.0);
    assert!(
        out.ready && !out.watching,
        "a question in the answer: done instead of watching"
    );
    assert_eq!(
        said(&log),
        ["Hintergrund ruht seit 90 s: lookout-q gilt als fertig (Antwort an dich)"]
    );
    let ruhig = judge(
        &mut watch,
        session("Waiting for CI. Nothing needed from you.", false),
        102.0,
    );
    assert!(
        ruhig.watching && !ruhig.ready,
        "nothing for Kevin: watching as before"
    );
    let bitte = session(
        "Die Datei ist hochgeladen. Bitte klick im Shop auf Weiter.",
        false,
    );
    assert!(
        judge(&mut watch, bitte, 105.0).ready,
        "request to Kevin: done"
    );
    let antwort = session(
        "The three rounds can run at once; the limit is each family's usage window.",
        true,
    );
    assert!(
        judge(&mut watch, antwort, 108.0).ready,
        "answer to Kevin's own message: done, it is his turn"
    );
    // review quick 2026-10-01 (Codex): when the measurement fails, a watching session kept its state, also when its new
    // answer needs Kevin.
    let net_ok = fixed(Some(BTreeMap::new()));
    let mut wackel = watch_with(net_ok.clone(), fixed(Some(pids(&[542]))), &[], silent());
    let still = session("Waiting for CI. Nothing needed from you.", false);
    for dt in [0.0, 96.0] {
        judge(&mut wackel, still.clone(), dt);
    }
    assert!(
        judge(&mut wackel, still.clone(), 99.0).watching,
        "watching first"
    );
    *net_ok.lock().unwrap() = None;
    assert!(
        judge(&mut wackel, still, 102.0).watching,
        "measurement fails, answer unchanged: stays watching"
    );
    // review quick 2026-10-01 (GLM): without a readable answer text when in doubt done, not grey.
    let mut ohne_text = session("", false);
    ohne_text.digest.as_mut().unwrap().text = None;
    assert!(judge(&mut watch, ohne_text, 111.0).ready, "no text: done");
    let nachfrage = judge(
        &mut wackel,
        session("CI is green. Merge now?", false),
        105.0,
    );
    assert!(
        nachfrage.ready && !nachfrage.watching,
        "measurement fails, a new answer asks Kevin: done"
    );
    // Kevin 2026-10-01 evening: "nothing needed" in the first sentence wins, also if something is "complete" after it or
    // another session waits for Kevin. His own message keeps it done.
    let lage = session(
        "Nothing needed from you. The handover is complete. #13 waits for your answer by day.",
        false,
    );
    assert!(
        judge(&mut watch, lage, 114.0).watching,
        "a status report opening with 'nothing needed': watching"
    );
    let lage_kevin = session("Nothing needed from you. The handover is complete.", true);
    assert!(
        judge(&mut watch, lage_kevin, 117.0).ready,
        "answer to Kevin's own message: still done"
    );
    // Kevin 2026-10-03 (screenshot 08:58): if the answer to Kevin's message promises to report back and wants nothing
    // else from him, it only waits for its watcher: watching.
    let zusage = session(
        concat!(
            "Kevin, alles eingetragen. ✅\n\n- **Merkmalsliste:** freigegeben.\n\n",
            "Ich melde mich, sobald v3 durch ist."
        ),
        true,
    );
    assert!(
        judge(&mut watch, zusage, 120.0).watching,
        "promise on Kevin's message: watching"
    );
    let zusage_frage = session(
        "Alles eingetragen. Ich melde mich, sobald v3 durch ist. Soll ich v4 gleich starten?",
        true,
    );
    assert!(
        judge(&mut watch, zusage_frage, 123.0).ready,
        "promise with a question: done"
    );
    let zusage_bitte = session(
        "Der Abruf läuft. Ich melde mich, sobald du den Code im Portal eingegeben hast.",
        true,
    );
    assert!(
        judge(&mut watch, zusage_bitte, 126.0).ready,
        "promise with a request to Kevin: done"
    );
    assert!(
        promises_report("Ich melde mich mit der fertigen Zahl.")
            && promises_report("Sobald v3 steht, sage ich Bescheid."),
        "German promises"
    );
    assert!(
        promises_report("I'll report back when CI is green.")
            && promises_report("I’ll let you know once it lands."),
        "English promises, also with a typographic apostrophe"
    );
    assert!(
        !promises_report("Sag Bescheid, dann öffne ich dir das Fenster wieder.")
            && !promises_report("Melde dich, wenn es hakt."),
        "a request to Kevin is no promise"
    );
    // review quick 2026-10-03 (GLM): an order without a signal word after the promise stays done; the promise only counts
    // as the last sentence.
    let zusage_danach_auftrag = session(
        "Ich melde mich, sobald der Build durch ist. Merge in der Zwischenzeit PR #12.",
        true,
    );
    assert!(
        judge(&mut watch, zusage_danach_auftrag, 132.0).ready,
        "order after the promise: done"
    );
    assert!(
        !promises_report("I'll let you know when it's done. Approve the budget in the dashboard."),
        "English likewise"
    );
    assert!(
        promises_report("Ich melde mich, sobald v3 durch ist. ✅"),
        "an emoji after the promise is no new sentence"
    );
    // review quick 2026-10-03 (Codex): only the first person is a promise, and a request or condition on Kevin blocks.
    assert!(
        promises_report("Sobald v3 durch ist, melde ich mich."),
        "first person, inverted"
    );
    assert!(
        !promises_report("Melde mich im Browser an, damit ich den Export starten kann."),
        "imperative to Kevin"
    );
    assert!(
        !promises_report("Please approve the deployment now. I'll report back when CI is green."),
        "please"
    );
    assert!(
        !promises_report("Ich melde mich, wenn du den Code im Portal eingegeben hast."),
        "waits for Kevin first"
    );
    assert!(
        !promises_report("I'll let you know if you need to restart it."),
        "if you"
    );
    // review quick 2026-10-03 (GLM R2, Kimi R2): an order in the same clause structure stays done.
    assert!(
        !promises_report("I'll report back when CI is green. merge PR #12 in the meantime"),
        "lowercase after the dot"
    );
    assert!(
        !promises_report(
            "Ich melde mich, sobald der Build durch ist — merge in der Zwischenzeit PR #12."
        ),
        "dash"
    );
    assert!(
        !promises_report("Ich melde mich, sobald der Build durch ist: merge danach PR #12."),
        "colon"
    );
    assert!(
        !promises_report("Ich melde mich, sobald v3 steht.\n- Merge PR #12"),
        "a list after it"
    );
    assert!(
        promises_report("Ich melde mich, sobald v3 durch ist (ca. 16:00)."),
        "a time is no colon clause"
    );
    // Acceptance 2026-10-03 (Kimi): an emoji at the sentence start does not hide the order after it.
    assert!(
        !promises_report(
            "Alles eingetragen. Ich melde mich, sobald der Build durch ist. ✅ Merge danach PR #12."
        ),
        "emoji before the order"
    );
    assert!(
        !promises_report("Ich melde mich, sobald der Build durch ist. 1) Merge PR #12"),
        "number before the order"
    );
    let fertig_und_zusage = session(
        "Der Auftrag ist fertig. Ich melde mich, sobald der Kontrolllauf durch ist.",
        true,
    );
    assert!(
        judge(&mut watch, fertig_und_zusage, 135.0).watching,
        "Kevin 2026-10-03: done + promise = the session only waits"
    );
    let ohne_zusage = session("Alles eingetragen. v3 läuft bis etwa 16 Uhr.", true);
    assert!(
        judge(&mut watch, ohne_zusage, 129.0).ready,
        "answer to Kevin's message without a promise: still done"
    );
}

/// Kevin 2026-10-09 (team lead, 6 days: 211 of 523 turn ends without his message DONE although it only waited): a lead
/// session (`lead_names`) reports on other windows. With a resting watcher only a question or Kevin's own unseen message
/// makes it done.
#[test]
fn test_lead_reports_on_other_windows() {
    let rows = background_fixture();
    let t0 = 80_000.0;
    let mut watch = watch_with(
        fixed(Some(BTreeMap::new())),
        fixed(Some(pids(&[542]))),
        &["Beta · Lead"],
        silent(),
    );
    let session = |text: &str, name: &str, kevin: bool| {
        background_session(&format!("claude:{name}"), name, text, kevin)
    };
    let mut dt = 0.0;
    let mut judged = |w: &mut BackgroundWatch, s: Session| {
        dt += 3.0;
        w.adjust(&[s], &rows, t0 + 96.0 + dt, 90.0, &BTreeMap::new())[0].clone()
    };
    let andere = "Kevin, two windows wait for you: #8's look at its page and #12's merge. One watcher runs. I wait.";
    for t in [0.0, 96.0] {
        watch.adjust(
            &[session(andere, "Beta · Lead", false)],
            &rows,
            t0 + t,
            90.0,
            &BTreeMap::new(),
        );
    }
    assert!(
        judged(&mut watch, session(andere, "Beta · Lead", false)).watching,
        "lead: a report on other windows is watching"
    );
    assert!(
        judged(
            &mut watch,
            session(
                "Kevin, #8's handover is complete: the new window runs since 22:13.",
                "Beta · Lead",
                false
            )
        )
        .watching,
        "lead: 'complete' about another card is watching"
    );
    assert!(
        judged(
            &mut watch,
            session(
                "Kevin, #8 waits on the case page. Shall I hold #12 until then?",
                "Beta · Lead",
                false
            )
        )
        .ready,
        "lead with a question: done"
    );
    assert!(
        judged(
            &mut watch,
            session("Kevin, noted: the order stays.", "Beta · Lead", true)
        )
        .ready,
        "Kevin's own message: done"
    );
    assert!(
        judged(&mut watch, session(andere, "Beta · Worker", false)).ready,
        "another session: word list as before"
    );
    assert!(
        judged(&mut watch, session(andere, "beta · lead", false)).ready,
        "the name must match exactly"
    );
    // review quick 2026-10-09 (Codex + GLM): the menu shows the own name from the transcript, not the status file's.
    let mut umbenannt = session(andere, "beta-lead", false);
    umbenannt.digest.as_mut().unwrap().name = Some("Beta · Lead".into());
    assert!(
        judged(&mut watch, umbenannt).watching,
        "the name as in the menu (transcript) counts too"
    );
}

/// Kevin 2026-11-07 (screenshots 11:44 one session with its watcher, 11:46 another waiting for its workers): a lead
/// said "✓", Kevin read it, the lead waits for its watcher or workers again: watching. Unread, "?" or a worker: done.
#[test]
fn test_lead_done_read() {
    let done = "Merged.\n✓ Lookout: fertig (merged as #745)";
    let ask = "Merged.\n? Lookout: Bitte (Go für #12)";
    let answered = 1_790_600_000.0;
    let read = BTreeMap::from([("claude:lead".to_owned(), answered + 60.0)]);
    let unread = BTreeMap::from([("claude:lead".to_owned(), answered - 60.0)]);

    // Own watcher (`BackgroundWatch`), the turn started by Kevin's message.
    let rows = background_fixture();
    let t0 = answered + 10.0;
    let mut watch = watch_with(
        fixed(Some(BTreeMap::new())),
        fixed(Some(pids(&[542]))),
        &["Beta · Lead"],
        silent(),
    );
    let bg = |text: &str, name: &str| {
        let mut s = background_session("claude:lead", name, text, true);
        s.digest.as_mut().unwrap().answer_at = Some(answered);
        s
    };
    for dt in [0.0, 96.0] {
        watch.adjust(&[bg(done, "Beta · Lead")], &rows, t0 + dt, 90.0, &unread);
    }
    let mut judge = |s: Session, dt: f64, seen: &BTreeMap<String, Time>| {
        watch.adjust(&[s], &rows, t0 + dt, 90.0, seen)[0].clone()
    };
    assert!(
        judge(bg(done, "Beta · Lead"), 99.0, &unread).ready,
        "watcher, unread: done"
    );
    let w = judge(bg(done, "Beta · Lead"), 102.0, &read);
    assert!(w.watching && !w.ready, "watcher, read: watching");
    assert!(
        judge(bg(ask, "Beta · Lead"), 105.0, &read).ready,
        "watcher, '?': stays with Kevin"
    );
    assert!(
        judge(bg(done, "Beta · #9"), 108.0, &read).ready,
        "watcher, a worker: done (Kevin closes it)"
    );

    // No own watcher, waits for its workers (`Standby`); the status file says "beta-ad", the menu "Beta · Lead".
    let now = answered + 300.0;
    let session = |text: &str, own_name: &str| Session {
        digest: Some(Digest {
            text: Some(text.into()),
            last: LastBlock::Text,
            turn_ended: true,
            user_turn: Some(true),
            answer_at: Some(answered),
            name: Some(own_name.into()),
            partners: BTreeSet::from(["Beta · #8".to_owned()]),
            ..Digest::default()
        }),
        ..Session::new(
            "claude:lead",
            Engine::Claude,
            Some(53833),
            "beta-ad",
            "/x",
            true,
            true,
        )
    };
    let worker = Session::new(
        "claude:w8",
        Engine::Claude,
        Some(61143),
        "Beta · #8",
        "/x",
        false,
        true,
    );
    let (log, report) = recorder();
    let mut standby = Standby::new(vec![], BTreeSet::from(["Beta · Lead".to_owned()]), report);
    let mut judge = |s: Session, with_worker: bool, seen: &BTreeMap<String, Time>| {
        let mut all = vec![s];
        if with_worker {
            all.push(worker.clone());
        }
        standby.adjust(&all, now, seen)[0].clone()
    };
    assert!(
        judge(session(done, "Beta · Lead"), true, &unread).ready,
        "workers, unread: done"
    );
    let w = judge(session(done, "Beta · Lead"), true, &read);
    assert!(w.watching && !w.ready, "workers, read: watching");
    assert!(
        said(&log).iter().any(|l| l.contains("(Lead, ✓ gelesen)")),
        "the log names the reason, got {:?}",
        said(&log)
    );
    assert!(
        judge(session(done, "Beta · Lead"), false, &read).ready,
        "no worker runs any more: done, nothing wakes it"
    );
    assert!(
        judge(session(ask, "Beta · Lead"), true, &read).ready,
        "workers, '?': stays with Kevin"
    );
    assert!(
        judge(session(done, "Beta · #9"), true, &read).ready,
        "a worker that talks with another one: done"
    );
    // Same wake sources as "◌" (review quick 2026-11-07, Grok): the last peer by its socket pid, an own timer.
    let mut by_pid = session(done, "Beta · Lead");
    let d = by_pid.digest.as_mut().unwrap();
    d.partners.clear();
    d.peer_pid = Some(61143);
    assert!(
        judge(by_pid, true, &read).watching,
        "the last peer (socket pid) still runs: watching"
    );
    let timer = |at: Time| {
        let mut s = session(done, "Beta · Lead");
        s.digest
            .as_mut()
            .unwrap()
            .wakeups
            .insert("loop".into(), Some(at));
        s
    };
    assert!(
        judge(timer(now + 60.0), false, &read).watching,
        "an own timer is due: watching"
    );
    assert!(
        judge(timer(now - 400.0), false, &read).ready,
        "the timer is long past: done"
    );
}

/// Kevin 2026-11-07 (screenshot 12:42): "? … (Karten bereit)" is done on the card page,
/// not in the window: it waits for Kevin until he has read it, then the window watches again (a worker parks).
#[test]
fn test_ask_until_read() {
    let names = ["Kartenseite".to_owned()];
    let answered = 1_790_700_000.0;
    let read = BTreeMap::from([("claude:x".to_owned(), answered + 30.0)]);
    let unread = BTreeMap::new();
    let card = "Zwei Karten liegen bereit.\n? Lookout: Bitte Repost und Like (Kartenseite)";
    let marked = |text: &str, seen: &BTreeMap<String, Time>| {
        let mut s = vec![background_session(
            "claude:x",
            "Studio · X-Redaktion",
            text,
            false,
        )];
        s[0].digest.as_mut().unwrap().answer_at = Some(answered);
        mark_read_asks(&mut s, &names, &[], &[], seen);
        s.remove(0)
    };
    let status = |s: &Session| s.digest.as_ref().unwrap().line(&[]).map(|l| l.status);
    assert_eq!(
        status(&marked(card, &unread)),
        Some(LineStatus::Ask),
        "unread: asks"
    );
    assert_eq!(
        status(&marked(card, &read)),
        Some(LineStatus::Nothing),
        "read: needs nothing"
    );
    assert_eq!(
        status(&marked(
            "? Lookout: Bitte hier entscheiden, dann Kartenseite",
            &read
        )),
        Some(LineStatus::Ask),
        "'here': still a request in this window"
    );
    assert_eq!(
        status(&marked("? Lookout: Bitte entscheiden (A oder B)", &read)),
        Some(LineStatus::Ask),
        "a request without the card page stays, read or not"
    );
    // Only "(Kartenseite)" at the end counts.
    for here in [
        "? Lookout: Bitte die Kartenseite freigeben",
        "? Lookout: Bitte Kartenseite mit someone@example.com als Editor teilen und 1, 2 oder 3 für Postiz wählen",
        "? Lookout: Bitte die offenen Karten auf der Kartenseite posten",
    ] {
        assert_eq!(
            status(&marked(here, &read)),
            Some(LineStatus::Ask),
            "{here}: the card page only mentioned, stays"
        );
    }
    for there in [
        "? Lookout: Bitte die Antwort an Sam Doe posten (Kartenseite)",
        "? Lookout: Bitte Repost (kartenseite) ",
        "? Lookout: Bitte (Kartenseite)",
    ] {
        assert_eq!(
            status(&marked(there, &read)),
            Some(LineStatus::Nothing),
            "{there}: ends with the card page, read"
        );
    }
    // review quick 2026-11-07 (GLM): in the window named in `other_window_names` itself a request stays a request.
    let mut key = vec![background_session(
        "claude:x",
        "Acme · Key account",
        "? Lookout: Bitte Freigabe im Key account, dann Like (Kartenseite)",
        false,
    )];
    key[0].digest.as_mut().unwrap().answer_at = Some(answered);
    let others = ["key account".to_owned()];
    key[0].digest.as_mut().unwrap().name = Some("Acme · Key account".into());
    mark_read_asks(&mut key, &names, &others, &[], &read);
    assert_eq!(
        key[0]
            .digest
            .as_ref()
            .unwrap()
            .line(&others)
            .map(|l| l.status),
        Some(LineStatus::Ask),
        "the key account's own request stays, read or not"
    );
    let mut again = marked(card, &read);
    mark_read_asks(std::slice::from_mut(&mut again), &names, &[], &[], &unread);
    assert_eq!(
        status(&again),
        Some(LineStatus::Ask),
        "decided anew each tick"
    );
    let mut early = marked(card, &read);
    early.digest.as_mut().unwrap().answer_at = Some(answered + 60.0);
    mark_read_asks(std::slice::from_mut(&mut early), &names, &[], &[], &read);
    assert_eq!(
        status(&early),
        Some(LineStatus::Ask),
        "seen before this answer: not read"
    );

    // The whole tick order: watcher, then the line pass.
    let rows = background_fixture();
    let t0 = answered + 10.0;
    let mut watch = watch_with(
        fixed(Some(BTreeMap::new())),
        fixed(Some(pids(&[542]))),
        &[],
        silent(),
    );
    let mut tick = |seen: &BTreeMap<String, Time>, dt: f64| {
        let s = marked(card, seen);
        let out = watch.adjust(&[s], &rows, t0 + dt, 90.0, seen);
        apply_lookout_lines(out, &[]).remove(0)
    };
    tick(&unread, 0.0);
    tick(&unread, 96.0);
    let before = tick(&unread, 99.0);
    assert!(before.waiting && before.ready, "unread: waits for you");
    let after = tick(&read, 102.0);
    assert!(
        after.watching && !after.ready && !after.waiting,
        "read: watching again, the radar runs"
    );
}

/// Kevin 2026-11-09 (screenshot 02:42, a lead session: "◌ … (Wochenauswertung am Montag)", 36 min FERTIG): a
/// lead whose read line needs nothing rests, even with nothing due to wake it. Unread, "✓", another window: done.
#[test]
fn test_lead_rests_without_a_waker() {
    let nothing = "Gesichert.\n◌ Lookout: braucht nichts (Wochenauswertung am Montag)";
    let done = "Gesichert.\n✓ Lookout: fertig (gesichert)";
    let answered = 1_790_800_000.0;
    let now = answered + 300.0;
    let read = BTreeMap::from([("claude:lead".to_owned(), answered + 60.0)]);
    let unread = BTreeMap::new();
    let session = |text: &str, own_name: &str| Session {
        digest: Some(Digest {
            text: Some(text.into()),
            last: LastBlock::Text,
            turn_ended: true,
            answer_at: Some(answered),
            name: Some(own_name.into()),
            ..Digest::default()
        }),
        ..Session::new(
            "claude:lead",
            Engine::Claude,
            Some(53833),
            "beta-ad",
            "/x",
            true,
            true,
        )
    };
    let (log, report) = recorder();
    let mut standby = Standby::new(vec![], BTreeSet::from(["Beta · Lead".to_owned()]), report);
    let mut judge =
        |s: Session, seen: &BTreeMap<String, Time>| standby.adjust(&[s], now, seen)[0].clone();
    assert!(
        judge(session(nothing, "Beta · Lead"), &unread).ready,
        "unread: Kevin still sees it"
    );
    let w = judge(session(nothing, "Beta · Lead"), &read);
    assert!(w.watching && !w.ready, "read: rests");
    assert!(
        said(&log)
            .iter()
            .any(|l| l.contains("Beta") || l.contains("beta")),
        "the log says so, got {:?}",
        said(&log)
    );
    assert!(
        judge(session(done, "Beta · Lead"), &read).ready,
        "a read '✓' without a waker stays done"
    );
    assert!(
        judge(session(nothing, "Beta · #9"), &read).ready,
        "not a lead: nothing wakes it, done"
    );
}

/// Kevin 2026-11-07 (screenshot 18:36, a lead session): "wenn er eine frage hat muss es dialog sein sonst
/// sehe ich es nicht". A lead's text "?" waits only until Kevin has read it; its dialog (open question) still waits.
#[test]
fn test_lead_ask_read() {
    let leads = ["Studio · Team-Lead".to_owned()];
    let answered = 1_790_700_000.0;
    let read = BTreeMap::from([("claude:l".to_owned(), answered + 30.0)]);
    let unread = BTreeMap::new();
    let ask =
        "Willst du ihm antworten?\n? Lookout: Bitte hier antworten: Antwort-Karte, ja oder nein?";
    let marked = |name: &str, own: Option<&str>, seen: &BTreeMap<String, Time>| {
        let mut s = vec![background_session("claude:l", name, ask, false)];
        let d = s[0].digest.as_mut().unwrap();
        d.answer_at = Some(answered);
        d.name = own.map(str::to_owned);
        mark_read_asks(&mut s, &[], &[], &leads, seen);
        s.remove(0)
    };
    let status = |s: &Session| s.digest.as_ref().unwrap().line(&[]).map(|l| l.status);
    let lead = "Studio · Team-Lead";
    assert_eq!(
        status(&marked(lead, None, &unread)),
        Some(LineStatus::Ask),
        "unread: asks"
    );
    assert_eq!(
        status(&marked(lead, None, &read)),
        Some(LineStatus::Nothing),
        "read: needs nothing, 'here' or not"
    );
    assert_eq!(
        status(&marked("studio-ad", Some(lead), &read)),
        Some(LineStatus::Nothing),
        "the lead's own name from the transcript counts too"
    );
    assert_eq!(
        status(&marked("Studio · X-Redaktion", None, &read)),
        Some(LineStatus::Ask),
        "not a lead: the read '?' stays with Kevin"
    );
    let line_pass = |s: Session| apply_lookout_lines(vec![s], &[]).remove(0);
    assert!(
        !line_pass(marked(lead, None, &read)).waiting,
        "read text '?': not waiting"
    );
    let mut dialog = marked(lead, None, &read);
    dialog.waiting = true;
    assert!(
        line_pass(dialog).waiting,
        "an open dialog still waits for Kevin"
    );
}

/// Kevin 2026-10-04 (screenshot 22:38): the answer to Kevin's own message is done until he had its window in front of
/// him once after it; then it counts like an answer he did not start (watching, if it wants nothing from him).
#[test]
fn test_seen_outside_front() {
    let seen = BTreeMap::from([("a".to_owned(), 900.0), ("b".to_owned(), 800.0)]);
    assert_eq!(seen_outside_front(&seen, None), seen);
    let held = ("a".to_owned(), Some(100.0));
    let out = seen_outside_front(&seen, Some(&held));
    assert_eq!(out["a"], 100.0, "as it was before it came to the front");
    assert_eq!(out["b"], 800.0, "the others are untouched");
    let never = ("a".to_owned(), None);
    assert!(!seen_outside_front(&seen, Some(&never)).contains_key("a"));
}

#[test]
fn test_watcher_answer_seen() {
    let rows = background_fixture();
    let t0 = 70_000.0;
    let answered = t0 - 30.0;
    let (log, report) = recorder();
    let mut watch = watch_with(
        fixed(Some(BTreeMap::new())),
        fixed(Some(pids(&[542]))),
        &[],
        report,
    );
    let session = |text: &str, answer_at: Option<Time>| {
        let mut s = background_session("claude:s", "lookout-s", text, true);
        s.digest.as_mut().unwrap().answer_at = answer_at;
        s
    };
    let judge = |w: &mut BackgroundWatch, s: &Session, dt: f64, seen: &BTreeMap<String, Time>| {
        w.adjust(std::slice::from_ref(s), &rows, t0 + dt, 90.0, seen)[0].clone()
    };
    let none = BTreeMap::new();
    let antwort = session(
        concat!(
            "Der Zusatz ist notiert ✅: Er kommt als eigene Zeile in den Bericht, und danach entscheidest du. ",
            "Die Punkte 1, 2 und 4–6 sind noch offen, das hat bis Mittwoch Zeit."
        ),
        Some(answered),
    );
    for dt in [0.0, 96.0] {
        judge(&mut watch, &antwort, dt, &none);
    }
    assert!(
        judge(&mut watch, &antwort, 99.0, &none).ready,
        "unseen: done"
    );
    let seen_at = |t: Time| BTreeMap::from([(antwort.id.clone(), t)]);
    let vorher = judge(&mut watch, &antwort, 102.0, &seen_at(answered - 5.0));
    assert!(
        vorher.ready && !vorher.watching,
        "only had it in front before the answer: done"
    );
    let gesehen = judge(&mut watch, &antwort, 105.0, &seen_at(answered + 5.0));
    assert!(
        gesehen.watching && !gesehen.ready,
        "had it in front after the answer: watching"
    );
    assert_eq!(
        said(&log).last().map(String::as_str),
        Some(
            "Hintergrund ruht seit 90 s: lookout-s gilt als überwacht (Antwort gesehen, Aufpasser läuft)"
        )
    );
    let fremd = judge(
        &mut watch,
        &antwort,
        108.0,
        &BTreeMap::from([("claude:andere".to_owned(), t0)]),
    );
    assert!(fremd.ready, "another session seen: done");
    // Seen does not mean "wants nothing": a question, a request or a done message keep the window done.
    let seen = seen_at(t0);
    for text in [
        "CI ist grün. Soll ich v4 starten?",
        "Der Export liegt bereit. Bitte lade ihn hoch.",
        "Card #9 is done.",
    ] {
        assert!(
            judge(&mut watch, &session(text, Some(answered)), 111.0, &seen).ready,
            "seen, but '{text}': done"
        );
    }
    // As without Kevin's message: "nothing needed" in the first sentence wins.
    let lage = session(
        "Nothing needed from you. The handover is complete.",
        Some(answered),
    );
    assert!(
        judge(&mut watch, &lage, 114.0, &seen).watching,
        "seen + 'nothing needed' first: watching"
    );
    // When in doubt done: without the answer's time "after" cannot be checked.
    let ohne_zeit = session("Die Punkte sind noch offen.", None);
    assert!(
        judge(&mut watch, &ohne_zeit, 117.0, &seen).ready,
        "without an answer time: done"
    );
    // If the measurement fails, the seen answer keeps its state (`keep_quiet`).
    let net_ok = fixed(Some(BTreeMap::new()));
    let mut wackel = watch_with(net_ok.clone(), fixed(Some(pids(&[542]))), &[], silent());
    for dt in [0.0, 96.0] {
        judge(&mut wackel, &antwort, dt, &seen);
    }
    assert!(
        judge(&mut wackel, &antwort, 99.0, &seen).watching,
        "watching first"
    );
    *net_ok.lock().unwrap() = None;
    assert!(
        judge(&mut wackel, &antwort, 102.0, &seen).watching,
        "measurement fails, answer seen: stays watching"
    );
    let neu = session("Die Punkte sind noch offen.", Some(t0 + 101.0));
    assert!(
        judge(&mut wackel, &neu, 105.0, &seen).ready,
        "measurement fails, a new unseen answer: done"
    );
}

/// Kevin 2026-10-01 evening: opens the answer explicitly with "nothing needed" (first sentence without question, request
/// or done message, no question mark anywhere), the rest does not count for the watcher. Measured on 798 cases of
/// 2026-10-01: grey 239 -> 282 of 380, hid 1 done message and 1 uncertain request.
#[test]
fn test_opens_with_nothing_needed() {
    let lage = concat!(
        "Nothing needed from you. The handover to the next window is complete and clean. Three windows run into ",
        "the night: #13 (replay), #32 (trial), PR-3 (waits for your answer by day)."
    );
    assert!(
        !asks_nothing(lage, false) && opens_with_nothing_needed(lage),
        "the first sentence decides"
    );
    assert!(
        opens_with_nothing_needed("Für dich gerade nichts. #13 wartet auf deine Antwort."),
        "German"
    );
    assert!(
        opens_with_nothing_needed("  Nothing needed from you!\n\n#13 waits for your merge."),
        "exclamation mark, empty lines"
    );
    assert!(
        !opens_with_nothing_needed("Nothing needed from you. Should I merge #13 now?"),
        "a question mark still blocks"
    );
    assert!(
        !opens_with_nothing_needed("Card #9 is finished. Nothing needed from you."),
        "'nothing needed' only in the second sentence"
    );
    assert!(
        !opens_with_nothing_needed("Nothing needed from you except the merge."),
        "request in the first sentence"
    );
    assert!(
        !opens_with_nothing_needed("Nothing needed from you, I'm done."),
        "done message in the first sentence"
    );
    assert!(
        !opens_with_nothing_needed("All good\nNothing needed from you."),
        "a line break ends the first sentence"
    );
    assert!(!opens_with_nothing_needed(""), "empty");
    // review quick 2026-10-01 (Codex): an abbreviation dot ends no sentence, markdown after the dot does.
    assert!(
        !opens_with_nothing_needed(
            "Nothing needed from you for e.g. the logs, but I need your approval for deployment."
        ),
        "an abbreviation dot ends no sentence"
    );
    assert!(
        !opens_with_nothing_needed(
            "Nothing needed from you for e.g.  the logs, but I need your approval."
        ),
        "also with two spaces"
    );
    assert!(
        opens_with_nothing_needed("**Nothing needed from you.** The handover is complete."),
        "markdown after the dot"
    );
    // review quick 2026-10-01 round 3 (Codex): opening markdown before a lowercase letter ends no sentence, closing German
    // quotes and guillemets belong to the sentence end.
    assert!(
        !opens_with_nothing_needed(
            "Nothing needed from you for e.g. **the logs**, but I need your approval."
        ),
        "markdown after the abbreviation dot"
    );
    assert!(
        opens_with_nothing_needed("„Für dich gerade nichts.“ #13 wartet auf deine Antwort."),
        "German quotes"
    );
    assert!(
        opens_with_nothing_needed("»Für dich gerade nichts.« #13 wartet auf deine Antwort."),
        "guillemets"
    );
    // review quick 2026-10-01 round 4 (Codex): after the white space the first letter or digit counts, every character
    // before it (backticks, markdown, brackets) is skipped.
    assert!(
        !opens_with_nothing_needed(
            "Nothing needed from you for e.g. `the logs`, but I need your approval."
        ),
        "inline code"
    );
    assert!(
        !opens_with_nothing_needed(
            "Nothing needed from you for e.g. ~~the logs~~, but I need your approval."
        ),
        "strike-through"
    );
    assert!(
        !opens_with_nothing_needed("Nothing needed from you. `make test` waits for your merge."),
        "lowercase code after the dot: when in doubt the same sentence, so done"
    );
    assert!(
        opens_with_nothing_needed("Nothing needed from you. `Make` waits for your merge."),
        "uppercase code: a new sentence"
    );
    // R11 C (Kevin 2026-10-05): an earlier question still open blocks even behind "nothing needed" in the first sentence.
    assert!(
        !opens_with_nothing_needed(
            "Kevin, nothing needed from you. #12 runs.\n\nStill open with you: the planning question."
        ),
        "open question at the end"
    );
    assert!(
        asks_nothing(
            "Part 1 of the check is done. The lead pings me when #12 is through.",
            false
        ),
        "watcher: partial step (A')"
    );
    // Kevin 2026-10-06: "auf deinen Mac" is a place, no request; waiting for Kevin stays one.
    assert!(
        asks_nothing(
            "Ich habe sie auf deinen Mac durchgereicht. Nach einem Neustart sag Bescheid.",
            false
        ),
        "a place, no request"
    );
    assert!(
        asks_nothing("Die Datei liegt auf deinem Mac.", false),
        "place in the dative"
    );
    assert!(
        !asks_nothing("Kevin, protokolliert. Ich warte auf deinen Stick.", false),
        "waits for Kevin"
    );
    assert!(
        !asks_nothing("Ich warte nur noch auf dein Go.", false),
        "waits for Kevin, with 'nur noch'"
    );
    assert!(
        !asks_nothing("Das wartet auf deine Freigabe.", false),
        "'auf deine' as a whole word"
    );
}

/// Kevin 2026-10-01 evening: a window that another session started by handover (`claude "…"`) has its first turn not
/// from Kevin; the prompt is among the process's start arguments. Typed stays Kevin's.
#[test]
fn test_launch_prompt_is_not_kevins() {
    let brief = "Read the brief at /tmp/handover/x.md completely and work through it.";
    let strings = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let args = strings(&["/usr/local/bin/claude", "--model", "opus", brief]);
    let unquoted: Vec<String> = std::iter::once("claude")
        .chain(brief.split(' '))
        .map(str::to_owned)
        .collect();
    assert!(launch_prompt(brief, &args), "start prompt");
    assert!(
        launch_prompt(&format!("{brief}\n"), &args),
        "a line end at the prompt does not matter"
    );
    assert!(
        !launch_prompt(brief, &unquoted),
        "typed without quotes: when in doubt Kevin's turn"
    );
    assert!(
        !launch_prompt("Please also check the logs from yesterday.", &args),
        "typed"
    );
    assert!(
        !launch_prompt("work through it", &args),
        "typed, only by chance inside the start prompt"
    );
    assert!(
        !launch_prompt("the brief at /tmp/handover/x.md", &unquoted),
        "typed, part of an unquoted start prompt"
    );
    assert!(!launch_prompt("opus", &args), "too short to be sure");
    // review quick 2026-10-01 (Codex R1 + R3): the value of a free-text option (`-n/--name`, `--system-prompt`,
    // `--append-system-prompt`) is no start prompt.
    let name = "Prüfe die CI-Läufe jetzt";
    assert!(
        !launch_prompt(name, &strings(&["claude", "--name", name]))
            && !launch_prompt(name, &strings(&["claude", "-n", name])),
        "a session name, no handover"
    );
    assert!(
        launch_prompt(brief, &strings(&["claude", "--name", name, brief])),
        "start prompt after the name"
    );
    // review quick 2026-10-01 round 4 (Codex): a start prompt is a sentence; paths, ids, model names have no white space.
    let pfad = "/Users/test/Projects/Alpha/Tools";
    assert!(
        !launch_prompt(pfad, &strings(&["claude", "--add-dir", pfad])),
        "a path as first message"
    );
    // The triggering prompt is remembered; Kevin's message mid-turn is always typed.
    let start = format!(
        r#"{{"type":"user","origin":{{"kind":"human"}},"turnPosition":{{"promptIndex":1,"turnIndex":1}},"timestamp":"2026-10-01T19:02:01.288Z","message":{{"content":"{brief}"}}}}"#
    );
    let mitten = r#"{"type":"attachment","attachment":{"type":"queued_command","commandMode":"prompt","origin":{"kind":"human"},"prompt":"und #12?"}}"#;
    let d = digest_transcript(&start);
    assert!(
        d.user_turn == Some(true) && d.user_prompt.as_deref() == Some(brief),
        "got {:?}",
        d.user_prompt
    );
    assert!(
        digest_transcript(&lines(&[&start, mitten]))
            .user_prompt
            .is_none(),
        "message mid-turn: typed"
    );
    // review quick 2026-10-01 (Codex): only a session's first prompt can be a start prompt; the same text sent again
    // later (input history) is typed; without a position too when in doubt.
    let nochmal = format!(
        r#"{{"type":"user","origin":{{"kind":"human"}},"turnPosition":{{"promptIndex":3,"turnIndex":7}},"message":{{"content":"{brief}"}}}}"#
    );
    assert!(
        digest_transcript(&lines(&[&start, &nochmal]))
            .user_prompt
            .is_none(),
        "sent again later"
    );
    let ohne_position = format!(
        r#"{{"type":"user","origin":{{"kind":"human"}},"message":{{"content":"{brief}"}}}}"#
    );
    assert!(
        digest_transcript(&ohne_position).user_prompt.is_none(),
        "without a position"
    );
    let started_at = d.user_prompt_at.map(|t| t - 3.0); // process started 3 s before the prompt
    let corrected = |d: &Digest, a: Option<Vec<String>>, s: Option<Time>| {
        launch_corrected(d.clone(), || a, || s).user_turn
    };
    assert_eq!(
        corrected(&d, Some(args.clone()), started_at),
        Some(false),
        "handover: not Kevin's turn"
    );
    assert_eq!(
        corrected(
            &d,
            Some(strings(&["claude", "--resume", "abc"])),
            started_at
        ),
        Some(true),
        "typed: Kevin's turn"
    );
    assert_eq!(
        corrected(&d, None, started_at),
        Some(true),
        "start arguments not readable: as before"
    );
    // review quick 2026-10-01 round 3 (Codex): after `/clear` the same process begins a new conversation; Kevin typing the
    // same text there is his turn. A start prompt comes within 30 s of the process start (measured 3 s; round 4: 2
    // minutes allowed an early /clear).
    assert_eq!(
        corrected(&d, Some(args.clone()), started_at.map(|t| t - 45.0)),
        Some(true),
        "45 s after the start: typed"
    );
    assert_eq!(
        corrected(&d, Some(args.clone()), started_at.map(|t| t - 3600.0)),
        Some(true),
        "an hour after the start (after /clear): Kevin's turn"
    );
    assert_eq!(
        corrected(&d, Some(args.clone()), None),
        Some(true),
        "start time unknown: as before"
    );
    let ohne_zeit = Digest {
        user_prompt_at: None,
        ..d.clone()
    };
    assert_eq!(
        corrected(&ohne_zeit, Some(args.clone()), started_at),
        Some(true),
        "prompt without time: as before"
    );
    // (Swift's last assert, `processArguments(getpid()) == CommandLine.arguments`, reads sysctl: stage 2.)
}

/// Server or watcher? (Kevin 2026-09-27) A server listens on a port. `lsof -Fp` measured live 2026-09-27: per process
/// "p<pid>", below it "f<fd>"; exit 1 already when nothing listens or one of the processes no longer lives.
#[test]
fn test_listening_pids() {
    assert_eq!(
        parse_lsof_pids("p957\nf14\nf16\np22053\nf21\n"),
        pids(&[957, 22053]),
        "one p line per process"
    );
    assert!(
        parse_lsof_pids("").is_empty() && parse_lsof_pids("kaputt\n").is_empty(),
        "nothing listens"
    );
}

// ---------- Wartet auf andere Session oder Wecker (Kevin 2026-09-27) ----------

/// Forms live from the transcripts of 2026-09-27 (CLI 2.1.283), cut to what matters.
#[test]
fn test_digest_peer_and_wakeups() {
    let kevin = r#"{"type":"user","origin":{"kind":"human"},"message":{"content":"so this session stays open and waits?"}}"#;
    let lead = r#"{"type":"user","isMeta":true,"origin":{"kind":"peer","from":"uds:/tmp/cc-socks/33134.sock","verifiedPeerPid":33134,"name":"Beta · Lead","fromMode":"prompting"},"message":{"content":"Another Claude session sent a message: …"}}"#;
    let handback = r#"{"type":"user","isMeta":true,"origin":{"kind":"peer","from":"ac2046f78b16995a7","senderTaskId":"ac2046f78b16995a7","handback":true},"message":{"content":"[Subagent hand-back] …"}}"#;
    let meldung = r#"{"type":"user","origin":{"kind":"task-notification"},"message":{"content":"<task-notification>fertig</task-notification>"}}"#;
    let dig = |l: &[&str]| digest_transcript(&lines(l));
    let d = dig(&[kevin, lead]);
    assert!(
        d.peer_pid == Some(33134) && d.peer_name.as_deref() == Some("Beta · Lead"),
        "another session's message after Kevin's prompt"
    );
    // R11 (Kevin 2026-10-05): Kevin's question no longer ends the conversation with the other session; he asked whether
    // he could close, and it waited for the lead session after that.
    let d = dig(&[lead, kevin]);
    assert!(
        d.peer_pid == Some(33134) && d.peer_name.as_deref() == Some("Beta · Lead"),
        "Kevin's new prompt does not end the conversation"
    );
    assert!(
        dig(&[kevin, handback]).peer_name.is_none(),
        "an own subagent's report is no other session"
    );
    assert_eq!(
        dig(&[lead, meldung]).peer_pid,
        Some(33134),
        "a background notice is no prompt from Kevin"
    );
    // Who started the turn? Kevin's prompt yes, the other session and a background notice not (Kevin 2026-10-01).
    assert_eq!(dig(&[lead, kevin]).user_turn, Some(true), "Kevin's prompt");
    assert_eq!(
        dig(&[kevin, lead]).user_turn,
        Some(false),
        "the other session's message"
    );
    assert_eq!(
        dig(&[kevin, meldung]).user_turn,
        Some(false),
        "background notice"
    );
    assert_eq!(
        dig(&[kevin, handback]).user_turn,
        Some(true),
        "an own subagent's report: Kevin's turn goes on"
    );
    // review quick 2026-10-01 (GLM): the timer starts the turn; its prompt is a system insert without origin.
    let feuer = r#"{"type":"system","subtype":"scheduled_task_fire","taskId":"c10455b9","timestamp":"2026-09-27T09:57:00.519Z"}"#;
    let weck_prompt = r#"{"type":"user","isMeta":true,"message":{"content":"Prüfe den Fortschritt der Prüfer."}}"#;
    assert_eq!(
        dig(&[kevin, feuer, weck_prompt]).user_turn,
        Some(false),
        "the timer starts the turn"
    );
    // Kevin writes into a turn the other session started (attachment instead of prompt).
    let mitten = r#"{"type":"attachment","attachment":{"type":"queued_command","commandMode":"prompt","origin":{"kind":"human"},"prompt":"und #12?"}}"#;
    assert_eq!(
        dig(&[kevin, lead, mitten]).user_turn,
        Some(true),
        "Kevin's message mid-turn"
    );
    let lead_mitten = r#"{"type":"attachment","attachment":{"type":"queued_command","commandMode":"prompt","origin":{"kind":"peer"},"prompt":"ok"}}"#;
    assert_eq!(
        dig(&[kevin, lead, lead_mitten]).user_turn,
        Some(false),
        "the other session's message mid-turn"
    );

    // This session writes to another one itself (SendMessage; the result names "another Claude session").
    let send = r##"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"s1","name":"SendMessage","input":{"to":"Beta · Lead [97e5ab]","message":"#12 ready"}}]}}"##;
    let sent = r##"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"s1","content":"{\"success\":true}"}]},"toolUseResult":{"success":true,"message":"“#12 ready” → Beta · Lead (another Claude session on this machine; queued there)"}}"##;
    let d = dig(&[kevin, send, sent]);
    assert!(
        d.peer_name.as_deref() == Some("Beta · Lead") && d.peer_pid.is_none(),
        "written to another session: name without [id]"
    );
    let to_agent = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"s2","name":"SendMessage","input":{"to":"a1b2c304ddff2ae50","message":"go"}}]}}"#;
    let agent_ok = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"s2","content":"ok"}]},"toolUseResult":{"success":true,"message":"delivered to a1b2c304ddff2ae50"}}"#;
    assert!(
        dig(&[kevin, to_agent, agent_ok]).peer_name.is_none(),
        "a message to an own subagent"
    );
    // Kevin 2026-10-10 (screenshot 23:33): written to a socket address, the answer names no name.
    let to_sock = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"s3","name":"SendMessage","input":{"to":"uds:/tmp/cc-socks/55848.sock","message":"holding"}}]}}"#;
    let sock_ok = r##"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"s3","content":"ok"}]},"toolUseResult":{"success":true,"message":"“holding” → uds:/tmp/cc-socks/55848.sock; in that session's inbox, not yet read by its Claude"}}"##;
    let d = dig(&[kevin, send, sent, to_sock, sock_ok]);
    assert!(
        d.peer_pid == Some(55848) && d.peer_name.is_none(),
        "to a socket address: pid from the path"
    );
    let sock_fail = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"s3","content":"error"}]},"toolUseResult":{"success":false,"message":"no such socket"}}"#;
    assert!(
        dig(&[kevin, to_sock, sock_fail]).peer_pid.is_none(),
        "delivery failed"
    );
    // review quick 2026-10-10 (Codex): a denied call without toolUseResult; an answer to the same session keeps the name.
    let sock_denied = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"s3","is_error":true,"content":"Tool execution was denied"}]}}"#;
    assert!(
        dig(&[kevin, to_sock, sock_denied]).peer_pid.is_none(),
        "call denied"
    );
    let to_other = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"s4","name":"SendMessage","input":{"to":"uds:/tmp/service/33134.sock","message":"x"}}]}}"#;
    let other_ok = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"s4","content":"ok"}]},"toolUseResult":{"success":true,"message":"sent"}}"#;
    assert!(
        dig(&[kevin, to_other, other_ok]).peer_pid.is_none(),
        "a foreign socket is no session"
    );
    let from_lead = r#"{"type":"user","isMeta":true,"origin":{"kind":"peer","from":"uds:/tmp/cc-socks/55848.sock","verifiedPeerPid":55848,"name":"Lead"},"message":{"content":"go"}}"#;
    let d = dig(&[from_lead, to_sock, sock_ok]);
    assert!(
        d.peer_pid == Some(55848) && d.peer_name.as_deref() == Some("Lead"),
        "an answer to the same session: the name stays"
    );

    // Timer (ScheduleWakeup): set, stopped, fired.
    let wake = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"w1","name":"ScheduleWakeup","input":{"delaySeconds":1200}}]}}"#;
    let woke = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"w1","content":"Next wakeup scheduled for 11:57:00"}]},"toolUseResult":{"scheduledFor":1790503020000,"clampedDelaySeconds":1200,"wasClamped":false}}"#;
    assert_eq!(
        dig(&[wake, woke]).wakeups,
        BTreeMap::from([("loop".to_owned(), Some(1_790_503_020.0))]),
        "timer set"
    );
    let stop = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"w2","name":"ScheduleWakeup","input":{"stop":true}}]}}"#;
    let stopped = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"w2","content":"Loop stopped"}]},"toolUseResult":{"scheduledFor":0,"stopped":true,"cancelledWakeups":1}}"#;
    assert!(
        dig(&[wake, woke, stop, stopped]).wakeups.is_empty(),
        "timer stopped"
    );
    let fired = r#"{"type":"system","subtype":"scheduled_task_fire","taskId":"c10455b9","timestamp":"2026-09-27T09:57:00.519Z"}"#;
    assert!(dig(&[wake, woke, fired]).wakeups.is_empty(), "timer fired");
    let early = r#"{"type":"system","subtype":"scheduled_task_fire","taskId":"x1","timestamp":"2026-09-27T09:30:00.000Z"}"#;
    assert_eq!(
        dig(&[wake, woke, early]).wakeups.len(),
        1,
        "another, earlier run leaves the timer standing"
    );

    // CronCreate: one-shot (time from the expression, local time), deleted, fired; recurring stays.
    let cron = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"c1","name":"CronCreate","input":{"cron":"43 2 27 9 *","recurring":false}}]}}"#;
    let cron_ok = r#"{"type":"user","timestamp":"2026-09-26T21:03:02.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"c1","content":"Scheduled one-shot task f874851d"}]},"toolUseResult":{"id":"f874851d","humanSchedule":"43 2 27 9 *","recurring":false,"durable":false}}"#;
    let at = dig(&[cron, cron_ok])
        .wakeups
        .get("f874851d")
        .copied()
        .flatten();
    let c = at.and_then(transcript::local);
    assert!(
        c.is_some_and(|c| c.year() == 2026
            && c.month() == 9
            && c.day() == 27
            && c.hour() == 2
            && c.minute() == 43),
        "one-shot cron: 27.09. 02:43 local time, got {at:?}"
    );
    let del = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"c2","name":"CronDelete","input":{"id":"f874851d"}}]}}"#;
    let del_ok = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"c2","content":"Cancelled job f874851d."}]},"toolUseResult":{"id":"f874851d"}}"#;
    assert!(
        dig(&[cron, cron_ok, del, del_ok]).wakeups.is_empty(),
        "cron deleted"
    );
    let cron_fired = r#"{"type":"system","subtype":"scheduled_task_fire","taskId":"f874851d","timestamp":"2026-09-27T00:43:00.000Z"}"#;
    assert!(
        dig(&[cron, cron_ok, cron_fired]).wakeups.is_empty(),
        "one-shot cron fired"
    );
    let every = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"c3","name":"CronCreate","input":{"cron":"*/30 * * * *","recurring":true}}]}}"#;
    let every_ok = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"c3","content":"Scheduled recurring task 1a2b3c4d"}]},"toolUseResult":{"id":"1a2b3c4d","humanSchedule":"*/30 * * * *","recurring":true}}"#;
    let every_fired = r#"{"type":"system","subtype":"scheduled_task_fire","taskId":"1a2b3c4d","timestamp":"2026-09-27T00:30:00.000Z"}"#;
    let rec = dig(&[every, every_ok, every_fired]).wakeups;
    assert!(
        rec.len() == 1 && rec.get("1a2b3c4d") == Some(&None),
        "recurring: stays after firing, without a time"
    );

    // review quick 2026-09-27 (Kimi): a one-shot cron without a readable time does not count (else it would stand pending
    // forever after firing), a missing recurring flag means one-shot; impossible dates and a set weekday give no time
    // (Calendar silently turns 31 February into 3 March).
    let odd = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"c4","name":"CronCreate","input":{"cron":"0 9 * * 1"}}]}}"#;
    let odd_ok = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"c4","content":"Scheduled"}]},"toolUseResult":{"id":"9f9f9f9f","humanSchedule":"0 9 * * 1"}}"#;
    assert!(
        dig(&[odd, odd_ok]).wakeups.is_empty(),
        "one-shot without a readable time: does not count"
    );
    let no_flag = r#"{"type":"user","timestamp":"2026-09-26T21:03:02.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"c1","content":"Scheduled"}]},"toolUseResult":{"id":"f874851d","humanSchedule":"43 2 27 9 *"}}"#;
    let cron_no_flag = cron.replace(r#","recurring":false"#, "");
    assert!(
        dig(&[&cron_no_flag, no_flag])
            .wakeups
            .get("f874851d")
            .is_some_and(Option::is_some),
        "without a flag: one-shot with a time"
    );
    let t = 1_790_000_000.0;
    assert!(
        one_shot_cron_date("0 9 31 2 *", t).is_none(),
        "31 February is no time"
    );
    assert!(
        one_shot_cron_date("0 9 15 10 3", t).is_none(),
        "a set weekday: no simple time"
    );
    assert!(one_shot_cron_date("0 25 1 1 *", t).is_none(), "hour 25");
    assert!(
        one_shot_cron_date("30 8 15 10 *", t).is_some(),
        "a valid one-shot time"
    );
}

/// Rule without a model (frozen 2026-09-27, checked on 189 + 46 real turn ends, no hidden question): explicitly
/// "nothing needed", no question, no request to Kevin, no done message.
#[test]
fn test_nothing_needed() {
    assert!(says_nothing_needed(
        "Nothing needed from you, Kevin, and nothing is running. #12 waits for the lead's go."
    ));
    assert!(says_nothing_needed(
        "Kevin, für dich gerade nichts. Die Lead-Session meldet sich, sobald #11 gemergt ist."
    ));
    assert!(
        says_nothing_needed("NOTHING NEEDED FROM YOU. Waiting on CI."),
        "case does not matter"
    );
    assert!(
        !says_nothing_needed("Nothing needed from you. Should I also rebase?"),
        "question"
    );
    assert!(
        !says_nothing_needed("Nothing needed from you, Kevin. You can close this window now."),
        "done: close the window"
    );
    assert!(
        !says_nothing_needed("Kevin, nothing needed from you beyond the night-card pick."),
        "open choice"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you. Pull request #83 waits only for your merge on GitHub."
        ),
        "merge"
    );
    assert!(
        !says_nothing_needed("Nichts von dir nötig, erledigt."),
        "done message"
    );
    assert!(
        !says_nothing_needed("#12 waits for the lead's go."),
        "without an explicit 'nothing needed': as before"
    );

    // Sharpened 2026-10-01 (fresh turn ends 2026-09-28 to 10-01: the rule hid 15 of 26 concerns): more requests and done
    // messages block. The planned loosening of "done" failed on the same cases.
    assert!(
        !says_nothing_needed("Nothing needed from you. Waiting for your next point."),
        "waits for your next point"
    );
    assert!(
        !says_nothing_needed(
            "Nothing more needed from you except the merges: #12 first, then #13."
        ),
        "except the merges"
    );
    assert!(
        !says_nothing_needed("Nothing more needed from you after you click merge on GitHub."),
        "after your click"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you. Card #12 is closed and the lead takes over."
        ),
        "card closed"
    );
    assert!(
        !says_nothing_needed("Nothing needed from you. This window stays open until you close it."),
        "window stays open"
    );
    assert!(
        !says_nothing_needed("Für dich gerade nichts, bitte nur kurz den PR freigeben."),
        "German: bitte"
    );
    assert!(
        !says_nothing_needed("Für dich gerade nichts außer dem Merge."),
        "German: außer"
    );
    // review quick 2026-10-01 (GLM): "click" only as a word, not in names like ClickHouse.
    assert!(
        says_nothing_needed(
            "Nothing needed from you. The ClickHouse backfill runs; the lead pings me."
        ),
        "ClickHouse"
    );
    assert!(
        says_nothing_needed(
            "Nothing needed from you. The lead session pings me when the shared ports are free."
        ),
        "waits only for the other session: stays grey"
    );

    // R11 A' (Kevin 2026-10-05, measured again on 1,204 fresh turn ends): "done/finished" only counts as a real done
    // message, not as a partial step, negated, in the future or as "Done:" beside "Still to do:".
    assert!(
        says_nothing_needed(concat!(
            "Nothing needed from you, Kevin. I wait for the lead's turn line.\n\n",
            "- Part 1 of the check is done and committed."
        )),
        "partial step"
    );
    assert!(
        says_nothing_needed("Nothing needed from you. I'll pick up again when #12's run is done."),
        "future"
    );
    assert!(
        says_nothing_needed("Nothing needed from you. I resume once done with the lead's list."),
        "once done"
    );
    assert!(
        says_nothing_needed("Nichts von dir nötig. Ich warte, bis #12 erledigt ist."),
        "future, German"
    );
    assert!(
        says_nothing_needed(
            "Nothing needed from you. The card is not finished yet; the lead pings me."
        ),
        "negated"
    );
    assert!(
        says_nothing_needed("Nothing needed from you. The card isn’t done, the lead pings me."),
        "negated, typographic"
    );
    // review quick 2026-10-05 (GLM): two adverbs in the negation; a last step stays a done message.
    assert!(
        says_nothing_needed(
            "Nothing needed from you. The card is not yet quite done, the lead pings me."
        ),
        "two adverbs"
    );
    assert!(
        !says_nothing_needed("Nothing needed from you. Step 2 is done — that was the last one."),
        "last step"
    );
    assert!(
        !says_nothing_needed(
            "Nichts von dir nötig. Schritt 3 ist erledigt, das waren alle Schritte."
        ),
        "all steps"
    );
    // review quick 2026-10-05 (Codex): clause borders, empty remainder list, "please".
    assert!(
        !says_nothing_needed(
            "Nothing needed from you. After two rounds, the implementation is done."
        ),
        "after + comma"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you. Step 1 passed; the entire task is complete."
        ),
        "semicolon"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you.\nDone: PR merged, card closed.\nStill to do: nothing."
        ),
        "empty remainder list"
    );
    assert!(
        !says_nothing_needed("Nothing needed from you.\nDone: PR merged.\nNext: none."),
        "Next: none"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you on the tests. Please approve the deployment now."
        ),
        "please"
    );
    // review quick 2026-10-05 round 2 (Kimi): plural, German, "Next:" is no remainder list, a last step only in the same
    // sentence.
    assert!(
        !says_nothing_needed(
            "Kevin, nothing needed from you. #12 runs. My questions above still stand."
        ),
        "plural"
    );
    assert!(
        !says_nothing_needed(
            "Für dich gerade nichts. Meine Fragen von heute Morgen sind noch offen."
        ),
        "questions open"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you.\n- **Done:** PR merged, card closed.\n- **Next:** release tomorrow."
        ),
        "Done beside Next: done message"
    );
    assert!(
        says_nothing_needed(
            "Nothing needed from you. Part 1 of the check is done. The final part waits for the lead's go."
        ),
        "final in another sentence"
    );
    // Round 2 (GLM): a condition followed by a comma starts a possible order.
    assert!(
        !says_nothing_needed("Nothing needed from you. When the CI run is done, merge PR #12."),
        "condition + order"
    );
    assert!(
        !says_nothing_needed("Nichts von dir nötig. Sobald #12 erledigt ist, merge den PR."),
        "German"
    );
    assert!(
        says_nothing_needed(
            "Nothing needed from you.\n- **Done:** the first call.\n- **Still to do:** the tests."
        ),
        "Done beside Still to do: interim state"
    );
    assert!(
        !says_nothing_needed("Nothing needed from you. Card #98 is done: merged."),
        "real done message"
    );
    assert!(
        !says_nothing_needed("Nothing needed from you. All done."),
        "all done"
    );
    assert!(
        !says_nothing_needed("Nothing needed from you.\n- **Done:** PR merged, card closed."),
        "Done without open items"
    );
    assert!(
        !says_nothing_needed("Nothing needed from you. I'm done here."),
        "I'm done"
    );
    assert!(
        !says_nothing_needed("Nothing needed from you. When you're done testing, merge PR #12."),
        "condition on Kevin"
    );
    assert!(
        !says_nothing_needed("Nichts von dir nötig. Teil 1 ist erledigt, alles erledigt."),
        "partial step plus all done"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you. Its review closed after two rounds and the reviewer finished the tests."
        ),
        "'after' as preposition: a real done message stays"
    );
    // R11 C: an earlier question to Kevin is still open (live the watcher hid 18 of 67 requests, mostly like this).
    assert!(
        !says_nothing_needed("Kevin, nothing needed from you. #12 runs. My question above stands."),
        "question stands"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you. Still open with you: the two settings lines."
        ),
        "open with you"
    );
    assert!(
        !says_nothing_needed("Kevin, nothing needed from you beyond what my last message named."),
        "last message"
    );
    assert!(
        !says_nothing_needed(
            "Für dich gerade nichts. Die fünf Entscheidungen von oben sind weiter offen, darauf warte ich."
        ),
        "German"
    );
    assert!(
        !asks_nothing(
            "Kevin, nothing new needs you. #12 runs its browser part. My question on the plan stands.",
            false
        ),
        "watcher: question stands"
    );

    // Kevin 2026-10-10 (screenshots 22:50, two windows waited for the team lead and stood DONE): "nothing to ask" means
    // "nothing needed"; if the session says itself that it waits, progress words like "is merged" do not block.
    assert!(
        says_nothing_needed(concat!(
            "I have nothing to ask you, Kevin. Nothing of mine runs. I wait for the team lead to start ",
            "the next window.\n\n- **To know:** the two faults Codex found are still fixed in this pull request."
        )),
        "nothing to ask"
    );
    assert!(
        says_nothing_needed("Ich habe nichts zu fragen. Die Lead-Session meldet sich."),
        "nothing to ask, German"
    );
    assert!(
        !says_nothing_needed("I have nothing to ask you. Should I merge #12?"),
        "nothing to ask, but a question"
    );
    assert!(
        says_nothing_needed(concat!(
            "Nothing needed from you, Kevin. Main, with the case, is merged into my branch, and my checks ",
            "are green. Nothing runs now; I hold until card #258's signed run, about 01:00."
        )),
        "waits itself"
    );
    assert!(
        says_nothing_needed(
            "Nichts von dir nötig. #12 ist gemergt und abgeschlossen. Ich warte auf die Lead-Session."
        ),
        "waits itself, German"
    );
    assert!(
        !says_nothing_needed("Nothing needed from you. PR #12 is merged."),
        "without waiting the done message stays"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you. PR #12 is merged. I hold until you merge #13."
        ),
        "waiting for Kevin"
    );
    assert!(
        !says_nothing_needed("Nothing needed from you. PR #12 is merged. I wait for your go."),
        "waiting for Kevin's go"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you. Card #98 is done. I wait for the lead to close this window."
        ),
        "closing the window still blocks"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you. The card is handed over; I wait for the lead."
        ),
        "handover still blocks"
    );
    assert!(
        !asks_nothing("PR #12 is merged. I hold until the lead's go.", false),
        "watcher without 'nothing needed': as before"
    );
    // review quick 2026-10-10 (GLM): waiting for Kevin by "dir/dich" or first name is no own waiting; negated neither.
    assert!(
        !says_nothing_needed(
            "Nichts von dir nötig. Der Bericht ist abgeschlossen. Ich warte auf Rückmeldung von dir."
        ),
        "waits for you (dir)"
    );
    set_greeting_name("Will");
    assert!(
        !says_nothing_needed(
            "Nothing needed from you. The audit is complete. I hold until Will reviews the PR."
        ),
        "first name"
    );
    assert!(
        says_nothing_needed(
            "Nothing needed from you. The audit is complete. I hold until Willow's run."
        ),
        "whole word only"
    );
    assert!(
        !says_nothing_needed("Nichts von dir nötig. Ich warte nicht mehr, alles erledigt."),
        "negated, German"
    );
    assert!(
        !says_nothing_needed("Nothing needed from you. All done; I wait no longer."),
        "negated, English"
    );
    // review quick 2026-10-10 (Codex): a sentence across the line break, a request to Kevin in another sentence.
    assert!(
        !says_nothing_needed(
            "Nichts von dir nötig. Die Aufgabe ist abgeschlossen. Ich warte nicht mehr auf die Lead-Session."
        ),
        "negated, with a target"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you on testing. PR #12 is merged. I hold until\nyou approve the release."
        ),
        "line break in the waiting sentence"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you. I wait for the lead. When you're done testing, merge PR #12."
        ),
        "condition on Kevin in another sentence"
    );
    // review quick 2026-10-10 round 2 (GLM): a clause after the done word, the first name in the done sentence,
    // possession, "for nothing".
    assert!(
        !says_nothing_needed(concat!(
            "Nothing needed from you. I wait for the lead's green light. The summary is finished — reply ",
            "with any corrections by noon."
        )),
        "order after a dash"
    );
    assert!(
        !says_nothing_needed(
            "Nothing more to ask. I wait for the nightly build. Card #9 is finished — approval whenever suits."
        ),
        "nothing to ask, order after a dash"
    );
    assert!(
        !says_nothing_needed(concat!(
            "Nothing needed from you. I wait for the timer. I hold until Will reviews the PR; the review ",
            "is done after that."
        )),
        "first name in the done sentence"
    );
    assert!(
        says_nothing_needed(concat!(
            "Nothing needed from you. I wait for the lead's line.\n\n- **Done:** all five updates are ",
            "committed.\n- The review loop is closed: four rounds."
        )),
        "colon: a list, no order"
    );
    // review quick 2026-10-10 round 2 (Codex).
    assert!(
        !says_nothing_needed(
            "Nothing needed from you. The card is closed out. I wait for the team lead."
        ),
        "closed out"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you on testing. I wait for the lead. When the CI run is done, merge PR #12."
        ),
        "condition + order stays"
    );
    assert!(
        !says_nothing_needed(concat!(
            "Nothing needed from you on testing. PR #12 is merged. I wait for the team lead. I hold until ",
            "you approve the release."
        )),
        "a second waiting sentence on Kevin blocks"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you on testing. PR #12 is merged. I hold until README.md is approved by you."
        ),
        "dot in a file name"
    );
    assert!(
        !says_nothing_needed(concat!(
            "Nichts von dir nötig. Die Aufgabe ist abgeschlossen. Ich warte jetzt nicht mehr auf die ",
            "Lead-Session."
        )),
        "filler word before the negation"
    );
    assert!(
        !says_nothing_needed("I have nothing to ask the lead. The release waits."),
        "nothing to ask a third party"
    );
    assert!(
        says_nothing_needed("Nothing needed from you. PR #12 is merged. I will wait for the lead."),
        "'will' is not Will"
    );
    // Check round (GLM): a third party after the line break; "no question" does not open the first sentence for the
    // watcher.
    assert!(
        !says_nothing_needed("I have nothing more to ask\nthe team lead; the release waits."),
        "third party after a line break"
    );
    assert!(
        !opens_with_nothing_needed("I have nothing to ask you. Please approve the deployment."),
        "no question != no request"
    );
    assert!(
        !says_nothing_needed("I have nothing to ask you. Please approve the deployment."),
        "request in the whole text"
    );

    // Kevin 2026-10-10 (screenshot 23:33): if the session waits itself and the first sentence says "nothing needed",
    // requests in the rest do not count (a report on another window); "?", an open question, closing still block.
    let rules_page = concat!(
        "Nothing needed from you, Kevin. Nothing runs now. I hold until the team lead's go for the test step ",
        "tomorrow.\n\n- **To know:** my turn comes after the rule editor's. The editor waits on your answer to its spec ",
        "question in the morning."
    );
    assert!(
        !says_nothing_needed(rules_page) && waits_with_nothing_needed(rules_page),
        "another window waits for Kevin"
    );
    assert!(
        !waits_with_nothing_needed("Nothing needed from you. The editor waits on your answer."),
        "without waiting itself"
    );
    assert!(
        !waits_with_nothing_needed(
            "Nothing needed from you. I hold until the lead's go. Should I merge #12?"
        ),
        "question mark"
    );
    assert!(
        !waits_with_nothing_needed(
            "Nothing needed from you. I wait for the lead. My question above stands."
        ),
        "open question"
    );
    assert!(
        !waits_with_nothing_needed(
            "Nothing needed from you. I wait for the lead. You can close this window."
        ),
        "close"
    );
    assert!(
        !waits_with_nothing_needed(
            "I have nothing to ask you. I wait for the lead. Please approve the release."
        ),
        "no question is no first sentence"
    );
    assert!(
        !waits_with_nothing_needed(
            "Nothing needed from you. I wait for the lead. Please approve the deployment now."
        ),
        "please still blocks (review quick, GLM)"
    );
    assert!(
        !waits_with_nothing_needed(
            "Nichts von dir nötig. Ich warte auf die Lead-Session. Bitte gib den PR frei."
        ),
        "bitte"
    );
    assert!(
        !waits_with_nothing_needed(
            "Nothing needed from you. I wait for the lead. The report for you is complete."
        ),
        "done message to Kevin (review quick, Codex)"
    );
    assert!(
        !waits_with_nothing_needed(concat!(
            "Nothing needed from you on testing. I wait for the lead. I need your approval for the ",
            "deployment now."
        )),
        "I need your"
    );
    assert!(
        !waits_with_nothing_needed(concat!(
            "Nothing needed from you. I wait for the lead. The final report is complete. It is ready ",
            "for you to review."
        )),
        "ready for you"
    );
    assert!(
        !waits_with_nothing_needed(concat!(
            "Nothing needed from you. I hold until you approve the release.\n- The editor waits on ",
            "your answer."
        )),
        "waiting for Kevin is no waiting itself"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you. The four tickets I hold are resolved, the queue is finished."
        ),
        "possession"
    );
    assert!(
        !says_nothing_needed(
            "Nothing needed from you. The queue is finished. I'm waiting for nothing else tonight."
        ),
        "waiting for nothing"
    );
    assert!(
        !says_nothing_needed(
            "Nichts von dir nötig. Die Liste ist abgeschlossen. Ich warte auf nichts mehr."
        ),
        "auf nichts"
    );
    assert!(
        says_nothing_needed(
            "Nothing needed from you. PR #12 is merged. I hold my report until the lead's check arrives."
        ),
        "hold … until"
    );
    set_greeting_name("Kevin");
}

#[derive(Clone)]
struct Worker {
    ready: bool,
    waiting: bool,
    text: &'static str,
    ended: bool,
    peer_pid: Option<Pid>,
    peer_name: Option<&'static str>,
    wakeups: BTreeMap<String, Option<Time>>,
    question: Option<&'static str>,
    kevin: bool,
    answer_at: Option<Time>,
}

impl Default for Worker {
    fn default() -> Worker {
        Worker {
            ready: true,
            waiting: false,
            text: "Nothing needed from you. I wait for the lead's go.",
            ended: true,
            peer_pid: Some(33134),
            peer_name: Some("Beta · Lead"),
            wakeups: BTreeMap::new(),
            question: None,
            kevin: false,
            answer_at: None,
        }
    }
}

fn worker(id: &str, w: Worker) -> Session {
    Session {
        waiting: w.waiting,
        digest: Some(Digest {
            text: Some(w.text.into()),
            last: LastBlock::Text,
            answer_at: w.answer_at,
            question: w.question.map(str::to_owned),
            turn_ended: w.ended,
            user_turn: Some(w.kevin),
            peer_pid: w.peer_pid,
            peer_name: w.peer_name.map(str::to_owned),
            wakeups: w.wakeups,
            ..Digest::default()
        }),
        ..Session::new(
            format!("claude:{id}"),
            Engine::Claude,
            Some(88855),
            "Beta · #12 Theme check",
            "/x",
            w.ready,
            true,
        )
    }
}

#[test]
fn test_standby() {
    let now = 1_790_503_000.0;
    let w = Worker::default;
    let lead = Session::new(
        "claude:lead",
        Engine::Claude,
        Some(33134),
        "Beta · Lead",
        "/u",
        false,
        true,
    );
    let none = BTreeMap::new();
    let (log, report) = recorder();
    let mut standby = Standby::new(vec![], BTreeSet::new(), report);
    let out = standby.adjust(&[worker("a", w()), lead.clone()], now, &none);
    assert!(
        out[0].watching && !out[0].ready,
        "waits for a living other session and says 'nothing needed': watching"
    );
    assert!(
        !out[1].watching && !out[1].ready,
        "the other session itself stays as reported"
    );
    assert_eq!(
        said(&log).len(),
        1,
        "one log line per change, got {:?}",
        said(&log)
    );
    let out = standby.adjust(&[worker("a", w()), lead.clone()], now, &none);
    assert!(
        out[0].watching && said(&log).len() == 1,
        "same state: no new line"
    );

    let mut s = Standby::new(vec![], BTreeSet::new(), silent());
    let mut first = |sessions: &[Session], seen: &BTreeMap<String, Time>| {
        s.adjust(sessions, now, seen)[0].clone()
    };
    assert!(
        first(
            &[worker(
                "b",
                Worker {
                    peer_pid: Some(4242),
                    peer_name: None,
                    ..w()
                }
            )],
            &none
        )
        .ready,
        "other session ended: done"
    );
    // review quick 2026-09-27 (GLM): pid reused; only a running Claude session counts, not any process.
    assert!(
        first(
            &[worker(
                "b2",
                Worker {
                    peer_name: None,
                    ..w()
                }
            )],
            &none
        )
        .ready,
        "pid without a Claude session to it: done"
    );
    assert!(
        first(
            &[
                worker(
                    "c",
                    Worker {
                        peer_pid: None,
                        ..w()
                    }
                ),
                lead.clone()
            ],
            &none
        )
        .watching,
        "only the name (SendMessage), the session runs"
    );
    // review quick 2026-10-10 (Codex): a session that started only after the message carries just the old pid.
    let mut reborn = worker(
        "c2",
        Worker {
            peer_name: None,
            ..w()
        },
    );
    reborn.digest.as_mut().unwrap().peer_at = Some(now - 3600.0);
    let newcomer = Session {
        started_at: Some(now - 600.0),
        ..Session::new(
            "claude:new",
            Engine::Claude,
            Some(33134),
            "Gamma · Other",
            "/v",
            false,
            true,
        )
    };
    assert!(
        first(&[reborn, newcomer.clone()], &none).ready,
        "reused pid: done"
    );
    let mut same = worker(
        "c3",
        Worker {
            peer_name: None,
            ..w()
        },
    );
    same.digest.as_mut().unwrap().peer_at = Some(now - 60.0);
    assert!(
        first(&[same, newcomer], &none).watching,
        "pid started before the message: the same session"
    );
    assert!(
        first(
            &[worker(
                "d",
                Worker {
                    peer_pid: None,
                    ..w()
                }
            )],
            &none
        )
        .ready,
        "only the name, no such session: done"
    );
    let wakeups = |at: Option<Time>, key: &str| BTreeMap::from([(key.to_owned(), at)]);
    let no_peer = |wk: BTreeMap<String, Option<Time>>| Worker {
        peer_pid: None,
        peer_name: None,
        wakeups: wk,
        ..w()
    };
    assert!(
        first(
            &[worker("e", no_peer(wakeups(Some(now + 600.0), "loop")))],
            &none
        )
        .watching,
        "timer pending"
    );
    assert!(
        first(
            &[worker("f", no_peer(wakeups(Some(now - 400.0), "loop")))],
            &none
        )
        .ready,
        "timer long past"
    );
    assert!(
        first(&[worker("g", no_peer(wakeups(None, "x")))], &none).watching,
        "recurring"
    );
    assert!(
        first(&[worker("h", no_peer(BTreeMap::new()))], &none).ready,
        "without a wake source: done as before"
    );

    assert!(
        first(
            &[
                worker(
                    "i",
                    Worker {
                        text: "Nothing needed from you. Should I merge?",
                        ..w()
                    }
                ),
                lead.clone()
            ],
            &none
        )
        .ready,
        "question: done"
    );
    assert!(
        first(
            &[
                worker(
                    "j",
                    Worker {
                        text: "Done. You can close this window.",
                        ..w()
                    }
                ),
                lead.clone()
            ],
            &none
        )
        .ready,
        "done stays done"
    );
    let asks = first(
        &[
            worker(
                "k",
                Worker {
                    waiting: true,
                    ..w()
                },
            ),
            lead.clone(),
        ],
        &none,
    );
    assert!(
        asks.waiting && !asks.watching,
        "an open question/approval stays 'waiting for you'"
    );
    assert!(
        !first(
            &[
                worker(
                    "l",
                    Worker {
                        question: Some("Welche Farbe?"),
                        ..w()
                    }
                ),
                lead.clone()
            ],
            &none
        )
        .watching,
        "open AskUserQuestion"
    );
    let busy = first(
        &[
            worker(
                "m",
                Worker {
                    ready: false,
                    ..w()
                },
            ),
            lead.clone(),
        ],
        &none,
    );
    assert!(!busy.watching && !busy.ready, "works: stays");
    assert!(
        first(
            &[
                worker(
                    "n",
                    Worker {
                        ended: false,
                        ..w()
                    }
                ),
                lead.clone()
            ],
            &none
        )
        .ready,
        "turn cancelled (no turn end): done"
    );
    // Kevin 2026-10-01: if his own message started the turn, it is his turn; never grey (fresh cases 09-28 to 10-01).
    assert!(
        first(
            &[worker("o", Worker { kevin: true, ..w() }), lead.clone()],
            &none
        )
        .ready,
        "answer to Kevin's message: done"
    );
    // R11 B1 (Kevin 2026-10-05, screenshot 13:11): once Kevin had the answer to his message in front of him, it no longer
    // counts; grey only with an explicit "nothing needed" (B1 strict, Kevin's choice over the loose B2).
    let answered = now - 60.0;
    let seen = |id: &str, t: Time| BTreeMap::from([(format!("claude:{id}"), t)]);
    let kevin_answered = || Worker {
        kevin: true,
        answer_at: Some(answered),
        ..w()
    };
    assert!(
        first(
            &[worker("p", kevin_answered()), lead.clone()],
            &seen("p", answered + 5.0)
        )
        .watching,
        "answer seen: watching"
    );
    assert!(
        first(
            &[worker("q", kevin_answered()), lead.clone()],
            &seen("q", answered - 5.0)
        )
        .ready,
        "only in front before the answer: done"
    );
    assert!(
        first(
            &[worker("q2", Worker { kevin: true, ..w() }), lead.clone()],
            &seen("q2", now)
        )
        .ready,
        "without the answer's time: not seen"
    );
    let keep_open = "Please keep this window open, Kevin. The card is not finished. The window waits for its turn.";
    assert!(
        first(
            &[
                worker(
                    "r",
                    Worker {
                        text: keep_open,
                        ..kevin_answered()
                    }
                ),
                lead.clone()
            ],
            &seen("r", now)
        )
        .ready,
        "seen, but without 'nothing needed': done (B1 strict)"
    );
    assert!(
        first(&[worker("r2", kevin_answered())], &seen("r2", now)).ready,
        "seen, the other session no longer runs: done"
    );
    let (seen_log, report) = recorder();
    Standby::new(vec![], BTreeSet::new(), report).adjust(
        &[worker("p", kevin_answered()), lead.clone()],
        now,
        &seen("p", now),
    );
    assert!(
        said(&seen_log)
            .first()
            .is_some_and(|l| l.contains("Antwort gesehen")),
        "the log names the reason, got {:?}",
        said(&seen_log)
    );

    // Back to done: one line.
    let (log, report) = recorder();
    let mut fresh = Standby::new(vec![], BTreeSet::new(), report);
    fresh.adjust(&[worker("a", w()), lead.clone()], now, &none);
    fresh.adjust(
        &[
            worker(
                "a",
                Worker {
                    peer_pid: None,
                    peer_name: None,
                    ..w()
                },
            ),
            lead,
        ],
        now,
        &none,
    );
    assert_eq!(
        said(&log).len(),
        2,
        "there and back: one line each, got {:?}",
        said(&log)
    );
}

/// Kevin 2026-11-10: after the turn end nothing came in, so a `busy` from Claude only means background agents run.
#[test]
fn test_quiet_since_end() {
    let end = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"? Lookout: Bitte (x)"}]}}"#,
        "\n",
        r#"{"type":"system","subtype":"turn_duration"}"#
    );
    let d = digest_transcript(end);
    assert!(
        d.turn_ended && d.quiet_since_end,
        "nothing after the turn end"
    );
    for (after, why) in [
        (
            r#"{"type":"user","isMeta":true,"origin":{"kind":"peer","from":"uds:/tmp/cc-socks/1.sock"},"message":{"content":"hi"}}"#,
            "another session's message",
        ),
        (
            r#"{"type":"user","origin":{"kind":"task-notification"},"message":{"content":"<task-notification>"}}"#,
            "a background notice",
        ),
        (
            r#"{"type":"queue-operation","operation":"enqueue"}"#,
            "a queued message",
        ),
        (
            r#"{"type":"attachment","attachment":{"type":"queued_command","commandMode":"prompt","prompt":"x"}}"#,
            "a message mid-turn",
        ),
        (
            r#"{"type":"system","subtype":"scheduled_task_fire","taskId":"c1"}"#,
            "an own timer fires",
        ),
    ] {
        assert!(
            !digest_transcript(&format!("{end}\n{after}")).quiet_since_end,
            "{why}"
        );
    }
    let hook = r#"{"type":"attachment","attachment":{"type":"hook_success"}}"#;
    assert!(
        digest_transcript(&format!("{end}\n{hook}")).quiet_since_end,
        "a hook note is no new input"
    );
    // Esc ends the turn with an interrupt line, a prompt without origin: the "?" before it no longer counts, so nothing
    // is left to lift (review quick 2026-11-10, Grok; order as recorded live 2026-10-15).
    let esc = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"? Lookout: Bitte (x)"}]}}"#,
        "\n",
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]}}"#,
        "\n",
        r#"{"type":"user","message":{"content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#,
        "\n",
        r#"{"type":"system","subtype":"turn_duration"}"#
    );
    let d = digest_transcript(esc);
    assert!(
        d.turn_ended && d.quiet_since_end && d.line(&[]).is_none(),
        "Esc: no line left to lift"
    );
}

/// Kevin 2026-10-12: the session says at its turn end itself whether it needs Kevin ("◌ / ? / ✓ Lookout: …"); that goes
/// before the word list; without the line everything stays as before.
#[test]
fn test_lookout_line() {
    let line_of = |status: LineStatus, reason: &str| {
        Some(LookoutLine {
            status,
            reason: reason.into(),
        })
    };
    let status = |text: &str| lookout_line(text).map(|l| l.status);
    let reason = |text: &str| lookout_line(text).map(|l| l.reason);
    // Reading
    assert_eq!(
        lookout_line("Erledigt.\n\n✓ Lookout: fertig (Karte #121)"),
        line_of(LineStatus::Done, "Karte #121")
    );
    assert_eq!(
        lookout_line("x\n? Lookout: Bitte (Merge #126)\n\n"),
        line_of(LineStatus::Ask, "Merge #126"),
        "empty lines after it"
    );
    assert_eq!(
        status("◌ Lookout: braucht nichts (wartet auf CI)"),
        Some(LineStatus::Nothing)
    );
    assert_eq!(
        reason("**? Lookout: Bitte (Passwort eingeben)**").as_deref(),
        Some("Passwort eingeben"),
        "markdown around it"
    );
    assert_eq!(
        status("`✓ Lookout: done (shipped)`"),
        Some(LineStatus::Done),
        "backticks, English"
    );
    assert_eq!(
        status("\u{2713}\u{FE0E} Lookout: fertig"),
        Some(LineStatus::Done),
        "check mark with text variation selector"
    );
    assert_eq!(
        reason("? Lookout: Bitte Freigabe für den Merge von PR #42 geben").as_deref(),
        Some("Freigabe für den Merge von PR #42 geben"),
        "without brackets: the text without status word (sample 2026-10-12)"
    );
    assert_eq!(
        reason("✓ Lookout: fertig").as_deref(),
        Some("fertig"),
        "only the status word: it stays"
    );
    assert_eq!(
        reason("✓ Lookout: fertig ()").as_deref(),
        Some("fertig"),
        "empty brackets"
    );
    assert_eq!(
        reason("? Lookout: Bitte (a) oder (b)").as_deref(),
        Some("(a) oder (b)"),
        "two brackets (review quick, GLM)"
    );
    assert_eq!(
        reason("? Lookout: Bitte (Merge #126 (PR-3))").as_deref(),
        Some("Merge #126 (PR-3)"),
        "a bracket in the bracket"
    );
    assert_eq!(
        reason("? Lookout: (Passwort)").as_deref(),
        Some("Passwort"),
        "without status word"
    );
    assert!(
        lookout_line("◌ Lookout:").is_none() && lookout_line("? Lookout:   ").is_none(),
        "empty: no line (review quick, Muse)"
    );
    assert_eq!(
        reason("? Lookout: Bitte (Merge #126).").as_deref(),
        Some("Merge #126"),
        "a dot at the end (review quick, Grok)"
    );
    assert!(
        lookout_line("? Lookout: Bitte (Merge #126)\nNoch ein Satz danach.").is_none(),
        "only the last line counts"
    );
    assert!(
        lookout_line("Soll ich mergen?").is_none() && lookout_line("").is_none(),
        "without a line: nothing"
    );
    assert!(
        lookout_line("? Lookouts: x").is_none() && lookout_line("! Lookout: x").is_none(),
        "only the three marks"
    );

    // A request for another window (`other_window_names`, Kevin 2026-10-13: 32 of 291 "?" sent him to another window
    // and brought this one forward): counts as "needs nothing", the reason stays for the label.
    let front_desk = ["front desk".to_owned()];
    let line = |names: &[String], text: &str, name: Option<&str>| {
        Digest {
            name: name.map(str::to_owned),
            text: Some(text.into()),
            ..Digest::default()
        }
        .line(names)
    };
    let ask = |text: &str, name: Option<&str>| line(&front_desk, text, name).map(|l| l.status);
    assert_eq!(
        line(
            &front_desk,
            "? Lookout: request (approve the plan in the Front Desk)",
            None
        ),
        line_of(LineStatus::Nothing, "approve the plan in the Front Desk"),
        "another window"
    );
    assert_eq!(
        ask(
            "? Lookout: request (approve the plan, here or in the front desk)",
            None
        ),
        Some(LineStatus::Ask),
        "also here"
    );
    assert_eq!(
        ask(
            "? Lookout: Bitte (hier freigeben, Info an Front Desk)",
            None
        ),
        Some(LineStatus::Ask),
        "hier"
    );
    assert_eq!(
        ask(
            "? Lookout: request (type it in this window; the front desk knows)",
            None
        ),
        Some(LineStatus::Ask),
        "this window"
    );
    assert_eq!(
        ask(
            "? Lookout: request (approve the plan in the front desk)",
            Some("Beta · Front desk")
        ),
        Some(LineStatus::Ask),
        "the named window itself"
    );
    assert_eq!(
        ask("? Lookout: request (merge #12)", None),
        Some(LineStatus::Ask),
        "without a name"
    );
    assert_eq!(
        ask("✓ Lookout: done (the front desk has it)", None),
        Some(LineStatus::Done),
        "only ? is reinterpreted"
    );
    assert_eq!(
        line(
            &[],
            "? Lookout: request (approve the plan in the front desk)",
            None
        )
        .map(|l| l.status),
        Some(LineStatus::Ask),
        "without configuration"
    );

    // Transcript: the line holds only for the turn it stands in.
    let entry = |v: Value| v.to_string();
    let answer = entry(
        json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "Fertig.\n✓ Lookout: fertig (X)"}]}}),
    );
    let ended = entry(json!({"type": "system", "subtype": "turn_duration"}));
    let prompt = entry(json!({"type": "user", "message": {"content": "weiter"}}));
    let fire = entry(json!({"type": "system", "subtype": "scheduled_task_fire"}));
    let digest_line = |l: &[&str]| digest_transcript(&lines(l)).line(&[]);
    assert_eq!(
        digest_line(&[&answer, &ended]).map(|l| l.reason).as_deref(),
        Some("X")
    );
    assert!(
        digest_line(&[&answer, &ended, &prompt]).is_none(),
        "a new message: the old line is gone"
    );
    // After another session or a timer without new text the last self-report stays (review quick, Grok).
    let peer_msg =
        entry(json!({"type": "user", "message": {"content": "go"}, "origin": {"kind": "peer"}}));
    assert!(
        digest_line(&[&answer, &ended, &fire]).is_some(),
        "timer: the line stays"
    );
    assert!(
        digest_line(&[&answer, &ended, &peer_msg, &ended]).is_some(),
        "another session: stays"
    );
    let talk = [
        entry(
            json!({"type": "user", "message": {"content": "q"}, "origin": {"kind": "peer", "from": "uds:/tmp/cc-socks/1.sock", "name": "Beta · A"}}),
        ),
        entry(
            json!({"type": "user", "message": {"content": "q"}, "origin": {"kind": "peer", "from": "uds:/tmp/cc-socks/2.sock", "name": "Beta · B"}}),
        ),
    ];
    assert_eq!(
        digest_transcript(&talk.join("\n")).partners,
        BTreeSet::from(["Beta · A".to_owned(), "Beta · B".to_owned()]),
        "all partners"
    );
    let mid_turn = entry(
        json!({"type": "attachment", "attachment": {"commandMode": "prompt", "origin": {"kind": "human"}}}),
    );
    assert!(
        digest_line(&[&answer, &mid_turn]).is_none(),
        "Kevin's message mid-turn: the old line is gone (review quick, Codex)"
    );
    assert!(
        digest_line(&[&answer, &ended, &prompt, &answer]).is_some(),
        "a new answer: a new line"
    );

    // Another session or a timer (`Standby`)
    let now = 1_790_503_000.0;
    let session =
        |text: &str, ready: bool, background: bool, ended: bool, kevin: bool, peer: bool| Session {
            background_only: background,
            digest: Some(Digest {
                text: Some(text.into()),
                last: LastBlock::Text,
                turn_ended: ended,
                user_turn: Some(kevin),
                peer_pid: peer.then_some(33134),
                ..Digest::default()
            }),
            ..Session::new(
                "claude:w",
                Engine::Claude,
                Some(88855),
                "Beta · Worker",
                "/x",
                ready,
                true,
            )
        };
    let plain = |text: &str| session(text, true, false, true, false, true);
    let lead = Session::new(
        "claude:lead",
        Engine::Claude,
        Some(33134),
        "Beta · Lead",
        "/u",
        false,
        true,
    );
    let none = BTreeMap::new();
    let (log, report) = recorder();
    let mut s = Standby::new(vec![], BTreeSet::new(), report);
    let grau = s.adjust(
        &[
            session(
                "Merged. Soll ich?\n◌ Lookout: braucht nichts (Lead prüft)",
                true,
                false,
                true,
                true,
                true,
            ),
            lead.clone(),
        ],
        now,
        &none,
    );
    assert!(
        grau[0].watching && !grau[0].ready,
        "'needs nothing' beats a question mark and Kevin's message"
    );
    assert!(
        said(&log)
            .first()
            .is_some_and(|l| l.contains("(Lookout-Zeile)")),
        "the log names the reason, got {:?}",
        said(&log)
    );
    assert!(
        s.adjust(
            &[
                plain(
                    "Nothing needed from you. I wait for the lead.\n✓ Lookout: fertig (Karte #12)"
                ),
                lead.clone()
            ],
            now,
            &none
        )[0]
        .ready,
        "'done' beats 'nothing needed'"
    );
    assert!(
        s.adjust(
            &[session(
                "◌ Lookout: braucht nichts (CI)",
                true,
                false,
                true,
                false,
                false
            )],
            now,
            &none
        )[0]
        .ready,
        "without a wake source: done, nothing wakes it by itself"
    );
    // Kevin 2026-10-12 (screenshot 16:08, "Key account"): the last partner ended, another one runs.
    let mut hub = session(
        "◌ Lookout: braucht nichts (naechste Frage eines Workers)",
        true,
        false,
        true,
        false,
        false,
    );
    hub.digest.as_mut().unwrap().partners =
        BTreeSet::from(["Beta · #294 beendet".to_owned(), "Beta · Lead".to_owned()]);
    assert!(
        s.adjust(&[hub.clone(), lead.clone()], now, &none)[0].watching,
        "collecting session: any running partner wakes it"
    );
    assert!(
        s.adjust(&[hub], now, &none)[0].ready,
        "no partner runs any more: done"
    );
    let mut wort_hub = session(
        "Nothing needed from you. I wait.",
        true,
        false,
        true,
        false,
        false,
    );
    wort_hub.digest.as_mut().unwrap().partners = BTreeSet::from(["Beta · Lead".to_owned()]);
    assert!(
        s.adjust(&[wort_hub, lead.clone()], now, &none)[0].ready,
        "without a Lookout line it stays with the last partner (measured rule)"
    );
    assert!(
        s.adjust(
            &[
                plain("Nothing needed from you. I wait for the lead's go."),
                lead.clone()
            ],
            now,
            &none
        )[0]
        .watching,
        "without a line: word list as before"
    );

    // Own watcher (`BackgroundWatch`)
    let rows = background_fixture();
    let t0 = 60_000.0;
    let mut watch = watch_with(
        fixed(Some(BTreeMap::new())),
        fixed(Some(pids(&[542]))),
        &[],
        silent(),
    );
    let bg = |text: &str, kevin: bool| background_session("claude:q", "lookout-q", text, kevin);
    let leise = bg(
        "Soll ich danach mergen? Erst CI.\n◌ Lookout: braucht nichts (CI läuft)",
        true,
    );
    for dt in [0.0, 96.0] {
        watch.adjust(std::slice::from_ref(&leise), &rows, t0 + dt, 90.0, &none);
    }
    assert!(
        watch.adjust(&[leise], &rows, t0 + 99.0, 90.0, &none)[0].watching,
        "watcher + 'needs nothing': watching"
    );
    assert!(
        watch.adjust(
            &[bg(
                "Waiting for CI. Nothing needed from you.\n✓ Lookout: fertig (PR offen)",
                false
            )],
            &rows,
            t0 + 102.0,
            90.0,
            &none
        )[0]
        .ready,
        "'done' beats 'nothing needed'"
    );

    // Request: waits for you
    let applied = |s: Session| apply_lookout_lines(vec![s], &[]).remove(0);
    let bitte = applied(plain("Bereit.\n? Lookout: Bitte (Merge #126)"));
    assert!(
        bitte.waiting && bitte.ready && !bitte.watching,
        "request: waits for you"
    );
    assert!(
        applied(session(
            "? Lookout: Bitte (Test)",
            false,
            true,
            true,
            false,
            true
        ))
        .waiting,
        "also when only a background command still runs"
    );
    let mut grau_bitte = session("? Lookout: Bitte (Test)", false, false, true, false, true);
    grau_bitte.watching = true;
    let vorn = applied(grau_bitte);
    assert!(
        vorn.waiting && !vorn.watching,
        "watching becomes waits for you"
    );
    let arbeitet = applied(session(
        "? Lookout: Bitte (x)",
        false,
        false,
        true,
        false,
        true,
    ));
    assert!(
        !arbeitet.waiting && !arbeitet.ready,
        "busy: the model works"
    );
    // Busy with nothing new since the turn end: only background agents run (Kevin 2026-11-10).
    let quiet = |text: &str| {
        let mut s = session(text, false, false, true, false, true);
        s.digest.as_mut().unwrap().quiet_since_end = true;
        s
    };
    let nur_helfer = applied(quiet("? Lookout: Bitte (x)"));
    assert!(
        nur_helfer.waiting && nur_helfer.ready,
        "busy only through background agents: waits for you"
    );
    assert!(
        !applied(quiet("✓ Lookout: fertig (x)")).ready,
        "only a request is lifted, done stays working while agents run"
    );
    let mut ohne_fenster = quiet("? Lookout: Bitte (x)");
    ohne_fenster.pid = None;
    assert!(
        !applied(ohne_fenster).waiting,
        "a background agent without a window stays as reported"
    );
    assert!(
        !applied(session(
            "? Lookout: Bitte (x)",
            true,
            false,
            false,
            false,
            true
        ))
        .waiting,
        "not without a turn end"
    );
    assert!(
        !applied(plain("✓ Lookout: fertig (x)")).waiting,
        "done stays done"
    );
    let mut alt = plain("? Lookout: Bitte (x)");
    alt.digest.as_mut().unwrap().text_current = false;
    assert!(!applied(alt).waiting, "a line from an earlier turn");
    let codex = Session::new("codex:c", Engine::Codex, Some(1), "c", "", true, true);
    assert!(!applied(codex).waiting, "Claude only");

    // Label: the line says what it is about.
    let label = |s: Session, title: &str| {
        let line = s.digest.as_ref().and_then(|d| d.line(&[]));
        crate::overlay::label_content(
            &crate::testutil::tab_window(vec![s], title),
            None,
            line.as_ref(),
            now,
        )
        .detail
    };
    let mut f = Session::new(
        "claude:l",
        Engine::Claude,
        Some(2),
        "alpha-l",
        "/Users/test/Projects/Alpha",
        true,
        true,
    );
    f.digest = Some(Digest {
        title: Some("Rechnungen".into()),
        text: Some("Kevin, alles gebucht. Mehr folgt.\n✓ Lookout: fertig (12 Rechnungen)".into()),
        last: LastBlock::Text,
        ..Digest::default()
    });
    assert_eq!(label(f, "✳ Rechnungen").as_deref(), Some("12 Rechnungen"));
    let mut w = applied(plain("Bereit.\n? Lookout: Bitte (Merge #126)"));
    if let Some(d) = w.digest.as_mut() {
        d.step = Some("deploy".into());
    }
    assert_eq!(
        label(w, "✳ Shop").as_deref(),
        Some("Merge #126"),
        "the request instead of 'Freigabe: <step>'"
    );
}

// ---------- Phase 2: teilbar (language, settings, greeting) ----------

/// Code point 3: `claude agents` for the default folder without CLAUDE_CONFIG_DIR, otherwise claude creates a second
/// configuration file there. Removed actively: Lookout inherits its starter's environment.
#[test]
fn test_agents_environment() {
    let home = std::env::var("HOME").unwrap_or_default();
    let (set, unset) = agents_environment(&format!("{home}/.claude"), &home);
    assert!(
        set.is_empty() && unset == ["CLAUDE_CONFIG_DIR"],
        "default folder: remove the variable"
    );
    let (set, unset) = agents_environment(&format!("{home}/.claude-work"), &home);
    assert!(
        set == BTreeMap::from([(
            "CLAUDE_CONFIG_DIR".to_owned(),
            format!("{home}/.claude-work")
        )]) && unset.is_empty(),
        "second folder: set it"
    );
    // review quick R2 (2026-10-01): a link (symlink) to the default folder is the default folder.
    let fake_home = TempDir::new();
    let fh = &fake_home.0;
    fs::create_dir_all(format!("{fh}/.claude")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(format!("{fh}/.claude"), format!("{fh}/alias")).unwrap();
    #[cfg(unix)]
    assert_eq!(
        agents_environment(&format!("{fh}/alias"), fh).1,
        ["CLAUDE_CONFIG_DIR"],
        "a link to ~/.claude"
    );
    assert_eq!(
        agents_environment(&format!("{fh}/.claude"), fh).1,
        ["CLAUDE_CONFIG_DIR"]
    );
    assert_eq!(
        agents_environment(&format!("{fh}/anders"), fh).0,
        BTreeMap::from([("CLAUDE_CONFIG_DIR".to_owned(), format!("{fh}/anders"))])
    );
    // Swift sets a variable in its own process first; `set_var` is unsafe in Rust 2024, so PATH is the inherited one.
    let env = |set: &[(&str, &str)], unset: &[&str]| {
        let set: BTreeMap<String, String> = set
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let unset: Vec<String> = unset.iter().map(|s| s.to_string()).collect();
        let out = run_command("/usr/bin/env", &[], &set, &unset, 5.0, &[0], &mut |_| {});
        String::from_utf8(out.unwrap_or_default()).unwrap()
    };
    let path = std::env::var("PATH").unwrap();
    assert!(
        env(&[], &[]).lines().any(|l| l == format!("PATH={path}")),
        "inherited without anything given"
    );
    assert!(
        !env(&[], &["PATH"]).lines().any(|l| l.starts_with("PATH=")),
        "removed actively, not just left out"
    );
    assert!(
        env(&[("LOOKOUT_TEST_VAR", "gesetzt")], &[])
            .lines()
            .any(|l| l == "LOOKOUT_TEST_VAR=gesetzt")
    );
}

/// English interface: the `step_label` asserts of Swift's testEnglishTexts (the rest is in layout and overlay).
#[test]
fn test_english_texts_status() {
    set_german(false);
    let label = |tool: &str, input: Value| step_label(tool, &obj(input));
    assert_eq!(
        label("Edit", json!({"file_path": "/a/b/Layout.swift"})),
        "edits Layout.swift"
    );
    assert_eq!(
        label("WebFetch", json!({"url": "https://docs.warp.dev/x"})),
        "reads web page docs.warp.dev"
    );
    assert_eq!(
        label(
            "AskUserQuestion",
            json!({"questions": [{"question": "Which one?"}]})
        ),
        "Question: Which one?"
    );
}

// ---------- Beyond Swift: run_command and the timestamp parser ----------

#[test]
fn run_command_limits_and_exit_codes() {
    let none: BTreeMap<String, String> = BTreeMap::new();
    let (log, mut report) = recorder();
    let out = run_command(
        "/bin/sh",
        &["-c", "sleep 5"],
        &none,
        &[],
        0.3,
        &[0],
        &mut *report,
    );
    assert!(out.is_none(), "time limit: no result");
    assert_eq!(
        said(&log),
        ["Watcher: /bin/sh -c sleep 5 ueberschritt 0s — abgebrochen"]
    );
    assert!(
        run_command(
            "/bin/sh",
            &["-c", "exit 1"],
            &none,
            &[],
            5.0,
            &[0],
            &mut |_| {}
        )
        .is_none(),
        "exit 1 not accepted"
    );
    assert_eq!(
        run_command(
            "/bin/sh",
            &["-c", "printf x; exit 1"],
            &none,
            &[],
            5.0,
            &[0, 1],
            &mut |_| {}
        ),
        Some(b"x".to_vec()),
        "exit 1 accepted (lsof)"
    );
    let mut lines = vec![];
    assert!(
        run_command("/nonexistent/x", &[], &none, &[], 1.0, &[0], &mut |l| lines
            .push(l.to_owned()))
        .is_none()
    );
    assert!(lines[0].starts_with("Watcher: /nonexistent/x nicht startbar — "));
}

/// review quick 2026-11-06 (Codex): the output was read without a limit; a command that floods it is a failure now.
#[test]
fn run_command_refuses_a_flood() {
    let none: BTreeMap<String, String> = BTreeMap::new();
    let mut lines = vec![];
    let out = run_command(
        "/bin/sh",
        &["-c", "head -c 17000000 /dev/zero"],
        &none,
        &[],
        10.0,
        &[0],
        &mut |l| lines.push(l.to_owned()),
    );
    assert!(out.is_none(), "over 16 MB: no result");
    assert_eq!(
        lines,
        ["Watcher: /bin/sh -c head -c 17000000 /dev/zero lieferte mehr als 16 MB — verworfen"]
    );
}

/// review quick 2026-11-06 (GLM): a grandchild that inherits the output kept a pipe open after the command ended; the
/// reading blocked, the run counted as a timeout, and its reader thread leaked for good.
#[test]
fn run_command_does_not_wait_for_grandchildren() {
    let none: BTreeMap<String, String> = BTreeMap::new();
    let start = std::time::Instant::now();
    let out = run_command(
        "/bin/sh",
        &["-c", "sleep 3 & echo hi"],
        &none,
        &[],
        2.0,
        &[0],
        &mut |_| {},
    );
    assert_eq!(
        out,
        Some(b"hi\n".to_vec()),
        "the command ended with its output"
    );
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "no wait for the grandchild"
    );
}

#[test]
fn timestamps_like_iso8601_date_formatter() {
    // Measured with Foundation 2026-11-06.
    let t = transcript::parse_timestamp;
    assert_eq!(t("2026-09-21T12:40:00Z"), Some(1_789_994_400.0));
    assert_eq!(t("2026-09-21T12:40:00.000Z"), Some(1_789_994_400.0));
    assert_eq!(t("2026-09-21T12:40:00.519Z"), Some(1_789_994_400.519));
    assert_eq!(
        t("2026-09-21T12:40:00.123456Z"),
        Some(1_789_994_400.123),
        "cut to milliseconds"
    );
    assert_eq!(t("2026-09-21T12:40:00+02:00"), Some(1_789_987_200.0));
    assert_eq!(t("2026-09-21T12:40:00+0200"), Some(1_789_987_200.0));
    assert_eq!(t("2026-09-21T12:40:00z"), Some(1_789_994_400.0));
    assert_eq!(t(" 2026-09-21T12:40:00Z "), Some(1_789_994_400.0));
    for bad in [
        "2026-09-21 12:40:00Z",
        "2026-09-21t12:40:00Z",
        "2026-09-21T12:40Z",
        "2026-09-21T12:40:00",
        "x",
    ] {
        assert_eq!(t(bad), None, "{bad}");
    }
}

/// review quick 2026-11-06 (Codex), measured before the fix in a release build: a line of n blanks in an answer made the
/// heading pattern try every split of its three `[ \t]*` (cubic): n = 500 took 1.1 s, 1,000 took 8.5 s, 2,000 took 68 s
/// per evaluation, every 6 s. The regex engine now gives up a start position after a step budget.
#[test]
fn long_blank_lines_do_not_stall_the_word_lists() {
    let text = format!("nothing needed\n{}x", " ".repeat(2_000));
    let start = std::time::Instant::now();
    let _ = super::words::without_progress_notes(&text);
    let _ = super::words::asks_nothing(&text, false);
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "took {:?}",
        start.elapsed()
    );
}
