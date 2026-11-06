//! Mapping tests, ported assert for assert from LayoutTests.swift, plus the title ping rounds and joining sessions to
//! windows (no Swift test: they needed live AX windows).

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::geom::Rect;
use crate::model::Digest;
use crate::motion::{ax_origin, cocoa_to_ax};
use crate::testutil::*;

fn row(pid: Pid, ppid: Pid, tty: &str, cpu: f64, comm: &str) -> PsRow {
    PsRow {
        pid,
        ppid,
        tty: tty.into(),
        cpu,
        comm: comm.into(),
    }
}

/// Swift `EngineProc(pid:engine:tty:cwd:)`: under Warp or Ghostty by default.
fn engine_proc(pid: Pid, engine: Engine, tty: &str, cwd: &str) -> EngineProc {
    EngineProc {
        pid,
        engine,
        tty: tty.into(),
        cwd: cwd.into(),
        in_terminal: true,
    }
}

fn no_cwd(_: Pid) -> String {
    String::new()
}

fn ids(sessions: &[Session]) -> Vec<&str> {
    sessions.iter().map(|s| s.id.as_str()).collect()
}

/// Swift `psFixture` as rows (the ps text parser is not ported). Live 2026-09-26: claude ← -zsh ← Warp helper ← Warp ←
/// launchd (both Warp processes are called `stable` in ps); plus a session in iTerm, an orphan (parent pid missing) and
/// a loop in the parent map.
fn ps_fixture() -> Vec<PsRow> {
    vec![
        row(1, 0, "??", 20_248.94, "/sbin/launchd"),
        row(
            500,
            1,
            "??",
            3_662.1,
            "/Applications/Warp.app/Contents/MacOS/stable",
        ),
        row(
            510,
            500,
            "??",
            251.0,
            "/Applications/Warp.app/Contents/MacOS/stable",
        ),
        row(520, 510, "ttys001", 0.1, "-zsh"),
        row(530, 520, "ttys001", 185.5, "/Users/test/.local/bin/claude"),
        row(
            600,
            1,
            "??",
            30.0,
            "/Applications/iTerm.app/Contents/MacOS/iTerm2",
        ),
        row(610, 600, "ttys002", 0.1, "-zsh"),
        row(620, 610, "ttys002", 2.0, "claude"),
        row(700, 999, "ttys003", 0.2, "claude"),
        row(800, 801, "ttys004", 0.2, "codex"),
        row(801, 800, "ttys004", 0.1, "-zsh"),
        row(
            900,
            530,
            "ttys001",
            0.4,
            "npm exec @playwright/mcp@0.0.79 --browser chrome",
        ),
    ]
}

fn digest(title: Option<&str>, name: Option<&str>) -> Option<Digest> {
    Some(Digest {
        title: title.map(str::to_owned),
        name: name.map(str::to_owned),
        ..Digest::default()
    })
}

// ---------- Task 5: Session an Prozess haengen ----------

#[test]
fn test_attach_processes() {
    let procs = [
        engine_proc(100, Engine::Claude, "ttys001", "/Users/test/Projects/Alpha"),
        engine_proc(200, Engine::Codex, "ttys003", "/Users/test/Projects/Alpha"),
    ];
    let sessions = [
        // interactive, pid known -> stays
        Session::new("claude:a", Engine::Claude, Some(100), "a", "", true, true),
        // interactive, pid no longer runs -> out
        Session::new("claude:b", Engine::Claude, Some(999), "b", "", true, true),
        // background agent without pid -> stays, but WITHOUT a guessed pid
        Session::new(
            "claude:c",
            Engine::Claude,
            None,
            "c",
            "/Users/test/Projects/Alpha",
            true,
            true,
        ),
        // Codex log session -> attached by working folder
        Session::new(
            "codex:d",
            Engine::Codex,
            None,
            "d",
            "/Users/test/Projects/Alpha",
            true,
            true,
        ),
        // Codex log session without a running process -> out
        Session::new("codex:e", Engine::Codex, None, "e", "/tmp/weg", true, true),
    ];
    let live = attach_processes(&sessions, &procs);
    assert_eq!(ids(&live), ["claude:a", "claude:c", "codex:d"]);
    let pid_of = |id: &str| live.iter().find(|s| s.id == id).and_then(|s| s.pid);
    assert_eq!(
        pid_of("claude:c"),
        None,
        "a background agent must not get a guessed pid, else it lands in a foreign window"
    );
    assert_eq!(pid_of("codex:d"), Some(200));
}

/// Acceptance 2026-09-18: a session started as bare `claude` stayed "Ohne Fenster": ps indents the name with two spaces
/// and " claude" was no engine. Only the `engine_of` asserts remain; the rows come from the platform layer now, the ps
/// text parser and its asserts (columns, CPU time formats, unreadable lines) are obsolete.
#[test]
fn test_parse_ps_line() {
    assert_eq!(
        engine_of(&row(7087, 6746, "ttys003", 0.12, "claude")),
        Some(Engine::Claude),
        "bare name"
    );
    let path = row(
        29834,
        29800,
        "ttys000",
        20_248.94,
        "/Users/test/.local/bin/claude",
    );
    assert_eq!(engine_of(&path), Some(Engine::Claude), "full path");
    assert_eq!(
        engine_of(&row(1234, 1200, "ttys009", 1.0, "codex")),
        Some(Engine::Codex)
    );
    let no_tty = row(
        20894,
        1,
        "??",
        130.0,
        "/Applications/ChatGPT.app/Contents/Resources/codex",
    );
    assert_eq!(
        engine_of(&no_tty),
        None,
        "read, but without a terminal no window"
    );
    let npm = row(
        7174,
        6746,
        "ttys003",
        0.4,
        "npm exec @playwright/mcp@0.0.79 --browser chrome",
    );
    assert_eq!(engine_of(&npm), None, "no AI CLI, name with spaces");
    assert_eq!(
        engine_of(&row(6746, 6700, "ttys003", 0.1, "-zsh")),
        None,
        "shell: only for the parent map"
    );
}

/// The table of Apple's `lastPathComponent` documentation, and the cases above.
#[test]
fn last_path_component_like_nsstring() {
    for (path, last) in [
        ("/tmp/scratch.tiff", "scratch.tiff"),
        ("/tmp/scratch", "scratch"),
        ("/tmp/", "tmp"),
        ("scratch///", "scratch"),
        ("/", "/"),
        ("", ""),
        ("claude", "claude"),
        (
            "npm exec @playwright/mcp@0.0.79 --browser chrome",
            "mcp@0.0.79 --browser chrome",
        ),
    ] {
        assert_eq!(last_path_component(path), last, "{path:?}");
    }
}

// ---------- Plan 1a: fremde Terminals in Ruhe lassen (2026-09-26) ----------

/// Review finding 8 (2026-09-25): AI processes in VS Code, iTerm and Terminal.app were pinged every 2 min, their tab
/// title flashed and jumped to the folder name. `in_terminal`: does the process run under Warp or Ghostty?
#[test]
fn test_process_tree_in_terminal() {
    let list = process_list(&ps_fixture(), &BTreeSet::from([500]), no_cwd);
    let procs: BTreeMap<Pid, &EngineProc> = list.iter().map(|p| (p.pid, p)).collect();
    assert_eq!(
        procs.keys().copied().collect::<Vec<_>>(),
        [530, 620, 700, 800],
        "only AI CLIs with a terminal"
    );
    assert!(
        procs[&530].in_terminal,
        "Warp ancestor: shell and Warp helper stand only in the parent map"
    );
    assert!(!procs[&620].in_terminal, "iTerm ancestor");
    assert!(!procs[&700].in_terminal, "orphan");
    assert!(!procs[&800].in_terminal, "a loop ends");

    let parents = BTreeMap::from([(3, 2), (2, 1), (1, 0), (7, 8), (8, 7)]);
    let one = BTreeSet::from([1]);
    assert!(has_ancestor(3, &one, &parents));
    assert!(
        !has_ancestor(1, &one, &parents),
        "only ancestors, not the process itself"
    );
    assert!(!has_ancestor(7, &one, &parents), "loop");
    let deep: BTreeMap<Pid, Pid> = (1..=40).map(|p| (p, p - 1)).collect();
    assert!(
        has_ancestor(40, &BTreeSet::from([9]), &deep) && !has_ancestor(40, &one, &deep),
        "at most 32 steps"
    );

    // R2-8 ("unreadable" is "unknown", not "no processes") is the platform reader's job now.
    assert!(
        process_list(&[], &BTreeSet::from([500]), no_cwd).is_empty(),
        "nothing read -> no processes"
    );
    assert!(
        process_list(
            &[row(1, 0, "??", 1.0, "/sbin/launchd")],
            &BTreeSet::new(),
            no_cwd
        )
        .is_empty(),
        "readable, but no AI CLI -> empty"
    );
}

/// Plan 1a (GP 1, R2-8): a session in another terminal stays in the list ("Ohne Fenster"), is never pinged and does not
/// block the name matching; before, without a title it blocked every name match.
#[test]
fn test_foreign_terminal_left_alone() {
    let procs = process_list(&ps_fixture(), &BTreeSet::from([500]), no_cwd);
    let warp = Session {
        digest: digest(Some("Lookout bauen"), None),
        ..Session::new("claude:w", Engine::Claude, Some(530), "w", "", false, true)
    };
    let iterm = Session::new("claude:i", Engine::Claude, Some(620), "i", "", false, true);
    let live = attach_processes(&[warp, iterm], &procs);
    assert_eq!(
        ids(&live),
        ["claude:w", "claude:i"],
        "the foreign session stays listed"
    );
    let terminal: Vec<Pid> = procs
        .iter()
        .filter(|p| p.in_terminal)
        .map(|p| p.pid)
        .collect();
    let s = ping_schedule(
        &terminal,
        &BTreeSet::new(),
        &BTreeMap::new(),
        true,
        1_000_000.0,
        true,
        true,
    );
    assert_eq!(
        s.due,
        BTreeSet::from([530]),
        "only processes under Warp/Ghostty are pinged"
    );
    let wins = [tab_window(vec![], "◐ Lookout bauen")];
    // review quick 2026-09-26 (Kimi): the function itself leaves foreign processes out, also with the full list.
    assert_eq!(
        title_matches(&live, &wins, &procs),
        BTreeMap::from([(530, 0)]),
        "a foreign session without a title does not block the name matching"
    );
}

// ---------- Fehler 1: Zuordnung nach herausgezogenem Tab ----------

#[test]
fn test_ping_schedule() {
    let t = 1_000_000.0;
    let nothing = BTreeSet::new();
    let fresh_map = BTreeMap::new();
    // New process without a window -> at once; known matching -> rest.
    let s = ping_schedule(
        &[1, 2],
        &BTreeSet::from([2]),
        &fresh_map,
        false,
        t,
        true,
        true,
    );
    assert_eq!(s.due, BTreeSet::from([1]), "only the unknown one");
    // A NEW window (tab pulled out): every matching is checked again.
    let s = ping_schedule(
        &[1, 2],
        &BTreeSet::from([1, 2]),
        &fresh_map,
        true,
        t,
        true,
        true,
    );
    assert_eq!(
        s.due,
        BTreeSet::from([1, 2]),
        "new window -> check all again"
    );
    // Misses: never give up for good, only ask more rarely.
    let mut pending = BTreeMap::new();
    let mut now = t;
    for attempt in 1..=8 {
        let r = ping_schedule(&[1], &nothing, &pending, false, now, true, true);
        assert_eq!(r.due, BTreeSet::from([1]), "try {attempt} due");
        pending = ping_result(&r.due, &nothing, &r.pending, now);
        let wait = ping_retry_interval(pending[&1].attempts);
        assert_eq!(
            wait,
            if attempt < 4 { 15.0 } else { 120.0 },
            "try {attempt}: gap {wait}"
        );
        let early = ping_schedule(
            &[1],
            &nothing,
            &pending,
            false,
            now + wait - 1.0,
            true,
            true,
        );
        assert!(early.due.is_empty(), "not again before the gap is over");
        now += wait;
    }
    // Without Accessibility no title is readable: no ping at all, every try would only write markers and folder names
    // into the user's tab titles (live 18:26 after the rebuild).
    let blind = ping_schedule(&[1, 2], &nothing, &fresh_map, true, t, false, true);
    assert!(
        blind.due.is_empty(),
        "no ping without access, got {:?}",
        blind.due
    );
    assert_eq!(
        blind.pending.keys().copied().collect::<BTreeSet<_>>(),
        BTreeSet::from([1, 2]),
        "noted for after the access"
    );
    // Locked screen: AX gives not a single terminal window. A ping could find nothing and only set every tab title to the
    // folder name (log until 2026-09-25: 3,249 rounds, 15,696 titles).
    let dunkel = ping_schedule(&[1, 2], &nothing, &fresh_map, true, t, true, false);
    assert!(
        dunkel.due.is_empty(),
        "no ping without a readable window, got {:?}",
        dunkel.due
    );
    assert_eq!(
        dunkel.pending.keys().copied().collect::<BTreeSet<_>>(),
        BTreeSet::from([1, 2]),
        "noted for later"
    );
    // A new window checks everything again at once, but a background tab falls back to its long gap afterwards instead
    // of flashing every 15 s again (review quick v3).
    let fresh = ping_schedule(&[1], &nothing, &pending, true, now, true, true);
    assert_eq!(
        fresh.due,
        BTreeSet::from([1]),
        "new window -> check again at once"
    );
    let after = ping_result(&fresh.due, &nothing, &fresh.pending, now);
    assert_eq!(
        ping_retry_interval(after[&1].attempts),
        120.0,
        "the counter stays, got {}",
        after[&1].attempts
    );
    // Found -> rest; vanished process -> forgotten.
    pending = ping_result(&BTreeSet::from([1]), &BTreeSet::from([1]), &pending, now);
    assert_eq!(pending.get(&1), None);
    let old = BTreeMap::from([(
        7,
        PingState {
            attempts: 2,
            last_try: t,
        },
    )]);
    let gone = ping_schedule(&[], &nothing, &old, false, now, true, true);
    assert!(gone.pending.is_empty() && gone.due.is_empty());
}

/// `pingMap` without AX (no Swift test, it needed live windows). Known pairs live while process and window live; a
/// marker maps its window and gets the original title back at once; two windows with the same marker: the first wins; a
/// marker left in the snapshot title and a process never found fall back to the folder, without folder the engine.
#[test]
fn ping_rounds_match_markers() {
    let procs = [
        engine_proc(11, Engine::Claude, "ttys011", "/Users/test/Projects/Alpha"),
        engine_proc(12, Engine::Claude, "ttys012", "/Users/test/Projects/Beta/"),
        engine_proc(13, Engine::Codex, "ttys013", ""),
        engine_proc(14, Engine::Claude, "ttys014", "/w/known"),
    ];
    let mut before = vec![
        tab_window(vec![], "✳ Alpha task"),
        tab_window(vec![], "wg-map-99"),
        tab_window(vec![], "zsh"),
        tab_window(vec![], "◐ known"),
    ];
    for (i, w) in before.iter_mut().enumerate() {
        w.id = 100 + i as u64;
    }
    // 14 lives in window 103 and stays; 15 has ended; the window of 16 is gone.
    let known = BTreeMap::from([(14, 103), (15, 101), (16, 999)]);
    let mut ping = Ping::start(&procs, &known, &before, &BTreeSet::from([11, 12, 13]));
    assert_eq!(
        ping.map,
        BTreeMap::from([(14, 103)]),
        "known pairs live while process and window live"
    );
    let markers: Vec<(String, String)> = ping
        .markers()
        .into_iter()
        .map(|w| (w.tty, w.title))
        .collect();
    assert_eq!(
        markers,
        [
            ("ttys011".into(), "wg-map-11".into()),
            ("ttys012".into(), "wg-map-12".into()),
            ("ttys013".into(), "wg-map-13".into())
        ]
    );
    let titles = |t: [&str; 4]| t.map(String::from).to_vec();
    let write = |tty: &str, title: &str| TitleWrite {
        tty: tty.into(),
        title: title.into(),
    };
    let back = ping.read(&titles([
        "wg-map-11",
        "wg-map-12",
        "wg-map-12",
        "wg-map-77",
    ]));
    assert_eq!(
        back,
        [write("ttys011", "✳ Alpha task"), write("ttys012", "Beta")],
        "original title back at once; a marker in the snapshot title gives the folder name"
    );
    assert_eq!(
        ping.map,
        BTreeMap::from([(11, 100), (12, 101), (14, 103)]),
        "the first window with the marker wins"
    );
    assert_eq!(ping.open.iter().map(|p| p.pid).collect::<Vec<_>>(), [13]);
    assert!(
        ping.read(&titles(["wg-map-11", "", "", ""])).is_empty(),
        "found once is found"
    );
    assert_eq!(
        ping.finish(),
        [write("ttys013", "codex")],
        "never found: folder name, without folder the engine"
    );
    assert_eq!(
        ping.summary(),
        "Mapper: 2/3 gepingt, 3/4 Sessions zugeordnet (1 ohne Marker — Hintergrund-Tab oder Spinner, neuer Versuch folgt)"
    );
    let quiet = Ping::start(&procs, &known, &before, &BTreeSet::new());
    assert!(
        quiet.open.is_empty() && quiet.markers().is_empty(),
        "nothing due: titles stay alone"
    );
}

// ---------- v4: saubere Namen, Fenster am Namen, verworfene Klicks (design 2026-09-21) ----------

/// Kevin 2026-09-21: a new window came to the middle late. Log: working sessions paint over the ping marker ("ohne
/// Marker" four times in a row); now Lookout recognises windows by the name in the title.
#[test]
fn test_title_matches_windows() {
    let a = Session {
        digest: digest(Some("Auto A"), Some("Lookout · Fenster-Übersicht")),
        ..Session::new("claude:a", Engine::Claude, Some(11), "a", "", false, true)
    };
    let b = Session {
        digest: digest(Some("Qwen-Image safety check"), None),
        ..Session::new("claude:b", Engine::Claude, Some(12), "b", "", true, true)
    };
    let codex = Session::new(
        "codex:c",
        Engine::Codex,
        Some(13),
        "Alpha",
        "/Users/test/Projects/Alpha",
        false,
        true,
    );
    let wins = vec![
        tab_window(vec![], "◐ Lookout · Fenster-Übersicht"),
        tab_window(vec![], "✳ Qwen-Image safety check"),
        tab_window(vec![], "Alpha"),
        tab_window(vec![], "◑ Unbekannt"),
    ];
    // Running terminal processes of exactly these sessions.
    fn procs(s: &[&Session]) -> Vec<EngineProc> {
        s.iter()
            .filter_map(|x| {
                x.pid
                    .map(|p| engine_proc(p, x.engine, &format!("ttys0{p}"), ""))
            })
            .collect()
    }
    let m = title_matches(
        &[a.clone(), b.clone(), codex.clone()],
        &wins,
        &procs(&[&a, &b, &codex]),
    );
    assert_eq!(
        m,
        BTreeMap::from([(11, 0), (12, 1)]),
        "unique Claude pairs by name or topic"
    );
    let zwilling = Session {
        digest: a.digest.clone(),
        ..Session::new("claude:a2", Engine::Claude, Some(14), "a2", "", false, true)
    };
    assert!(
        title_matches(
            &[a.clone(), zwilling.clone()],
            &wins,
            &procs(&[&a, &zwilling])
        )
        .is_empty(),
        "two sessions with the same name -> the ping decides"
    );
    let doppelt = [
        tab_window(vec![], "◐ Lookout · Fenster-Übersicht"),
        tab_window(vec![], "◑ Lookout · Fenster-Übersicht"),
    ];
    assert!(
        title_matches(std::slice::from_ref(&a), &doppelt, &procs(&[&a])).is_empty(),
        "two windows with the same title -> no match"
    );
    // Without a Claude glyph the window shows no Claude session (Codex folder name, title written back after a ping).
    let ordner = Session {
        digest: digest(Some("Alpha"), None),
        ..Session::new("claude:o", Engine::Claude, Some(15), "o", "", true, true)
    };
    assert!(
        title_matches(std::slice::from_ref(&ordner), &wins, &procs(&[&ordner])).is_empty(),
        "a title without glyph proves nothing"
    );
    // Kevin 2026-10-24: a fixed tab title (Warp launch configuration) never shows a Claude glyph and swallows the ping
    // marker. Fallback: the session's own name as the WHOLE title (the topic does not count, see `ordner`); glyph hits
    // go first.
    let fest = Session {
        digest: digest(Some("Auto F"), Some("Shop · #12 Knopf bauen")),
        ..Session::new("claude:f", Engine::Claude, Some(18), "f", "", false, true)
    };
    let mut fest_wins = wins.clone();
    fest_wins.extend([
        tab_window(vec![], "Shop · #12 Knopf bauen"),
        tab_window(vec![], "Lookout · Fenster-Übersicht"),
    ]);
    let mf = title_matches(
        &[a.clone(), b.clone(), codex.clone(), fest.clone()],
        &fest_wins,
        &procs(&[&a, &b, &codex, &fest]),
    );
    assert_eq!(
        mf,
        BTreeMap::from([(11, 0), (12, 1), (18, 4)]),
        "fixed title = own name -> matched, a glyph hit goes first"
    );
    let fest_doppelt = [
        tab_window(vec![], "Shop · #12 Knopf bauen"),
        tab_window(vec![], "Shop · #12 Knopf bauen"),
    ];
    assert!(
        title_matches(std::slice::from_ref(&fest), &fest_doppelt, &procs(&[&fest])).is_empty(),
        "two equal fixed titles -> ping"
    );
    // review quick v4: a title is unique only if EVERY Claude session in a terminal is known. A fresh one (not asked yet
    // or without transcript) may already carry the same start name; else a background session would hang at the wrong
    // window for good (mapped processes are never pinged again).
    let three = [a.clone(), b.clone(), codex.clone()];
    let mut frisch = procs(&[&a, &b, &codex]);
    frisch.push(engine_proc(16, Engine::Claude, "ttys016", ""));
    assert!(
        title_matches(&three, &wins, &frisch).is_empty(),
        "Claude process without session -> the ping decides"
    );
    let ohne_titel = Session::new("claude:n", Engine::Claude, Some(16), "n", "", false, true);
    assert!(
        title_matches(
            &[a.clone(), b.clone(), codex.clone(), ohne_titel],
            &wins,
            &frisch
        )
        .is_empty(),
        "session without a known name -> the ping decides"
    );
    let mut grok = procs(&[&a, &b, &codex]);
    grok.push(engine_proc(17, Engine::Grok, "ttys017", ""));
    assert_eq!(
        title_matches(&three, &wins, &grok),
        BTreeMap::from([(11, 0), (12, 1)]),
        "other engines do not disturb"
    );
}

// ---------- Durchsicht 2026-09-25 (Kevin's screenshot: green frame without reason) ----------

/// A snapshot during which Lookout moved windows itself shows intermediate frames: the last measured ones stay, else a
/// window that had just arrived jumped back and glided again (variant of GLM/Kimi "menu click during the glide",
/// 2026-09-25).
#[test]
fn test_snapshot_during_own_move() {
    fn win(id: u64, frame: Rect) -> TermWindow {
        TermWindow {
            id,
            frame,
            ..tab_window(vec![], "")
        }
    }
    let angekommen = Rect::new(0.0, 0.0, 804.0, 413.0);
    let unterwegs = Rect::new(400.0, -300.0, 804.0, 413.0);
    let fremd = Rect::new(1000.0, 0.0, 500.0, 500.0);
    let merged = keep_measured_frames(
        vec![win(1, unterwegs), win(2, fremd)],
        &[win(1, angekommen)],
    );
    assert_eq!(
        merged[0].frame, angekommen,
        "the measured frame counts, not the intermediate one"
    );
    assert_eq!(
        merged[1].frame, fremd,
        "an unknown window keeps its snapshot"
    );
    // Plan 1c (R2-4): bringing to the front fetches a minimized window; a tick already running still brings the old
    // snapshot and must not set it back to "minimized".
    let mut alt = win(3, angekommen);
    alt.minimized = true;
    assert!(
        !keep_measured_frames(vec![alt], &[win(3, angekommen)])[0].minimized,
        "fetched stays fetched"
    );
}

/// Which session does the user have in front of him? The visible one of the focused window, only if its terminal is in
/// front.
#[test]
fn test_session_in_front() {
    fn win(id: u64, title: &str, sessions: Vec<Session>) -> TermWindow {
        TermWindow {
            id,
            owner_pid: 900,
            frame: Rect::new((id as f64 - 900.0) * 700.0, 100.0, 600.0, 400.0),
            ..tab_window(sessions, title)
        }
    }
    fn shown(ws: &[&TermWindow]) -> Vec<OnScreenWindow> {
        ws.iter()
            .map(|w| OnScreenWindow {
                number: w.frame.min_x() as i64,
                pid: w.owner_pid,
                rect: cocoa_to_ax(w.frame, ax_origin(&w.bundle_id), 1000.0),
            })
            .collect()
    }
    fn claude(id: &str, title: &str) -> Session {
        Session {
            digest: digest(Some(title), None),
            ..Session::new(
                format!("claude:{id}"),
                Engine::Claude,
                Some(1),
                id,
                "",
                true,
                true,
            )
        }
    }
    let eins = win(901, "✳ Bericht", vec![claude("a", "Bericht")]);
    let zwei = win(
        902,
        "✳ Plan",
        vec![claude("b", "Bericht"), claude("c", "Plan")],
    );
    let unklar = win(
        903,
        "✳ Neu",
        vec![claude("d", "Bericht"), claude("e", "Plan")],
    );
    let ohne_thema = Session {
        digest: Some(Digest::default()),
        ..claude("f", "")
    };
    let neu = win(904, "✳ Irgendwas", vec![ohne_thema]);
    let codex = win(
        905,
        "codex",
        vec![Session::new(
            "codex:g",
            Engine::Codex,
            Some(2),
            "g",
            "",
            true,
            true,
        )],
    );
    let windows = [
        eins.clone(),
        zwei.clone(),
        unklar.clone(),
        neu.clone(),
        codex.clone(),
    ];
    let alle = shown(&windows.iter().collect::<Vec<_>>());
    let call = |w: &TermWindow, title: &str, pid: Pid, on_screen: &[OnScreenWindow]| {
        session_in_front(
            &windows,
            Some(w.id),
            Some(title),
            Some(pid),
            Some(on_screen),
            1000.0,
        )
    };
    let front = |w: &TermWindow| call(w, &w.title, 900, &alle);
    assert_eq!(
        session_in_front(
            &windows,
            Some(eins.id),
            None,
            Some(900),
            Some(&alle),
            1000.0
        ),
        None,
        "title not readable fresh: none (review quick 2026-10-04, Codex)"
    );
    assert_eq!(front(&eins).as_deref(), Some("claude:a"), "one session");
    assert_eq!(front(&zwei).as_deref(), Some("claude:c"), "visible tab");
    assert_eq!(front(&unklar), None, "visible tab unclear: none");
    assert_eq!(
        call(&eins, &eins.title, 77, &alle),
        None,
        "another app in front: none"
    );
    assert_eq!(
        session_in_front(
            &windows,
            None,
            Some("✳ Bericht"),
            Some(900),
            Some(&alle),
            1000.0
        ),
        None,
        "no focus known: none"
    );
    assert_eq!(
        session_in_front(
            &windows,
            Some(999),
            Some("✳ Bericht"),
            Some(900),
            Some(&alle),
            1000.0
        ),
        None,
        "a foreign window"
    );
    // review quick 2026-10-04 (GLM): minimized or on another desktop = not in front of the user.
    assert_eq!(
        call(&eins, &eins.title, 900, &shown(&[&zwei])),
        None,
        "not on the visible desktop: none"
    );
    assert_eq!(
        session_in_front(
            &windows,
            Some(eins.id),
            Some(&eins.title),
            Some(900),
            None,
            1000.0
        ),
        None,
        "window list not readable: none"
    );
    assert_eq!(
        call(&eins, &eins.title, 900, &shown(&[&eins, &eins])).as_deref(),
        Some("claude:a"),
        "congruent with a parked one: counts"
    );
    // review quick 2026-10-04 (Codex): one session, but the user has the plain shell tab beside it in front of him.
    assert_eq!(
        call(&eins, "~/projekt — zsh", 900, &alle),
        None,
        "shell tab without Claude glyph: none"
    );
    assert_eq!(
        call(&eins, "✳ Anderes Thema", 900, &alle),
        None,
        "Claude glyph, but a foreign topic: none"
    );
    assert_eq!(
        call(&eins, "◐ Bericht", 900, &alle).as_deref(),
        Some("claude:a"),
        "the fresh title counts (working right now)"
    );
    assert_eq!(
        call(&zwei, "✳ Bericht", 900, &alle).as_deref(),
        Some("claude:b"),
        "the fresh title picks the tab"
    );
    assert_eq!(
        front(&neu),
        None,
        "topic unknown: none, the tab of an ended session also carries ✳ (review quick, GLM)"
    );
    assert_eq!(front(&codex), None, "Claude sessions only");
}

/// `matchSessions` (no Swift test): sessions go to the window of their pid; old ones go, windows that vanished and
/// sessions without pid get none.
#[test]
fn match_sessions_puts_sessions_into_windows() {
    let mut windows = vec![
        tab_window(vec![tab_session("old", true, false, None)], "a"),
        tab_window(vec![], "b"),
    ];
    windows[0].id = 1;
    windows[1].id = 2;
    let s = |id: &str, pid: Option<Pid>| Session::new(id, Engine::Claude, pid, id, "", false, true);
    let sessions = [
        s("claude:x", Some(10)),
        s("claude:y", Some(11)),
        s("claude:z", None),
        s("claude:w", Some(12)),
    ];
    match_sessions(
        &mut windows,
        &sessions,
        &BTreeMap::from([(10, 2), (11, 2), (12, 9)]),
    );
    assert!(windows[0].sessions.is_empty(), "old sessions go");
    assert_eq!(
        ids(&windows[1].sessions),
        ["claude:x", "claude:y"],
        "by pid; window gone or no pid: none"
    );
}

/// review quick 2026-11-06 (GLM): the title read back from the window goes into an OSC 0 sequence on the session's tty.
/// An ESC or BEL in it would end that sequence early and let the rest act as terminal commands (ESC[6n makes the
/// terminal type its answer into the session), so control characters never go back.
#[test]
fn restored_titles_carry_no_control_characters() {
    let procs = [engine_proc(
        21,
        Engine::Claude,
        "ttys021",
        "/w/Bad\u{7}\u{1b}[6nDir",
    )];
    let mut before = vec![
        tab_window(vec![], "x\u{1b}]8;;http://evil\u{1b}\\y\u{7}z\u{9b}6n"),
        tab_window(vec![], "zsh"),
    ];
    before[0].id = 200;
    before[1].id = 201;
    let mut ping = Ping::start(&procs, &BTreeMap::new(), &before, &BTreeSet::from([21]));
    let back = ping.read(&["wg-map-21".to_string(), "zsh".to_string()]);
    assert_eq!(back.len(), 1);
    assert_eq!(
        back[0].title, "x]8;;http://evil\\yz6n",
        "original title without ESC, BEL, CSI"
    );
    let mut lost = Ping::start(&procs, &BTreeMap::new(), &before, &BTreeSet::from([21]));
    lost.read(&["".to_string(), "".to_string()]);
    assert_eq!(
        lost.finish()[0].title,
        "Bad[6nDir",
        "the folder fallback too"
    );
}

/// review quick 2026-11-06 (Muse): a process the name matching found kept the retry entry of an earlier miss and was
/// pinged again when that came due; its tab title flashed the marker although the matching had settled.
#[test]
fn mapped_processes_leave_the_ping_queue() {
    let t = 1_000_000.0;
    let missed = BTreeMap::from([(
        7,
        PingState {
            attempts: 1,
            last_try: t - 100.0,
        },
    )]);
    let s = ping_schedule(&[7], &BTreeSet::from([7]), &missed, false, t, true, true);
    assert!(
        s.due.is_empty() && s.pending.is_empty(),
        "matched by name: no more pings"
    );
    let s = ping_schedule(&[7], &BTreeSet::from([7]), &missed, true, t, true, true);
    assert_eq!(
        s.due,
        BTreeSet::from([7]),
        "a new window still checks it again"
    );
    assert_eq!(s.pending[&7].attempts, 1, "keeping its count of misses");
}
