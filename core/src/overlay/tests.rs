//! Overlay tests, ported assert for assert from LayoutTests.swift, plus the parking lot, the Lookout line as data, list
//! and preview content, glide timing and glow breathing (no Swift test: they lived in the app or in AppKit classes).

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::i18n::set_german;
use crate::layout::{PlanInput, ReadAnswers, mark_read, plan_arrangement, set_pinned};
use crate::model::LayoutMode::Reading;
use crate::model::StackSide::Right;
use crate::model::Target::Single;
use crate::model::{CardButton, Digest, LineStatus};
use crate::motion::{AxOrigin, GHOSTTY_BID};
use crate::testutil::*;

static NO_SINCE: BTreeMap<String, SinceEntry> = BTreeMap::new();
static NO_LINES: BTreeMap<String, LookoutLine> = BTreeMap::new();

fn some(s: &str) -> Option<String> {
    Some(s.to_owned())
}

fn target_ids<T>(targets: &[T], id: impl Fn(&T) -> &str) -> Vec<&str> {
    targets.iter().map(id).collect()
}

// ---------- Status-Dateien statt Dauer-Abfrage: window order ----------

/// Frames and labels are ordered again only if a foreign window lies between (a WindowServer error per ordering).
#[test]
fn test_sits_above() {
    // Swift: getpid(), the default of `own`.
    let me: Pid = 4242;
    let r = Rect::new(0.0, 0.0, 10.0, 10.0);
    let on = |number, pid| OnScreenWindow {
        number,
        pid,
        rect: r,
    };
    let (panel, other, term, chrome) = (on(1, me), on(2, me), on(9, 500), on(7, 600));
    let sits = |stack: Option<&[OnScreenWindow]>| sits_above(1, 9, stack, me);
    assert!(
        sits(Some(&[chrome, panel, term])),
        "right above, a foreign window in front does not matter"
    );
    assert!(sits(Some(&[panel, other, term])), "only own panels between");
    assert!(
        !sits(Some(&[term, panel])),
        "terminal clicked: it lies above the panel"
    );
    assert!(
        !sits(Some(&[panel, chrome, term])),
        "a foreign window between"
    );
    assert!(!sits(Some(&[term])), "panel not on the screen yet");
    assert!(!sits(Some(&[panel])), "terminal not in the list");
    assert!(!sits(None), "list unknown: order as before");
}

// ---------- Kevins zwei Bildschirme (gemessen 2026-09-18) ----------

#[test]
fn test_no_glow_on_maximized_window() {
    let t0 = 1000.0;
    let mut voll = fake_window(true, t0);
    voll.frame = MACBOOK; // fills the screen completely
    let mut gekachelt = fake_window(true, t0);
    gekachelt.frame = Rect::new(
        MACBOOK.min_x(),
        MACBOOK.min_y(),
        MACBOOK.width() / 2.0,
        MACBOOK.height(),
    );
    let screens = [MACBOOK];
    assert!(
        is_maximized(&voll, &screens),
        "the full area counts as maximized"
    );
    assert!(!is_maximized(&gekachelt, &screens), "half a tile not");
    let windows = [voll, gekachelt];
    let ziele = glow_targets(&OverlayInput::new(&windows, None, 1117.0, &screens));
    assert_eq!(ziele.len(), 1, "only the tiled window glows");
}

/// Kevin 2026-09-18: the window he works in NEVER gets a frame, however large. Between two answers it is technically
/// "done".
#[test]
fn test_no_glow_on_active_window() {
    let t0 = 1000.0;
    let mut a = fake_window(true, t0);
    let mut b = fake_window(true, t0 + 1.0);
    a.frame = Rect::new(0.0, 0.0, 1000.0, 600.0);
    b.frame = Rect::new(1000.0, 0.0, 1000.0, 600.0);
    let windows = [a, b.clone()];
    let o = OverlayInput::new(&windows, None, 1269.0, &[OLED]);
    let ziele = glow_targets(&OverlayInput {
        focused: Some(0),
        ..o
    });
    assert_eq!(
        target_ids(&ziele, |t| &t.id),
        [b.sessions[0].id.as_str()],
        "the active session never glows"
    );
    // No stack special case any more: every done window gets its frame, the real depth order covers it where other
    // windows lie in front.
    assert_eq!(glow_targets(&o).len(), 2);
}

#[test]
fn test_glow_knows_its_window() {
    let mut w = fake_window(true, 1000.0); // owner pid 0
    w.frame = Rect::new(100.0, 200.0, 800.0, 600.0);
    let ax = cocoa_to_ax(w.frame, AxOrigin::TopLeft, 1269.0);
    let on = |number, pid| OnScreenWindow {
        number,
        pid,
        rect: ax,
    };
    let windows = [w.clone()];
    let glow = |list: Option<&[OnScreenWindow]>| {
        glow_targets(&OverlayInput::new(&windows, list, 1269.0, &[OLED]))
    };
    assert_eq!(
        glow(Some(&[on(42, 0)]))
            .first()
            .and_then(|t| t.window_number),
        Some(42),
        "the frame knows its window -> level right above it"
    );
    let fremd = [on(43, 999)];
    assert!(
        glow(Some(&fremd)).is_empty(),
        "same rectangle, another app: the window is not on this space"
    );
    let ohne_liste = glow(None);
    assert!(
        ohne_liste.len() == 1 && ohne_liste[0].window_number.is_none(),
        "window list not readable: do not hide wrongly, only without level"
    );
    // List read, but no terminal window on this screen (the user on a desktop with Chrome only): no frame. Before, empty
    // counted as "not readable" and every done window got a frame on a spot without a window.
    assert!(
        glow(Some(&[])).is_empty(),
        "no terminal window on the screen -> no frame"
    );
    // review quick 2026-09-18: two windows of the same app on the same rectangle cannot be told apart, rather no frame
    // than one above the wrong window.
    let doppelt = [on(42, 0), on(44, 0)];
    assert!(
        glow(Some(&doppelt)).is_empty(),
        "ambiguous match -> no frame"
    );
    // For the window list only "lies on the screen" counts, ambiguous is visible too.
    assert_eq!(
        locate_on_screen(&w, Some(&doppelt), 1269.0),
        OnScreenMatch::Ambiguous
    );
    assert_eq!(
        locate_on_screen(&w, Some(&fremd), 1269.0),
        OnScreenMatch::Absent
    );
    assert_eq!(
        locate_on_screen(&w, Some(&[]), 1269.0),
        OnScreenMatch::Absent,
        "read empty = not on the screen"
    );
    assert_eq!(
        locate_on_screen(&w, None, 1269.0),
        OnScreenMatch::Unknown,
        "not readable = unknown"
    );
}

/// Kevin 2026-09-18: "kevin-05 sehe ich nicht, aber er zeigt an, dass da ein Fenster sei": the Ghostty window lay
/// minimized in the Dock. The window list now says where it is.
#[test]
fn test_window_whereabouts() {
    let mut w = fake_window(true, 1000.0);
    assert!(
        whereabouts(&w, true) == WindowWhereabouts::Visible
            && WindowWhereabouts::Visible.label().is_empty()
    );
    assert_eq!(
        whereabouts(&w, false),
        WindowWhereabouts::OtherSpace,
        "not on the screen -> other desktop"
    );
    w.movable = false;
    assert_eq!(whereabouts(&w, true), WindowWhereabouts::Immovable);
    w.fullscreen = true;
    assert_eq!(
        whereabouts(&w, false),
        WindowWhereabouts::Fullscreen,
        "full screen lies on its own desktop"
    );
    w.minimized = true;
    assert_eq!(
        whereabouts(&w, false),
        WindowWhereabouts::Minimized,
        "minimized beats everything"
    );
    assert_eq!(WindowWhereabouts::Minimized.label(), "minimiert (im Dock)");
    assert_eq!(
        WindowWhereabouts::OtherSpace.label(),
        "anderer Schreibtisch"
    );
}

// ---------- Schilder + Grün-Fix (design 2026-09-21) ----------

/// The glow assert of testVisibleTabDecides (the rest is `model::tests::test_visible_tab_decides`): /handoff opens the
/// new tab in the same window, the old tab was done, the visible one worked.
#[test]
fn test_visible_tab_decides_glow() {
    let alt = tab_session("beta-52", true, false, Some("Handoff spawn loop closing"));
    let neu = tab_session(
        "beta-e5",
        false,
        false,
        Some("Handoff spawn 2026-09-21 beta-pr1-build"),
    );
    let windows = [tab_window(
        vec![alt, neu],
        "◐ Handoff spawn 2026-09-21 beta-pr1-build",
    )];
    assert!(
        glow_targets(&OverlayInput::new(&windows, None, 1117.0, &[MACBOOK])).is_empty(),
        "no frame"
    );
}

#[test]
fn test_advance_since() {
    let t0 = 10_000.0;
    let laeuft = Session {
        digest: Some(Digest {
            answer_at: Some(t0 - 5.0),
            prompt_at: Some(t0 - 600.0),
            ..Digest::default()
        }),
        ..Session::new("claude:a", Engine::Claude, Some(1), "a", "", false, true)
    };
    let fertig = Session {
        digest: Some(Digest {
            answer_at: Some(t0 - 360.0),
            ..Digest::default()
        }),
        ..Session::new("claude:b", Engine::Claude, Some(2), "b", "", true, true)
    };
    let since = |m: &BTreeMap<String, SinceEntry>, id: &str| m.get(id).and_then(|e| e.since);
    let first = advance_since(&BTreeMap::new(), &[laeuft.clone(), fertig.clone()], t0);
    assert_eq!(
        since(&first, "claude:a"),
        Some(t0 - 600.0),
        "running since the user's last prompt"
    );
    assert_eq!(
        since(&first, "claude:b"),
        Some(t0 - 360.0),
        "done since the last answer"
    );
    let same = advance_since(&first, &[laeuft, fertig], t0 + 6.0);
    assert_eq!(same, first, "same state: the start stays");
    let jetzt_fertig = Session::new("claude:a", Engine::Claude, Some(1), "a", "", true, true);
    let t1 = t0 + 12.0;
    let changed = advance_since(&same, &[jetzt_fertig], t1);
    assert_eq!(
        changed.get("claude:a"),
        Some(&SinceEntry {
            tone: LabelTone::Ready,
            since: Some(t1)
        }),
        "change: from now"
    );
    assert_eq!(
        changed.get("claude:b"),
        None,
        "a vanished session drops out"
    );
    let ohne = Session {
        digest: Some(Digest::default()),
        ..Session::new("claude:c", Engine::Claude, Some(3), "c", "", false, true)
    };
    assert_eq!(
        since(&advance_since(&BTreeMap::new(), &[ohne], t0), "claude:c"),
        None,
        "unknown -> none, not guessed"
    );
    let wacht = Session {
        watching: true,
        digest: Some(Digest {
            answer_at: Some(t0 - 120.0),
            prompt_at: Some(t0 - 900.0),
            ..Digest::default()
        }),
        ..Session::new("claude:w", Engine::Claude, Some(4), "w", "", false, true)
    };
    assert_eq!(
        advance_since(&BTreeMap::new(), &[wacht], t0).get("claude:w"),
        Some(&SinceEntry {
            tone: LabelTone::Watching,
            since: Some(t0 - 120.0)
        }),
        "watching since the last answer"
    );
}

/// Kevin 2026-09-26 (label B): spelled out, so "FERTIG  seit 6 Min." reads like a sentence.
#[test]
fn test_duration_text() {
    assert_eq!(duration_text(30.0), "gerade eben");
    assert_eq!(duration_text(6.0 * 60.0 + 20.0), "6 Min.");
    assert_eq!(duration_text(3600.0 + 5.0 * 60.0), "1 Std. 5 Min.");
    assert_eq!(
        duration_text(2.0 * 3600.0 + 20.0),
        "2 Std.",
        "a full hour without '0 Min.'"
    );
    assert_eq!(
        duration_text(-4.0),
        "gerade eben",
        "clock twitch: never negative"
    );
}

#[test]
fn test_first_sentence() {
    assert_eq!(
        first_sentence(
            "Kevin, die Übersichtsseite ist jetzt in deinem Browser offen. Details folgen."
        ),
        "Die Übersichtsseite ist jetzt in deinem Browser offen.",
        "greeting gone, first sentence"
    );
    assert_eq!(
        first_sentence("Kevin 👋 **Die Währungsfrage ist geklärt: in Euro buchen.** Damit gilt …"),
        "Die Währungsfrage ist geklärt: in Euro buchen.",
        "emoji + markdown gone"
    );
    assert_eq!(
        first_sentence("Kevins Plan steht."),
        "Kevins Plan steht.",
        "only the greeting, not 'Kevins'"
    );
    assert_eq!(
        first_sentence("Für 2.1 gibt es noch keine bessere Version"),
        "Für 2.1 gibt es noch keine bessere Version",
        "a dot in a number ends no sentence"
    );
    assert_eq!(
        first_sentence("## Ergebnis\n- alles grün"),
        "Ergebnis alles grün",
        "heading and bullet smoothed"
    );
    assert_eq!(
        first_sentence(&"a".repeat(300)).chars().count(),
        160,
        "cut hard"
    );
    // Kevin 2026-09-27 (screenshot): "12 is running." — a "#" before a number is a number, no heading.
    assert_eq!(
        first_sentence("#12 is running. Its window opened at 23:27."),
        "#12 is running.",
        "a number at the line start stays"
    );
    assert_eq!(
        first_sentence("Kevin, #12 is running. Its window opened."),
        "#12 is running.",
        "a number after the greeting stays"
    );
    assert_eq!(
        first_sentence("-5 Grad am Morgen. Mehr folgt."),
        "-5 Grad am Morgen.",
        "a minus before a number is no bullet"
    );
}

/// Swift counts and cuts by `Character`: a flag, a ZWJ family and an emoji with variation selector are one each.
#[test]
fn first_sentence_counts_characters_like_swift() {
    let flags = "🇨🇭".repeat(170);
    let cut = first_sentence(&flags);
    assert!(
        cut.ends_with('…') && cut.trim_end_matches('…') == "🇨🇭".repeat(159),
        "159 whole flags and the ellipsis"
    );
    let family = "👨\u{200D}👩\u{200D}👧";
    assert_eq!(
        first_sentence(&format!("{} ok.", family.repeat(100))),
        format!("{} ok.", family.repeat(100)),
        "104 characters stay whole (Rust counts 504 chars)"
    );
    let characters = |s: &'static str| crate::model::graphemes(s).collect::<Vec<_>>();
    assert_eq!(
        characters("✳\u{FE0F}é\u{301}a"),
        ["✳\u{FE0F}", "é\u{301}", "a"]
    );
    assert_eq!(
        characters("🇨🇭🇩🇪🇨\u{301}🇭"),
        ["🇨🇭", "🇩🇪", "🇨\u{301}", "🇭"],
        "flags pair up only over regional indicators in a row"
    );
}

#[test]
fn test_label_content() {
    let now = 20_000.0;
    let mut s = Session {
        digest: Some(Digest {
            name: some("Build queue cards"),
            step: some("Tests der Karten laufen lassen"),
            text: some("Kevin, Zwischenstand: die Hälfte steht."),
            last: LastBlock::Tool,
            ..Digest::default()
        }),
        ..Session::new(
            "claude:e5",
            Engine::Claude,
            Some(1),
            "beta-e5",
            "/Users/test/Projects/Beta/.claude/worktrees/shop+feature-pr1",
            false,
            true,
        )
    };
    let running = SinceEntry {
        tone: LabelTone::Running,
        since: Some(now - 22.0 * 60.0),
    };
    let w = tab_window(vec![s.clone()], "◐ Build queue cards");
    let laeuft = label_content(&w, Some(&running), None, now);
    // Kevin 2026-09-26 (label B): plain status, time always with "seit", reads like a sentence. Kevin 2026-10-12
    // evening: the step line while working is back (the calm variant of the midday was taken back).
    assert_eq!(
        laeuft.lines(),
        [
            "● ARBEITET  seit 22 Min.",
            "Build queue cards",
            "jetzt: Tests der Karten laufen lassen"
        ]
    );
    let zeile = session_line(
        &w,
        &BTreeMap::from([(s.id.clone(), running)]),
        &NO_LINES,
        now,
    );
    assert_eq!(
        zeile.title, "● Build queue cards — seit 22 Min.",
        "menu and list line with the same time"
    );
    s.digest.as_mut().unwrap().last = LastBlock::Text;
    let w = tab_window(vec![s.clone()], "◐ Build queue cards");
    assert_eq!(
        label_content(&w, None, None, now).detail.as_deref(),
        Some("Zwischenstand: die Hälfte steht."),
        "writing text right now: its first sentence"
    );

    let f = Session {
        digest: Some(Digest {
            name: some("Rechnungen prüfen"),
            text: some("Kevin, die Übersichtsseite ist jetzt in deinem Browser offen. Mehr folgt."),
            last: LastBlock::Text,
            ..Digest::default()
        }),
        ..Session::new(
            "claude:14",
            Engine::Claude,
            Some(2),
            "alpha-14",
            "/Users/test/Projects/Alpha",
            true,
            true,
        )
    };
    let done = SinceEntry {
        tone: LabelTone::Ready,
        since: Some(now - 6.0 * 60.0),
    };
    let fertig = label_content(
        &tab_window(vec![f.clone()], "✳ Rechnungen prüfen"),
        Some(&done),
        None,
        now,
    );
    assert_eq!(
        fertig.lines(),
        [
            "✓ FERTIG  seit 6 Min.",
            "Rechnungen prüfen",
            "Die Übersichtsseite ist jetzt in deinem Browser offen."
        ]
    );

    let mut q = Session {
        waiting: true,
        digest: Some(Digest {
            title: some("Handoff spawn 2026-09-21"),
            step: some("Frage: in Euro buchen?"),
            question: some("in Euro buchen?"),
            ..Digest::default()
        }),
        ..Session::new(
            "claude:f8",
            Engine::Claude,
            Some(3),
            "gamma-f8",
            "/Users/test/Projects/Gamma",
            true,
            true,
        )
    };
    let asked = SinceEntry {
        tone: LabelTone::Waiting,
        since: Some(now - 30.0),
    };
    let frage = label_content(
        &tab_window(vec![q.clone()], "✳ Handoff spawn 2026-09-21"),
        Some(&asked),
        None,
        now,
    );
    // "wartet auf dich" instead of "Frage an dich": also covers approving a step (below). Without an own name never
    // Claude's topic, only the folder (Kevin 2026-10-12: the topic keeps changing, "Ja mach das · ClaudeCode").
    assert_eq!(
        frage.lines(),
        [
            "? WARTET AUF DICH  gerade eben",
            "Gamma",
            "„in Euro buchen?“"
        ]
    );
    q.digest = Some(Digest {
        title: some("Handoff spawn 2026-09-21"),
        step: some("Entwurf anlegen"),
        ..Digest::default()
    });
    assert_eq!(
        label_content(
            &tab_window(vec![q.clone()], "✳ Handoff spawn 2026-09-21"),
            None,
            None,
            now
        )
        .detail
        .as_deref(),
        Some("Freigabe: Entwurf anlegen"),
        "a question without AskUserQuestion = approval of the step"
    );

    let codex = Session::new(
        "codex:1",
        Engine::Codex,
        Some(4),
        "Alpha",
        "/Users/test/Projects/Alpha",
        false,
        true,
    );
    let c = label_content(&tab_window(vec![codex.clone()], "Alpha"), None, None, now);
    assert_eq!(
        c.lines(),
        ["● ARBEITET", "Alpha"],
        "without transcript: status + window title, nothing twice"
    );
    let other = SinceEntry {
        tone: LabelTone::Ready,
        since: Some(now),
    };
    assert_eq!(
        label_content(&tab_window(vec![codex], "Alpha"), Some(&other), None, now).duration,
        None,
        "the duration of another state is not shown"
    );

    let mut alt = q.clone();
    alt.digest = Some(Digest {
        title: some("Alte Aufgabe"),
        ..Digest::default()
    });
    assert_eq!(
        label_content(
            &tab_window(vec![alt, s.clone()], "◐ Build queue cards"),
            None,
            None,
            now
        )
        .hint
        .as_deref(),
        Some("Tab dahinter hat eine Frage")
    );
    // Visible tab not recognisable (topic not in the transcript yet): topic from the title, but no step, it might belong
    // to the hidden tab.
    let unklar = label_content(&tab_window(vec![f, s], "◐ Neues Thema"), None, None, now);
    assert!(
        unklar.topic == "Neues Thema" && unklar.detail.is_none(),
        "got {:?}",
        unklar.lines()
    );
}

#[test]
fn test_label_targets() {
    let t0 = 1000.0;
    let mut aktiv = fake_window(true, t0);
    let mut laeuft = fake_window(false, t0 + 1.0);
    let mut voll = fake_window(false, t0 + 2.0);
    let mut shell = fake_window_with(false, t0 + 3.0, None, false, false);
    let mut winzig = fake_window(false, t0 + 4.0);
    aktiv.frame = Rect::new(0.0, 0.0, 1248.0, 999.0);
    laeuft.frame = Rect::new(1248.0, 884.0, 480.0, 200.0);
    voll.frame = MACBOOK;
    shell.frame = Rect::new(1248.0, 684.0, 480.0, 200.0);
    winzig.frame = Rect::new(1248.0, 84.0, 200.0, 90.0);
    let windows = [aktiv, laeuft.clone(), voll.clone(), shell, winzig];
    let o = OverlayInput {
        focused: Some(0),
        ..OverlayInput::new(&windows, None, 1117.0, &[MACBOOK])
    };
    let ziele = label_targets(&o, &NO_SINCE, &NO_LINES, t0);
    // Kimi 2026-09-25: "voll" is the only tile on the MacBook, Lookout itself made it that large; it needs its label. The
    // maximized rule is only for the frame (Kevin 2026-09-18).
    assert_eq!(
        target_ids(&ziele, |t| &t.id),
        [laeuft.sessions[0].id.as_str(), voll.sessions[0].id.as_str()],
        "labels also on running and fully tiled windows, not on the active one, the shell, the tiny one"
    );
    let r = ziele[0].rect;
    assert!(
        r.max_y() == laeuft.frame.max_y() - 40.0 - 2.0
            && r.min_x() > laeuft.frame.min_x()
            && r.max_x() < laeuft.frame.max_x(),
        "right below Warp's tab bar, inside the window"
    );
    // Kevin 2026-09-26 (proposal B, "größer und klarer"): status + name 16 pt on 22-pt lines, step 13.5 pt on 18 pt,
    // padding 9: two lines 62 pt high, three 80 pt (before all 14 pt: 54 and 73 pt).
    assert_eq!(
        r.height(),
        9.0 * 2.0 + 22.0 + 22.0,
        "two lines: status + name"
    );
    assert_eq!(
        label_rect(laeuft.frame, WARP_BID, 3).height(),
        9.0 * 2.0 + 22.0 + 22.0 + 18.0,
        "the step line smaller than status and name"
    );
    assert!(
        [0, 1, 2, 3].map(|i| label_line_style(i).size) == [16.0, 16.0, 13.5, 13.5]
            && label_line_style(0).weight == FontWeight::Bold,
        "status bold, name large, step and hint smaller"
    );
    // Contrast recalculated 2026-09-26: at 58 % over light terminal text only 2.8:1, from 92 % at least 9.5:1.
    const { assert!(LABEL_BG_ALPHA >= 0.92, "background nearly opaque") };
}

/// Menu rows (design A, Kevin 2026-11-06 evening): the name in front, project and state below it, the time apart.
#[test]
fn test_menu_row() {
    let now = 50_000.0;
    let fragt = Session {
        waiting: true,
        digest: Some(Digest {
            name: some("Acme · #8 Rules page build"),
            question: some("Mergen?"),
            ..Digest::default()
        }),
        ..Session::new(
            "claude:m1",
            Engine::Claude,
            Some(1),
            "acme-m1",
            "/Users/test/AI/Acme",
            true,
            true,
        )
    };
    let seit = SinceEntry {
        tone: LabelTone::Waiting,
        since: Some(now - 10.0 * 60.0),
    };
    let since = BTreeMap::from([(fragt.id.clone(), seit)]);
    let row = menu_row(&tab_window(vec![fragt], "✳ Rules"), &since, &NO_LINES, now);
    assert_eq!(
        (
            row.name.as_str(),
            row.project.as_deref(),
            row.duration.as_deref(),
            row.tone
        ),
        (
            "#8 Rules page build",
            Some("Acme"),
            Some("10 Min."),
            LabelTone::Waiting
        ),
        "the task in front, the project apart, the time without „seit“"
    );
    assert_eq!(
        row.subtitle(&["minimiert (im Dock)".to_owned()]).as_deref(),
        Some("Acme · minimiert (im Dock) · „Mergen?“")
    );
    assert_eq!(
        row.project_line(&["Arbeiter".to_owned()]).as_deref(),
        Some("Acme · Arbeiter"),
        "submenus: project and note, the dot shows the state"
    );
    assert_eq!(row.compact(), "#8 Rules page build · Acme");

    // Without an own name the folder is the name; a watching one says so below it (it stands with the working ones).
    let wacht = Session {
        watching: true,
        digest: Some(Digest {
            text: some("Der Lauf ist gestartet. Ich melde mich."),
            last: LastBlock::Text,
            ..Digest::default()
        }),
        ..Session::new(
            "claude:m2",
            Engine::Claude,
            Some(2),
            "lookout-m2",
            "/Users/test/AI/Lookout",
            false,
            true,
        )
    };
    let tab = Session::new(
        "claude:m3",
        Engine::Claude,
        None,
        "lookout-m3",
        "/Users/test/AI/Lookout",
        true,
        false,
    );
    let row = menu_row(&tab_window(vec![wacht, tab], ""), &NO_SINCE, &NO_LINES, now);
    assert_eq!(
        (
            row.name.as_str(),
            row.project.as_deref(),
            row.duration.as_deref()
        ),
        ("Lookout  (+1 Tab)", None, None)
    );
    assert_eq!(
        row.subtitle(&[]).as_deref(),
        Some("überwacht · Der Lauf ist gestartet.")
    );
    assert_eq!(row.compact(), "Lookout  (+1 Tab)");

    // AppKit wraps a subtitle at about 290 pt whatever the menu's width: one line, cut with an ellipsis.
    let lang = Session {
        digest: Some(Digest {
            name: some("Studio · Team-Lead X und Labor"),
            step: some("Labor fragt nach dem Test-Konto und dem Wochenplan"),
            ..Digest::default()
        }),
        ..Session::new(
            "claude:m4",
            Engine::Claude,
            Some(4),
            "ao-m4",
            "/Users/test/AI/Studio",
            false,
            true,
        )
    };
    let sub = menu_row(&tab_window(vec![lang], ""), &NO_SINCE, &NO_LINES, now)
        .subtitle(&[])
        .unwrap();
    assert!(
        sub.starts_with("Studio · jetzt: Labor fragt")
            && sub.ends_with('…')
            && graphemes(&sub).count() <= MENU_SUBTITLE_LIMIT,
        "{sub}"
    );
    // Chinese or Japanese letters are about twice as wide: half as many fit (review quick 2026-11-06).
    let wide = MenuRow {
        name: "x".into(),
        project: None,
        state: vec!["汉".repeat(40)],
        duration: None,
        tone: LabelTone::Running,
    };
    assert_eq!(
        wide.subtitle(&[]).as_deref(),
        Some(format!("{}…", "汉".repeat(22)).as_str())
    );
}

/// Sections of the menu (design A): fixed order, empty ones left out, the count in the head (AppKit draws no badge on
/// a section header, measured 2026-11-06).
#[test]
fn test_menu_groups() {
    let fragt = fake_window_with(true, 1.0, None, true, true);
    let fertig = fake_window(true, 2.0);
    let laeuft = fake_window(false, 3.0);
    let mut wacht = fake_window(false, 4.0);
    wacht.sessions[0].watching = true;
    let arbeiter = fake_window(false, 5.0);
    let windows = [fragt, fertig, laeuft, wacht, arbeiter];
    assert_eq!(
        menu_groups(&windows, &[0, 1, 2, 3, 4], &BTreeSet::from([4])),
        vec![
            (MenuGroup::Waiting, vec![0]),
            (MenuGroup::Ready, vec![1]),
            (MenuGroup::Working, vec![2, 3]),
            (MenuGroup::Lot, vec![4]),
        ],
        "running and watching ones work together, the parking lot stands apart"
    );
    assert_eq!(
        menu_groups(&windows, &[2, 1], &BTreeSet::new()),
        vec![(MenuGroup::Ready, vec![1]), (MenuGroup::Working, vec![2])],
        "fixed order of the sections, empty ones left out"
    );
    assert_eq!(MenuGroup::Waiting.header(3), "WARTET AUF DICH · 3");
    assert_eq!(MenuGroup::Working.header(4), "ARBEITET · 4");
    assert_eq!(MenuGroup::Lot.header(7), "AUF DEM PARKPLATZ · 7");
}

/// The parking lot stays compact: a single rest row would cost the same line as the hint.
#[test]
fn test_rows_shown() {
    assert_eq!(
        [0, 3, 4, 5, 9].map(|n| rows_shown(n, MENU_LOT_ROWS)),
        [0, 3, 4, 3, 3]
    );
    assert_eq!(rows_shown(25, MENU_ROWS_MAX), MENU_ROWS_MAX);
    assert_eq!(more_rows_text(4), "… und 4 weitere");
}

// ---------- v4: saubere Namen, Fenster am Namen, verworfene Klicks (design 2026-09-21) ----------

#[test]
fn test_name_on_label_and_tab() {
    let s = Session {
        digest: Some(Digest {
            title: some("Handoff spawn 2026-09-21"),
            name: some("Gamma · Buchhaltung weiterführen"),
            step: some("liest die Doku"),
            last: LastBlock::Tool,
            ..Digest::default()
        }),
        ..Session::new(
            "claude:f8",
            Engine::Claude,
            Some(3),
            "gamma-f8",
            "/Users/test/Projects/Gamma",
            false,
            true,
        )
    };
    let c = label_content(
        &tab_window(vec![s.clone()], "◐ Gamma · Buchhaltung weiterführen"),
        None,
        None,
        0.0,
    );
    assert_eq!(
        c.topic, "Gamma · Buchhaltung weiterführen",
        "name instead of topic, no folder after it"
    );
    let alt = tab_session("alt", true, false, Some("Alte Aufgabe"));
    let w = tab_window(
        vec![alt.clone(), s.clone()],
        "◐ Gamma · Buchhaltung weiterführen",
    );
    assert!(
        w.visible_session().map(|v| &v.id) == Some(&s.id) && !w.is_ready(),
        "visible tab recognised by the own name"
    );
    let uebergang = tab_window(vec![alt, s.clone()], "◐ Handoff spawn 2026-09-21");
    assert_eq!(
        uebergang.visible_session().map(|v| &v.id),
        Some(&s.id),
        "transition: the title still shows the topic"
    );
}

/// Kevin 2026-09-21: clicks during Lookout's own moves were lost, now they are noted.
#[test]
fn test_move_gate() {
    let mut g = MoveGate::default();
    assert!(g.event(false), "without a move handle it at once");
    assert!(!g.event(true) && g.pending(), "during the move note it");
    assert!(
        g.move_ended() && !g.pending(),
        "after the move handle it once"
    );
    assert!(!g.move_ended(), "nothing noted -> nothing to handle");
}

/// Live 2026-09-25 after the rebuild: Chrome was in front, Lookout knew no active session, some window stood with a mint
/// frame in the middle until the user clicked a terminal. At start the last focused window of a terminal that is not in
/// front counts too.
#[test]
fn test_focus_source_at_start() {
    assert_eq!(
        focus_source(Some(5), &[5, 6], &[6], true),
        Some(5),
        "a terminal in front: its focus counts"
    );
    assert_eq!(
        focus_source(Some(9), &[5, 6], &[6], true),
        None,
        "another app in front, active session known: change nothing"
    );
    assert_eq!(
        focus_source(Some(9), &[5, 6], &[6], false),
        Some(6),
        "start with Chrome in front: last focused window of the AI terminal"
    );
    assert_eq!(
        focus_source(None, &[], &[], false),
        None,
        "without a terminal nothing"
    );
}

// ---------- Plan 1e: watching ----------

/// Kevin 2026-09-27 (screenshots of a lead session): "er hat ja nichts zu tun, er überwacht jetzt nur die Sessions":
/// grey, no frame, with the running ones; the third line says what it waits for.
#[test]
fn test_watching_label() {
    let now = 30_000.0;
    let s = Session {
        watching: true,
        digest: Some(Digest {
            name: some("Beta · Lead"),
            text: some("Nothing needed from you, Kevin. CI is running."),
            last: LastBlock::Text,
            ..Digest::default()
        }),
        ..Session::new(
            "claude:lead",
            Engine::Claude,
            Some(1),
            "beta-lead",
            "/Users/test/Projects/Beta",
            false,
            true,
        )
    };
    let w = tab_window(vec![s.clone()], "✳ Beta · Lead");
    assert!(
        !w.is_ready() && !w.is_waiting() && w.is_watching(),
        "watching: no frame, no question, not with the waiting ones"
    );
    assert!(window_tone(&w) == LabelTone::Watching && session_tone(&s) == LabelTone::Watching);
    let seit = SinceEntry {
        tone: LabelTone::Watching,
        since: Some(now - 8.0 * 60.0),
    };
    let c = label_content(&w, Some(&seit), None, now);
    assert_eq!(
        c.lines(),
        [
            "◌ ÜBERWACHT  seit 8 Min.",
            "Beta · Lead",
            "Nothing needed from you, Kevin."
        ]
    );
    assert_eq!(
        session_line(&w, &BTreeMap::from([(s.id.clone(), seit)]), &NO_LINES, now).title,
        "◌ Beta · Lead — seit 8 Min.",
        "menu and list with the same symbol"
    );
    let beides = Session {
        watching: true,
        ..Session::new("claude:x", Engine::Claude, Some(2), "x", "", true, true)
    };
    assert_eq!(
        window_tone(&tab_window(vec![beides], "✳ X")),
        LabelTone::Ready,
        "done goes first"
    );
    // review quick 2026-09-27 (GLM): two tabs, the visible one not recognisable, title ✳; no session done, one watches:
    // the calm visible tab is the watching one, no frame. If one is done, the old rule stays.
    let aufpasser = Session {
        watching: true,
        ..tab_session("w1", false, false, Some("Thema W"))
    };
    let zwei_tabs = tab_window(
        vec![
            aufpasser.clone(),
            tab_session("l1", false, false, Some("Thema L")),
        ],
        "✳ Neues Thema",
    );
    assert!(
        zwei_tabs.visible_session().is_none() && !zwei_tabs.is_ready() && zwei_tabs.is_watching(),
        "✳ without done ones: watching"
    );
    let mit_fertig = tab_window(
        vec![aufpasser, tab_session("f1", true, false, Some("Thema F"))],
        "✳ Neues Thema",
    );
    assert!(
        mit_fertig.is_ready() && !mit_fertig.is_watching(),
        "✳ with a done one: done as before"
    );
}

// ---------- Plan 1d: parked windows ----------

/// GP 2: parked windows never get frame or label, not even a Ghostty window behind a Warp main window (being congruent
/// only holds within the same app).
#[test]
fn test_parked_windows_have_no_overlay() {
    let t0 = 1000.0;
    let r = Rect::new(687.0, -1037.0, 1248.0, 1005.0);
    let mitte = TermWindow {
        frame: r,
        ..fake_window(false, t0)
    }; // Warp, pid 0
    let geparkt = TermWindow {
        id: 7,
        app: "Ghostty".into(),
        bundle_id: GHOSTTY_BID.into(),
        owner_pid: 7,
        frame: r,
        sessions: vec![Session::new(
            "claude:g",
            Engine::Claude,
            Some(70),
            "g",
            "",
            true,
            true,
        )],
        ready_since: Some(t0),
        ..fake_window(false, t0)
    };
    let ax = cocoa_to_ax(r, AxOrigin::TopLeft, 1239.0);
    let list = [
        OnScreenWindow {
            number: 1,
            pid: 0,
            rect: ax,
        },
        OnScreenWindow {
            number: 2,
            pid: 7,
            rect: ax,
        },
    ];
    let windows = [mitte, geparkt];
    let o = OverlayInput::new(&windows, Some(&list), 1239.0, &[MACBOOK]);
    assert_eq!(
        glow_targets(&o).len(),
        1,
        "without parking the Ghostty window would get a frame"
    );
    let parked = BTreeSet::from([1]);
    let o = OverlayInput {
        parked: &parked,
        ..o
    };
    assert!(glow_targets(&o).is_empty(), "parked: no frame");
    let labels = label_targets(&o, &NO_SINCE, &NO_LINES, t0);
    assert_eq!(
        target_ids(&labels, |l| &l.id),
        [windows[0].sessions[0].id.as_str()],
        "parked: no label"
    );
}

/// Parking lot (Kevin 2026-11-06): a running worker parked behind the main window gets neither frame nor label, like
/// any parked window. While the lot is peeked at, it stands tiled and is a window like any other.
#[test]
fn parking_lot_windows_have_no_overlay_until_peeked() {
    let mut w = vec![
        fake_window(false, 1000.0),
        fake_window(false, 1001.0),
        fake_window(true, 1002.0),
    ];
    w[1].sessions[0].name = "Shop · Worker 1".into();
    let workers = vec!["Worker".to_owned()];
    for (peek, labelled) in [(false, [2].as_slice()), (true, [1, 2].as_slice())] {
        let a = plan_arrangement(&PlanInput {
            focused: Some(0),
            worker_names: &workers,
            peek,
            ..input(Reading, Single(0), &w, Right)
        });
        assert!(
            a.lot.contains(&1) && a.parked.contains(&1) != peek,
            "fixture: the worker is in the lot, parked unless peeked at"
        );
        let mut placed = w.clone();
        for (&i, &r) in &a.frames {
            placed[i].frame = r;
        }
        let o = OverlayInput {
            focused: Some(0),
            parked: &a.parked,
            ..OverlayInput::new(&placed, None, 1239.0, &BOTH)
        };
        let want: Vec<&str> = labelled
            .iter()
            .map(|&i| w[i].sessions[0].id.as_str())
            .collect();
        let labels = label_targets(&o, &NO_SINCE, &NO_LINES, 1002.0);
        assert_eq!(target_ids(&labels, |l| &l.id), want, "peek {peek}: labels");
        assert_eq!(
            target_ids(&glow_targets(&o), |g| &g.id),
            [w[2].sessions[0].id.as_str()],
            "peek {peek}: only the done window glows"
        );
    }
}

/// R2-1: parked windows lie congruent behind the main window, `locate_on_screen` is ambiguous. The list takes the front
/// window number of this app on the main frame (the window list comes front to back).
#[test]
fn test_list_finds_main_window_number() {
    let mitte = TermWindow {
        frame: Rect::new(687.0, -1037.0, 1248.0, 1005.0),
        ..fake_window(false, 1000.0)
    }; // pid 0
    let ax = cocoa_to_ax(mitte.frame, AxOrigin::TopLeft, 1239.0);
    let on = |number, pid| OnScreenWindow {
        number,
        pid,
        rect: ax,
    };
    let stapel = [on(71, 0), on(72, 0), on(73, 0)];
    assert_eq!(
        locate_on_screen(&mitte, Some(&stapel), 1239.0),
        OnScreenMatch::Ambiguous,
        "congruent = ambiguous"
    );
    assert_eq!(
        frontmost_number(&mitte, &stapel, 1239.0),
        Some(71),
        "the front one is the main window after raising"
    );
    assert_eq!(
        frontmost_number(
            &mitte,
            &[on(80, 9), on(71, 0), on(72, 0), on(73, 0)],
            1239.0
        ),
        Some(71),
        "only windows of the same app"
    );
    assert_eq!(frontmost_number(&mitte, &[], 1239.0), None);
}

// ---------- Phase 2: teilbar (language, settings, greeting) ----------

/// Greeting (GP 9): the macOS user's first name at the start, and the next non-blank character is neither letter nor
/// digit. "Name, then a punctuation mark" would leave "Kevin 👋" standing.
#[test]
fn test_greeting_name() {
    assert!(
        first_name("Kevin Theobald") == "Kevin"
            && first_name("  Ada  Lovelace ") == "Ada"
            && first_name("").is_empty()
    );
    assert_eq!(
        first_sentence("Kevin, alles gut. Mehr folgt."),
        "Alles gut.",
        "comma"
    );
    assert_eq!(
        first_sentence("Kevin 👋 Alles gut. Mehr folgt."),
        "Alles gut.",
        "emoji"
    );
    assert_eq!(
        first_sentence("kevin, alles gut."),
        "Alles gut.",
        "case does not matter"
    );
    assert_eq!(
        first_sentence("Kevins Plan steht."),
        "Kevins Plan steht.",
        "a letter after it: no greeting"
    );
    let saved = greeting_name();
    set_greeting_name("Will");
    assert_eq!(
        first_sentence("Will this work? Maybe."),
        "Will this work?",
        "name Will, but a word after it"
    );
    assert_eq!(first_sentence("Will, it works. More."), "It works.");
    assert_eq!(
        first_sentence("Kevin, alles gut."),
        "Kevin, alles gut.",
        "another name stays"
    );
    set_greeting_name("");
    assert_eq!(
        first_sentence("Kevin, alles gut."),
        "Kevin, alles gut.",
        "without a name Lookout removes nothing"
    );
    set_greeting_name(&saved);
}

/// English interface: the asserts of testEnglishTexts on this module (the first four are `layout::tests`, the three
/// `stepLabel` ones belong to the status logic).
#[test]
fn test_english_texts_overlay() {
    set_german(false);
    assert_eq!(
        [30.0, 380.0, 3900.0, 7220.0].map(duration_text),
        ["just now", "6 min", "1 h 5 min", "2 h"]
    );
    assert_eq!(MenuGroup::Waiting.header(2), "WAITING FOR YOU · 2");
    assert_eq!(MenuGroup::Lot.header(1), "IN THE PARKING LOT · 1");
    assert_eq!(more_rows_text(4), "… and 4 more");
    assert!(
        WindowWhereabouts::Minimized.label() == "minimized (in the Dock)"
            && WindowWhereabouts::OtherSpace.label() == "other desktop"
    );

    let now = 20_000.0;
    let s = Session {
        digest: Some(Digest {
            name: some("Build queue cards"),
            step: some("Run the card tests"),
            last: LastBlock::Tool,
            ..Digest::default()
        }),
        ..Session::new(
            "claude:e1",
            Engine::Claude,
            Some(1),
            "alpha-e1",
            "/Users/test/Projects/Alpha",
            false,
            true,
        )
    };
    let w = tab_window(vec![s.clone()], "◐ Build queue cards");
    let seit = SinceEntry {
        tone: LabelTone::Running,
        since: Some(now - 22.0 * 60.0),
    };
    assert_eq!(
        label_content(&w, Some(&seit), None, now).lines(),
        [
            "● WORKING  for 22 min",
            "Build queue cards",
            "now: Run the card tests"
        ]
    );
    assert_eq!(
        session_line(&w, &BTreeMap::from([(s.id.clone(), seit)]), &NO_LINES, now).title,
        "● Build queue cards — for 22 min"
    );
    let mut q = Session {
        waiting: true,
        digest: Some(Digest {
            name: some("Release notes"),
            step: some("Question: Ship it?"),
            question: some("Ship it?"),
            ..Digest::default()
        }),
        ..Session::new("claude:q1", Engine::Claude, Some(2), "q1", "", true, true)
    };
    let frage = label_content(
        &tab_window(vec![q.clone()], "✳ Release notes"),
        None,
        None,
        now,
    );
    assert_eq!(
        frage.lines(),
        ["? WAITING FOR YOU", "Release notes", "“Ship it?”"],
        "got {:?}",
        frage.lines()
    );
    q.digest = Some(Digest {
        name: some("Release notes"),
        step: some("Upload the build"),
        ..Digest::default()
    });
    assert_eq!(
        label_content(
            &tab_window(vec![q.clone()], "✳ Release notes"),
            None,
            None,
            now
        )
        .detail
        .as_deref(),
        Some("Approval: Upload the build")
    );
    let mut alt = q;
    alt.digest = Some(Digest {
        title: some("Old task"),
        ..Digest::default()
    });
    assert_eq!(
        label_content(
            &tab_window(vec![alt, s], "◐ Build queue cards"),
            None,
            None,
            now
        )
        .hint
        .as_deref(),
        Some("A hidden tab has a question")
    );
}

// ---------- New: the Lookout line, list and preview content ----------

/// The session's own Lookout line (Kevin 2026-10-12) goes before the first sentence and before "Freigabe: <step>", after
/// an open question; without a digest there is none (Swift `d?.line`). Swift checked this at the end of testLookoutLine
/// with the status logic's parser; here the line comes in as data.
#[test]
fn lookout_line_goes_before_the_first_sentence() {
    let done = LookoutLine {
        status: LineStatus::Done,
        reason: "12 Rechnungen".into(),
    };
    let f = Session {
        digest: Some(Digest {
            title: some("Rechnungen"),
            text: some("Kevin, alles gebucht. Mehr folgt.\n✓ Lookout: fertig (12 Rechnungen)"),
            last: LastBlock::Text,
            ..Digest::default()
        }),
        ..Session::new(
            "claude:l",
            Engine::Claude,
            Some(2),
            "alpha-l",
            "/Users/test/Projects/Alpha",
            true,
            true,
        )
    };
    let w = tab_window(vec![f.clone()], "✳ Rechnungen");
    assert_eq!(
        label_content(&w, None, Some(&done), 0.0).detail.as_deref(),
        Some("12 Rechnungen")
    );
    let lines = BTreeMap::from([(f.id.clone(), done)]);
    assert_eq!(
        session_line(&w, &NO_SINCE, &lines, 0.0).subtitle.as_deref(),
        Some("12 Rechnungen"),
        "menu and list follow the label"
    );
    let bitte = LookoutLine {
        status: LineStatus::Ask,
        reason: "Merge #126".into(),
    };
    let mut wartet = Session {
        waiting: true,
        digest: Some(Digest {
            step: some("deploy"),
            text: some("Bereit."),
            ..Digest::default()
        }),
        ..Session::new("claude:s", Engine::Claude, Some(3), "s", "", true, true)
    };
    assert_eq!(
        label_content(
            &tab_window(vec![wartet.clone()], "✳ Shop"),
            None,
            Some(&bitte),
            0.0
        )
        .detail
        .as_deref(),
        Some("Merge #126"),
        "the request instead of 'Freigabe: <step>'"
    );
    wartet.digest.as_mut().unwrap().question = some("Welche Variante?");
    assert_eq!(
        label_content(&tab_window(vec![wartet], "✳ Shop"), None, Some(&bitte), 0.0)
            .detail
            .as_deref(),
        Some("„Welche Variante?“"),
        "an open question goes first"
    );
    let ohne = tab_window(
        vec![Session::new(
            "claude:o",
            Engine::Claude,
            Some(4),
            "o",
            "/w/Ohne",
            true,
            true,
        )],
        "✳ Ohne",
    );
    assert_eq!(
        label_content(&ohne, None, Some(&bitte), 0.0).detail,
        None,
        "no digest, no line"
    );
}

/// List panel content (Main.swift `refreshLists` + `ListView`) and the preview outlines of a planned arrangement.
#[test]
fn list_and_preview_content() {
    let mut windows = vec![
        fake_window(false, 1000.0),
        fake_window(true, 1001.0),
        fake_window(false, 1002.0),
    ];
    for (i, w) in windows.iter_mut().enumerate() {
        w.id = 10 + i as u64;
    }
    windows[1].sessions.push(Session::new(
        "claude:tab2",
        Engine::Claude,
        Some(5),
        "tab2",
        "",
        false,
        true,
    ));
    let list = SideList {
        rect: Rect::new(0.0, 0.0, 400.0, 120.0),
        entries: vec![
            ListEntry::Header("ARBEITET (2)".into()),
            ListEntry::Row(1),
            ListEntry::Row(2),
            ListEntry::More(3),
        ],
        inbox: false,
    };
    // The plan saw windows 10, 11, 12; since then 12 is gone and 11 stands first.
    let now_windows = [windows[1].clone(), windows[0].clone()];
    let t = list_target(
        &list,
        &[10, 11, 12],
        &now_windows,
        &NO_SINCE,
        &NO_LINES,
        0.0,
        71,
    );
    let row = ListLine::Row {
        ids: vec![windows[1].sessions[0].id.clone(), "claude:tab2".into()],
        title: "✓ claude  (+1 Tab)".into(),
        detail: None,
        tone: LabelTone::Ready,
    };
    assert_eq!(
        t,
        ListTarget {
            rect: list.rect,
            lines: vec![
                ListLine::Header("ARBEITET (2)".into()),
                row,
                ListLine::More(3)
            ],
            window_number: 71,
        },
        "a row whose window is gone is left out"
    );
    assert_eq!(row_at(&t.lines, 10.0), None, "header");
    assert_eq!(
        row_at(&t.lines, 28.0).map(<[String]>::len),
        Some(2),
        "the row starts below the header"
    );
    assert_eq!(row_at(&t.lines, 74.0), None, "'+3 more' takes no click");
    assert_eq!(more_text(3), "+3 weitere (im Menü)");

    let (r0, r1, list_rect) = (
        Rect::new(0.0, 0.0, 800.0, 600.0),
        Rect::new(800.0, 0.0, 400.0, 600.0),
        Rect::new(800.0, 600.0, 400.0, 120.0),
    );
    let a = Arrangement {
        frames: BTreeMap::from([(0, r0), (1, r1), (2, r0)]),
        parked: BTreeSet::from([2]),
        lists: vec![SideList {
            rect: list_rect,
            entries: vec![ListEntry::Row(2)],
            inbox: false,
        }],
        main: Some(0),
        lot: BTreeSet::new(),
    };
    let preview = |rect, title: &str, detail: &str| PreviewBox {
        rect,
        title: title.into(),
        detail: detail.into(),
    };
    assert_eq!(
        preview_boxes(&a, &windows, Some(0)),
        [
            preview(r0, "x", "claude · aktiv"),
            preview(r1, "x · tab2", "claude · fertig"),
            preview(list_rect, "Liste (1)", "Fenster liegen hinter der Mitte")
        ],
        "placed windows in the label's words, parked ones as one list box"
    );
}

// ---------- New: moving and breathing (Animator.swift, GlowPanel) ----------

#[test]
fn glide_timing() {
    assert!(
        eased(0.0) == 0.0 && eased(0.5) == 0.5 && eased(1.0) == 1.0,
        "ease-in-out ends and middle"
    );
    assert!(
        eased(-1.0) == 0.0 && eased(2.0) == 1.0 && eased(f64::NAN) == 0.0,
        "clamped like Swift's min(1, max(0, t))"
    );
    assert!(
        eased(0.1) < 0.1 && eased(0.9) > 0.9,
        "soft start, soft arrival"
    );
    assert_eq!(
        lerp(
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Rect::new(100.0, 50.0, 300.0, 100.0),
            0.5
        ),
        Rect::new(50.0, 25.0, 200.0, 100.0)
    );

    let to = Rect::new(0.0, 1.0, 804.0, 619.0);
    let moves: Vec<(Rect, Rect)> = (0..8)
        .map(|i| (Rect::new(804.0 + i as f64, 0.0, 1400.0, 1239.0), to))
        .collect();
    let mut g = Glide::new(&moves, MOVE_DURATION);
    assert!(g.frame(0.0).is_empty(), "nobody starts at 0");
    let first = g.frame(0.01);
    assert_eq!(
        first[0],
        GlideStep::Shrink(0, glide_path(moves[0].0, to).0.size()),
        "window 0 shrinks at its own start"
    );
    assert!(
        first.len() == 2 && matches!(first[1], GlideStep::Move(0, _)),
        "only window 0 has started"
    );
    let moving = |steps: &[GlideStep], i: usize| {
        steps
            .iter()
            .any(|s| matches!(s, GlideStep::Move(j, _) | GlideStep::Shrink(j, _) if *j == i))
    };
    assert!(!moving(&g.frame(0.26), 7), "window 7 waits for its start");
    assert!(
        moving(&g.frame(0.28), 7),
        "the stagger is capped at 0.27 s, not 7 × 0.045"
    );
    let end = g.frame(MOVE_STAGGER_MAX + MOVE_DURATION + 0.01);
    assert_eq!(
        end.iter()
            .filter(|s| matches!(s, GlideStep::Arrive(_)))
            .count(),
        8,
        "everyone arrives"
    );
    assert!(g.done() && g.frame(10.0).is_empty(), "arrived once");
    let mut late = Glide::new(&[(moves[0].0, to)], MOVE_DURATION);
    assert_eq!(
        late.frame(1.0),
        [GlideStep::Arrive(0)],
        "a late first frame sets the target directly, no shrink"
    );
    let mut same = Glide::new(&[(to, to.offset_by(100.0, 0.0))], 0.0);
    assert!(
        matches!(same.frame(0.01)[..], [GlideStep::Move(0, _)]),
        "same size: no shrink; duration at least 0.05 s"
    );
    assert!(
        Glide::new(&[], MOVE_DURATION).done(),
        "nothing to move: done at once"
    );
}

#[test]
fn glow_breathes_and_fades() {
    // Whole breathing periods after Swift's reference date: the sine is at 0, the breath at its base.
    let born = REFERENCE_DATE + GLOW_BREATH_PERIOD * 200_000.0;
    let alpha = |now: f64, dying: Option<f64>, phase: f64| glow_alpha(now, born, dying, phase);
    assert_eq!(alpha(born, None, 0.0), Some(0.0), "starts invisible");
    let half = alpha(born + GLOW_FADE_IN / 2.0, None, 0.0).unwrap();
    let full = alpha(born + GLOW_BREATH_PERIOD, None, 0.0).unwrap();
    assert!(
        (full - GLOW_ALPHA_BASE).abs() < 1e-6,
        "faded in, at a whole period: the base, got {full}"
    );
    assert!(half > 0.0 && half < full, "smoothstep halfway, got {half}");
    let samples: Vec<f64> = (0..90)
        .map(|i| alpha(born + 1.0 + f64::from(i) * 0.05, None, 0.0).unwrap())
        .collect();
    let (lo, hi) = samples
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &a| (lo.min(a), hi.max(a)));
    assert!(
        lo >= GLOW_ALPHA_BASE - GLOW_ALPHA_SWING - 1e-9
            && hi <= GLOW_ALPHA_BASE + GLOW_ALPHA_SWING + 1e-9
            && hi - lo > GLOW_ALPHA_SWING,
        "breathes around the base, got {lo}..{hi}"
    );
    let quarter = alpha(born + GLOW_BREATH_PERIOD, None, 0.25).unwrap();
    assert!(
        (quarter - (GLOW_ALPHA_BASE + GLOW_ALPHA_SWING)).abs() < 1e-6,
        "the phase shifts the breath, got {quarter}"
    );
    let dying = born + GLOW_BREATH_PERIOD;
    assert!(
        (alpha(dying, Some(dying), 0.0).unwrap() - GLOW_ALPHA_BASE).abs() < 1e-6,
        "fading out starts from the breath"
    );
    assert!(
        alpha(dying + GLOW_FADE_OUT - 0.01, Some(dying), 0.0).is_some()
            && alpha(dying + GLOW_FADE_OUT + 0.01, Some(dying), 0.0).is_none(),
        "faded out after GLOW_FADE_OUT: gone"
    );
    // review quick 2026-11-06 (Muse): the wall clock can step back (manual or network time correction). Smoothstep
    // below 0 gave a frame brighter than ever (t = -1: 5x), the fade-out above its start.
    assert_eq!(
        alpha(born - 0.3, None, 0.0),
        Some(0.0),
        "clock back while fading in"
    );
    assert!(
        alpha(dying - 0.3, Some(dying), 0.0).unwrap() <= GLOW_ALPHA_BASE + GLOW_ALPHA_SWING,
        "clock back while fading out: never above the breath"
    );
}

// ---------- Inbox (Kevin 2026-11-09) ----------

fn inbox_window(id: u64, waiting: bool, name: &str, question: Option<&str>) -> TermWindow {
    let mut w = fake_window_with(true, 1000.0 + id as f64, None, waiting, true);
    w.id = id;
    w.sessions[0].digest = Some(Digest {
        name: Some(name.into()),
        question: question.map(Into::into),
        text: Some("Sendung ist angelegt.".into()),
        answer_at: Some(500.0),
        ..Digest::default()
    });
    w
}

fn inbox_windows() -> Vec<TermWindow> {
    vec![
        inbox_window(0, true, "Shop · Italien", Some("Pausieren oder halbieren?")),
        inbox_window(1, false, "Shop · Spanien", None),
    ]
}

fn inbox_list(rows: &[usize], height: f64) -> SideList {
    SideList {
        rect: Rect::new(0.0, 0.0, 430.0, height),
        entries: rows.iter().map(|&i| ListEntry::Row(i)).collect(),
        inbox: true,
    }
}

fn inbox_lines_of(
    w: &[TermWindow],
    list: &SideList,
    main: Option<usize>,
    read: &ReadAnswers,
    tab: InboxTab,
) -> Vec<ListLine> {
    inbox_lines_full(w, list, main, read, tab, &BTreeSet::new(), &[])
}

fn inbox_lines_full(
    w: &[TermWindow],
    list: &SideList,
    main: Option<usize>,
    read: &ReadAnswers,
    tab: InboxTab,
    snoozed: &BTreeSet<String>,
    pinned: &[String],
) -> Vec<ListLine> {
    let (since, lookout) = (BTreeMap::new(), BTreeMap::new());
    let planned: Vec<u64> = w.iter().map(|x| x.id).collect();
    inbox_target(
        list,
        &InboxInput {
            planned: &planned,
            windows: w,
            since: &since,
            lookout: &lookout,
            now: 1000.0,
            read,
            main,
            tab,
            snoozed,
            pinned,
            plan: None,
            plan_day: TEST_DAY,
            today: TEST_DAY,
            teams: &BTreeSet::new(),
            cards: &[],
            lead_names: &[],
            team: None,
        },
        7,
    )
    .lines
}

/// Tuesday 10 November 2026, the day the tests' plans are made for.
const TEST_DAY: NaiveDate = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();

/// The inbox's head with the three session counts and no planner.
fn head(tab: InboxTab, waiting: usize, working: usize, parked: usize) -> ListLine {
    ListLine::InboxHead {
        tab,
        counts: InboxCounts {
            waiting,
            working,
            parked,
            planned: None,
        },
        team: None,
    }
}

fn inbox_lines(
    w: &[TermWindow],
    list: &SideList,
    main: Option<usize>,
    read: &ReadAnswers,
) -> Vec<ListLine> {
    inbox_lines_of(w, list, main, read, InboxTab::Waiting)
}

fn card(line: &ListLine) -> &InboxCard {
    match line {
        ListLine::Card(c) => c,
        other => panic!("no card: {other:?}"),
    }
}

/// Questions first, then done; the card of the terminal in front is open, and the question stands without quotes.
#[test]
fn test_inbox_groups_and_open_card() {
    let w = inbox_windows();
    let lines = inbox_lines(
        &w,
        &inbox_list(&[0, 1], 900.0),
        Some(0),
        &ReadAnswers::new(),
    );
    assert_eq!(lines[0], head(InboxTab::Waiting, 2, 0, 0));
    assert_eq!(lines[1], ListLine::Group("Fragen an dich".into()));
    let question = card(&lines[2]);
    assert!(question.open && question.tone == LabelTone::Waiting);
    assert_eq!(question.name, "Italien");
    assert_eq!(question.project.as_deref(), Some("Shop"));
    assert_eq!(question.what.as_deref(), Some("Pausieren oder halbieren?"));
    assert_eq!(
        lines[3],
        ListLine::Hint("Rechts im Terminal antworten".into())
    );
    assert_eq!(lines[4], ListLine::Group("Fertig".into()));
    let done = card(&lines[5]);
    assert!(!done.open);
    assert_eq!(done.what.as_deref(), Some("Sendung ist angelegt."));
    assert_eq!(lines.len(), 6);
}

/// Kevin 2026-11-09: no buttons on the cards, a right-click offers what can be done; a done card can also be marked read.
#[test]
fn test_inbox_done_card_has_a_right_click_and_no_button() {
    let w = inbox_windows();
    let lines = inbox_lines(
        &w,
        &inbox_list(&[0, 1], 900.0),
        Some(1),
        &ReadAnswers::new(),
    );
    assert!(!lines.iter().any(|l| matches!(l, ListLine::Buttons { .. })));
    let k = lines
        .iter()
        .position(|l| matches!(l, ListLine::Card(c) if c.open))
        .unwrap();
    let top: f64 = lines[..k].iter().map(ListLine::height).sum();
    let hit = card_at(&lines, top + 5.0).unwrap();
    assert_eq!(hit.ids, [w[1].sessions[0].id.clone()]);
    assert_eq!(
        hit.menu,
        [
            CardButton::Pin,
            CardButton::Park,
            CardButton::ReadNext,
            CardButton::Rename,
            CardButton::MakeCard
        ]
    );
    assert_eq!(card_at(&lines, 5.0), None, "the tabs are no card");
}

/// The way back is a button only in Parked, on the open card; a click on it is not a click on the card.
#[test]
fn test_inbox_parked_card_button_is_hit() {
    let w = vec![inbox_window(0, false, "A · B", None)];
    let id = w[0].sessions[0].id.clone();
    let snoozed = BTreeSet::from([id.clone()]);
    let list = inbox_list(&[0], 900.0);
    let lines = inbox_lines_full(
        &w,
        &list,
        Some(0),
        &ReadAnswers::new(),
        InboxTab::Parked,
        &snoozed,
        &[],
    );
    let k = lines
        .iter()
        .position(|l| matches!(l, ListLine::Card(c) if c.open))
        .unwrap();
    let top: f64 = lines[..k].iter().map(ListLine::height).sum();
    let button = top + lines[k].height() + 5.0;
    let first = INBOX_TEXT_X + 5.0;
    assert_eq!(
        button_at(&lines, first, button),
        Some((CardButton::Unpark, &[id.clone()][..]))
    );
    assert_eq!(button_at(&lines, 10.0, button), None, "left of the button");
    assert_eq!(row_at(&lines, button), None, "the button is not the row");
    assert_eq!(
        row_at(&lines, top + 5.0),
        Some(&[id][..]),
        "a click on the card"
    );
    assert_eq!(button_at(&lines, first, top + 5.0), None);
    assert_eq!(
        card(&lines[k]).menu,
        [CardButton::Unpark, CardButton::Rename, CardButton::MakeCard]
    );
}

#[test]
fn test_inbox_read_card_leaves_until_a_new_answer() {
    let mut w = inbox_windows();
    let mut read = ReadAnswers::new();
    mark_read(&w[1], &mut read);
    let list = inbox_list(&[0, 1], 900.0);
    let lines = inbox_lines(&w, &list, Some(0), &read);
    assert_eq!(lines[0], head(InboxTab::Waiting, 1, 0, 1));
    assert!(!lines.contains(&ListLine::Group("Fertig".into())));

    w[1].sessions[0].digest.as_mut().unwrap().answer_at = Some(600.0);
    let lines = inbox_lines(&w, &list, Some(0), &read);
    assert!(lines.contains(&ListLine::Group("Fertig".into())));
}

/// Longer than the window (Kevin 2026-11-10: "das soll scrollbar sein"): every card of every group is listed, each group
/// under its heading, and nothing goes into "+N"; the window scrolls below the tabs.
#[test]
fn test_inbox_lists_every_card() {
    let mut w: Vec<TermWindow> = (0..12)
        .map(|i| inbox_window(i, true, "A · B", Some("Ja?")))
        .collect();
    w.extend((12..20).map(|i| inbox_window(i, false, "A · C", None)));
    let rows: Vec<usize> = (0..20).collect();
    let lines = inbox_lines(&w, &inbox_list(&rows, 500.0), None, &ReadAnswers::new());
    let cards = lines
        .iter()
        .filter(|l| matches!(l, ListLine::Card(_)))
        .count();
    assert_eq!(cards, 20);
    assert!(lines.contains(&ListLine::Group("Fertig".into())));
    assert!(!lines.iter().any(|l| matches!(l, ListLine::More(_))));
    let total: f64 = lines.iter().map(ListLine::height).sum();
    assert!(total > 500.0, "taller than the window: {total}");
}

/// A question over several lines of its own is one text: the card counts and shows it wrapped, not by its line ends.
#[test]
fn test_inbox_question_with_line_ends() {
    let w = vec![inbox_window(0, true, "A · B", Some("Eins\nZwei\n\nDrei"))];
    let lines = inbox_lines(&w, &inbox_list(&[0], 900.0), Some(0), &ReadAnswers::new());
    let c = card(&lines[2]);
    assert_eq!(c.what.as_deref(), Some("Eins Zwei Drei"));
    assert_eq!(c.lines, 1);
}

#[test]
fn test_inbox_empty_says_so() {
    let list = SideList {
        rect: Rect::new(0.0, 0.0, 430.0, 900.0),
        entries: vec![],
        inbox: true,
    };
    let lines = inbox_lines(&[], &list, None, &ReadAnswers::new());
    assert_eq!(
        lines,
        [
            head(InboxTab::Waiting, 0, 0, 0),
            ListLine::Empty("Niemand wartet auf dich.".into())
        ]
    );
}

fn running_window(id: u64, name: &str) -> TermWindow {
    let mut w = fake_window(false, 1000.0 + id as f64);
    w.id = id;
    w.sessions[0].digest = Some(Digest {
        name: Some(name.into()),
        text: Some("Baut gerade.".into()),
        ..Digest::default()
    });
    w
}

/// Kevin 2026-11-09: a second tab for the sessions that work: what runs, then the parking lot, each card clickable.
#[test]
fn test_inbox_working_tab() {
    let w = vec![
        running_window(0, "Lookout · Eingang"),
        running_window(1, "Shop · PPC"),
        running_window(2, "Acme · #9"),
        inbox_window(3, true, "Shop · Italien", Some("Ja?")),
    ];
    let list = SideList {
        rect: Rect::new(0.0, 0.0, 430.0, 900.0),
        entries: vec![
            ListEntry::Row(3),
            ListEntry::Working(0),
            ListEntry::Working(1),
            ListEntry::Lot(2),
        ],
        inbox: true,
    };
    let lines = inbox_lines_of(&w, &list, Some(0), &ReadAnswers::new(), InboxTab::Working);
    assert_eq!(
        lines[0],
        head(InboxTab::Working, 1, 3, 0),
        "both tabs count, whichever is shown"
    );
    assert_eq!(
        lines[1],
        ListLine::WaitBar {
            n: 1,
            text: "Italien: Ja?".into()
        },
        "somebody waits: the amber strip, whatever area is open"
    );
    assert_eq!(lines[2], ListLine::Group("Arbeitet".into()));
    let front = card(&lines[3]);
    assert!(front.open && front.name == "Eingang");
    assert_eq!(
        card(&lines[4]).name,
        "PPC",
        "the open card of a running session has no button"
    );
    assert_eq!(lines[5], ListLine::Group("Auf dem Parkplatz".into()));
    assert_eq!(card(&lines[6]).name, "#9");
    assert_eq!(lines.len(), 7);
    let on_card: f64 = lines[..4].iter().map(ListLine::height).sum();
    assert!(
        row_at(&lines, on_card + 5.0).is_some(),
        "a click brings it to the terminal"
    );

    let waiting = inbox_lines_of(&w, &list, Some(0), &ReadAnswers::new(), InboxTab::Waiting);
    assert_eq!(
        waiting
            .iter()
            .filter(|l| matches!(l, ListLine::Card(_)))
            .count(),
        1
    );
}

/// "Read" only takes a card out of the done group: a session that works again stays visible on the running tab.
#[test]
fn test_inbox_read_does_not_hide_a_running_session() {
    let w = vec![
        running_window(0, "A · Vorn"),
        running_window(1, "A · Wieder dran"),
    ];
    let mut read = ReadAnswers::new();
    mark_read(&w[1], &mut read);
    let list = SideList {
        rect: Rect::new(0.0, 0.0, 430.0, 900.0),
        entries: vec![ListEntry::Working(0), ListEntry::Working(1)],
        inbox: true,
    };
    let lines = inbox_lines_of(&w, &list, Some(0), &read, InboxTab::Working);
    let names: Vec<&str> = lines
        .iter()
        .filter_map(|l| match l {
            ListLine::Card(c) => Some(c.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, ["Vorn", "Wieder dran"]);
}

/// Between two plans a session can change state: it moves to the group it belongs in now, it never drops out of both tabs.
#[test]
fn test_inbox_groups_by_the_state_now() {
    let w = vec![
        running_window(0, "A · Vorn"),
        inbox_window(1, false, "A · Eben fertig", None),
        inbox_window(2, true, "A · Fragt jetzt", Some("Ja?")),
    ];
    let list = SideList {
        rect: Rect::new(0.0, 0.0, 430.0, 900.0),
        entries: vec![
            ListEntry::Working(0),
            ListEntry::Working(1),
            ListEntry::Lot(2),
        ],
        inbox: true,
    };
    let lines = inbox_lines(&w, &list, Some(0), &ReadAnswers::new());
    assert_eq!(lines[0], head(InboxTab::Waiting, 2, 1, 0));
    assert_eq!(card(&lines[2]).name, "Fragt jetzt");
    assert_eq!(lines[3], ListLine::Group("Fertig".into()));
    assert_eq!(card(&lines[4]).name, "Eben fertig");
}

/// A turn broken off with Esc (Kevin 2026-11-10, "1a 2a"): stopped, not done; it waits in its own group between the
/// questions and the done ones, and a question still comes first.
#[test]
fn test_inbox_stopped_between_questions_and_done() {
    let mut w = vec![
        inbox_window(0, false, "A · Eben fertig", None),
        inbox_window(1, false, "A · Abgebrochen", None),
        inbox_window(2, true, "A · Fragt jetzt", Some("Ja?")),
    ];
    w[1].sessions[0].digest.as_mut().unwrap().interrupted = true;
    assert_eq!(window_tone(&w[1]), LabelTone::Stopped);
    assert_eq!(LabelTone::Stopped.symbol(), "⏸");
    let mut asking = w[2].clone();
    asking.sessions[0].digest.as_mut().unwrap().interrupted = true;
    assert_eq!(
        window_tone(&asking),
        LabelTone::Waiting,
        "a question beats stopped"
    );
    let lines = inbox_lines(
        &w,
        &inbox_list(&[0, 1, 2], 900.0),
        None,
        &ReadAnswers::new(),
    );
    let shown: Vec<String> = lines
        .iter()
        .filter_map(|l| match l {
            ListLine::Group(g) => Some(format!("[{g}]")),
            ListLine::Card(c) => Some(c.name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        shown,
        [
            "[Fragen an dich]",
            "Fragt jetzt",
            "[Angehalten]",
            "Abgebrochen",
            "[Fertig]",
            "Eben fertig"
        ]
    );
    assert_eq!(
        lines[0],
        head(InboxTab::Waiting, 3, 0, 0),
        "stopped counts as waiting for Kevin"
    );
    let s = &w[1].sessions[0];
    assert_eq!(
        session_tone(s),
        LabelTone::Stopped,
        "its own clock (review quick 2026-11-10, GLM)"
    );
    let since = advance_since(&BTreeMap::new(), std::slice::from_ref(s), 1000.0);
    let later = advance_since(&since, std::slice::from_ref(s), 1300.0);
    assert_eq!(later[&s.id].tone, LabelTone::Stopped);
}

/// Kevin 2026-11-10 ("H"): back to the overview from an area's head, Waiting from the amber strip, an area from its tile,
/// weeks and days in the week strip; the + stays at the right.
#[test]
fn test_inbox_clicks_lead_where_they_say() {
    let w = 430.0;
    let area = vec![
        head(InboxTab::Working, 1, 2, 0),
        ListLine::Group("Arbeitet".into()),
    ];
    assert_eq!(
        nav_at(&area, w, INBOX_BACK_X + 5.0, 20.0),
        Some(InboxNav::Open(InboxTab::Overview))
    );
    assert_eq!(
        nav_at(&area, w, 200.0, 20.0),
        None,
        "the title is no way back"
    );
    assert_eq!(
        nav_at(&area, w, INBOX_BACK_X + 5.0, 60.0),
        None,
        "below the head"
    );
    let overview = vec![head(InboxTab::Overview, 1, 2, 0)];
    assert_eq!(
        nav_at(&overview, w, INBOX_BACK_X + 5.0, 20.0),
        None,
        "no way back on the overview"
    );
    assert!(plus_at(&area, INBOX_PLUS_X + 5.0, 20.0));
    assert!(!plus_at(&area, INBOX_PLUS_X - 5.0, 20.0), "left of it");
    assert!(!plus_at(&area, INBOX_PLUS_X + 5.0, 60.0), "below it");
    assert!(
        !plus_at(&[ListLine::More(1)], INBOX_PLUS_X + 5.0, 20.0),
        "not an inbox"
    );

    let bar = vec![ListLine::WaitBar {
        n: 1,
        text: "A".into(),
    }];
    assert_eq!(
        nav_at(&bar, w, 300.0, 10.0),
        Some(InboxNav::Open(InboxTab::Waiting))
    );

    let tile = |tab| InboxTile {
        tab,
        team: None,
        title: String::new(),
        count: 0,
        lines: vec![],
        hot: false,
    };
    let tiles = vec![
        ListLine::Tiles(vec![tile(InboxTab::Waiting)]),
        ListLine::Tiles(vec![tile(InboxTab::Working), tile(InboxTab::Planned)]),
    ];
    let row = INBOX_TILE_H + INBOX_TILE_GAP;
    assert_eq!(
        nav_at(&tiles, w, 400.0, 10.0),
        Some(InboxNav::Open(InboxTab::Waiting)),
        "across"
    );
    assert_eq!(
        nav_at(&tiles, w, 20.0, row + 10.0),
        Some(InboxNav::Open(InboxTab::Working))
    );
    assert_eq!(
        nav_at(&tiles, w, 410.0, row + 10.0),
        Some(InboxNav::Open(InboxTab::Planned))
    );
    assert_eq!(
        nav_at(&tiles, w, w / 2.0, row + 10.0),
        None,
        "the gap between two tiles"
    );
    assert_eq!(
        nav_at(&tiles, w, 20.0, row - 3.0),
        None,
        "the gap below a row"
    );

    let day = |k: u32| NaiveDate::from_ymd_opt(2026, 11, 9 + k).unwrap();
    let week = vec![ListLine::Week(WeekStrip {
        title: String::new(),
        days: (0..7)
            .map(|k| WeekDay {
                date: day(k),
                name: String::new(),
                number: 9 + k,
                today: false,
                selected: false,
                dots: vec![],
            })
            .collect(),
    })];
    assert_eq!(nav_at(&week, w, 20.0, 15.0), Some(InboxNav::Week(-1)));
    assert_eq!(nav_at(&week, w, w - 20.0, 15.0), Some(InboxNav::Week(1)));
    assert_eq!(nav_at(&week, w, w / 2.0, 15.0), None, "the title");
    let (x, dw) = week_day_span(1, w);
    assert_eq!(
        nav_at(&week, w, x + dw / 2.0, 60.0),
        Some(InboxNav::Day(day(1)))
    );
    assert_eq!(nav_at(&week, w, 5.0, 60.0), None, "left of Monday");
}

#[test]
fn test_inbox_working_tab_empty_says_so() {
    let list = SideList {
        rect: Rect::new(0.0, 0.0, 430.0, 900.0),
        entries: vec![],
        inbox: true,
    };
    let lines = inbox_lines_of(&[], &list, None, &ReadAnswers::new(), InboxTab::Working);
    assert_eq!(lines[1], ListLine::Empty("Gerade arbeitet niemand.".into()));
    let lines = inbox_lines_of(&[], &list, None, &ReadAnswers::new(), InboxTab::Parked);
    assert_eq!(lines[1], ListLine::Empty("Nichts geparkt.".into()));
}

/// Kevin 2026-11-09: a third tab. What he parked, whatever it is doing, and what he marked read: nothing is simply gone.
#[test]
fn test_inbox_parked_tab() {
    let mut w = vec![
        running_window(0, "A · Postfach"),
        inbox_window(1, false, "A · Fertig geparkt", None),
        inbox_window(2, false, "A · Gelesen", None),
        inbox_window(3, true, "A · Fragt trotzdem", Some("Ja?")),
        running_window(4, "A · Laeuft"),
    ];
    w[3].sessions[0].waiting = true;
    let snoozed: BTreeSet<String> = [0, 1, 3].map(|i| w[i].sessions[0].id.clone()).into();
    let mut read = ReadAnswers::new();
    mark_read(&w[2], &mut read);
    let list = SideList {
        rect: Rect::new(0.0, 0.0, 430.0, 900.0),
        entries: vec![
            ListEntry::Row(3),
            ListEntry::Row(1),
            ListEntry::Row(2),
            ListEntry::Working(0),
            ListEntry::Working(4),
        ],
        inbox: true,
    };
    let names = |lines: &[ListLine]| -> Vec<String> {
        lines
            .iter()
            .filter_map(|l| match l {
                ListLine::Card(c) => Some(c.name.clone()),
                _ => None,
            })
            .collect()
    };
    let parked = inbox_lines_full(&w, &list, Some(1), &read, InboxTab::Parked, &snoozed, &[]);
    assert_eq!(parked[0], head(InboxTab::Parked, 1, 1, 3));
    assert!(
        matches!(parked[1], ListLine::WaitBar { n: 1, .. }),
        "a parked session that asks waits"
    );
    assert_eq!(parked[2], ListLine::Group("Von dir geparkt".into()));
    assert_eq!(names(&parked), ["Fertig geparkt", "Postfach", "Gelesen"]);
    assert_eq!(
        parked[4],
        ListLine::Buttons {
            ids: vec![w[1].sessions[0].id.clone()],
            buttons: vec![CardButton::Unpark]
        },
        "the open card can come back"
    );
    assert!(parked.contains(&ListLine::Group("Gelesen".into())));

    let waiting = inbox_lines_full(&w, &list, None, &read, InboxTab::Waiting, &snoozed, &[]);
    assert_eq!(
        names(&waiting),
        ["Fragt trotzdem"],
        "a parked session that asks still comes"
    );
    let working = inbox_lines_full(&w, &list, None, &read, InboxTab::Working, &snoozed, &[]);
    assert_eq!(names(&working), ["Laeuft"]);
}

/// Parking and pinning are offered everywhere: on a running card and on a question too.
#[test]
fn test_inbox_park_on_every_card() {
    let w = vec![running_window(0, "A · B")];
    let list = SideList {
        rect: Rect::new(0.0, 0.0, 430.0, 900.0),
        entries: vec![ListEntry::Working(0)],
        inbox: true,
    };
    let lines = inbox_lines_of(&w, &list, None, &ReadAnswers::new(), InboxTab::Working);
    assert_eq!(
        card(&lines[2]).menu,
        [
            CardButton::Pin,
            CardButton::Park,
            CardButton::Rename,
            CardButton::MakeCard
        ]
    );
    let asks = vec![inbox_window(0, true, "A · B", Some("Ja?"))];
    let list = inbox_list(&[0], 900.0);
    let lines = inbox_lines(&asks, &list, Some(0), &ReadAnswers::new());
    assert_eq!(
        card(&lines[2]).menu,
        [
            CardButton::Pin,
            CardButton::Park,
            CardButton::Rename,
            CardButton::MakeCard
        ]
    );
    assert!(!lines.iter().any(|l| matches!(l, ListLine::Buttons { .. })));
}

/// Kevin 2026-11-09: a pinned card stands in its own group on top of Waiting, in the order he pinned them, whatever its
/// session does, until he unpins it. Below it all stays as it was. One that works is listed under Running too; "Wartet"
/// counts only who waits.
#[test]
fn test_inbox_pinned_on_top() {
    let w = vec![
        inbox_window(0, true, "Shop · Italien", Some("Ja?")),
        inbox_window(1, false, "Shop · Spanien", None),
        running_window(2, "Acme · Vertrag"),
        inbox_window(3, false, "Studio · Ton", None),
    ];
    let id = |i: usize| w[i].sessions[0].id.clone();
    let list = SideList {
        rect: Rect::new(0.0, 0.0, 430.0, 900.0),
        entries: vec![
            ListEntry::Row(0),
            ListEntry::Row(1),
            ListEntry::Row(3),
            ListEntry::Working(2),
        ],
        inbox: true,
    };
    let names = |lines: &[ListLine]| -> Vec<String> {
        lines
            .iter()
            .filter_map(|l| match l {
                ListLine::Card(c) => Some(c.name.clone()),
                _ => None,
            })
            .collect()
    };
    let none = BTreeSet::new();
    let read = ReadAnswers::new();
    let tab = |which, snoozed: &BTreeSet<String>, pinned: &[String]| {
        inbox_lines_full(&w, &list, None, &read, which, snoozed, pinned)
    };

    let lines = tab(InboxTab::Waiting, &none, &[id(2), id(3)]);
    assert_eq!(
        lines[0],
        head(InboxTab::Waiting, 3, 1, 0),
        "the pinned one that works is not counted as waiting"
    );
    assert_eq!(lines[1], ListLine::Group("Angeheftet".into()));
    assert_eq!(lines[4], ListLine::Group("Fragen an dich".into()));
    assert_eq!(lines[6], ListLine::Group("Fertig".into()));
    assert_eq!(names(&lines), ["Vertrag", "Ton", "Italien", "Spanien"]);
    let (pin, other) = (card(&lines[2]), card(&lines[5]));
    assert!(pin.pinned && !other.pinned);
    assert_eq!(
        pin.menu,
        [CardButton::Unpin, CardButton::Rename, CardButton::MakeCard],
        "only unpinning (and renaming)"
    );
    assert_eq!(
        other.menu,
        [
            CardButton::Pin,
            CardButton::Park,
            CardButton::Rename,
            CardButton::MakeCard
        ]
    );

    let lines = tab(InboxTab::Waiting, &none, &[id(3), id(2)]);
    assert_eq!(
        names(&lines)[..2],
        ["Ton", "Vertrag"],
        "in the order pinned"
    );

    let lines = tab(InboxTab::Working, &none, &[id(2)]);
    assert_eq!(names(&lines), ["Vertrag"]);
    assert!(matches!(lines[1], ListLine::WaitBar { n: 3, .. }));
    assert!(card(&lines[3]).pinned);
}

/// Review quick 2026-11-09: pinning ends an old park, like "Bring back". A parked session that asked again and was pinned
/// there stays under Running once it works again, and after unpinning it does not drop back to Parked.
#[test]
fn test_inbox_pin_ends_a_park() {
    let w = vec![running_window(0, "Acme · Postfach")];
    let id = w[0].sessions[0].id.clone();
    let mut snoozed = BTreeSet::from([id.clone()]);
    let mut read = ReadAnswers::new();
    mark_read(&w[0], &mut read);
    let mut pinned = vec![];
    set_pinned(&w[0], &mut pinned, &mut snoozed, &mut read, true);
    let list = SideList {
        rect: Rect::new(0.0, 0.0, 430.0, 900.0),
        entries: vec![ListEntry::Working(0)],
        inbox: true,
    };
    let lines = inbox_lines_full(&w, &list, None, &read, InboxTab::Working, &snoozed, &pinned);
    assert_eq!(card(&lines[2]).name, "Postfach", "still under Running");
    assert!(card(&lines[2]).pinned);

    set_pinned(&w[0], &mut pinned, &mut snoozed, &mut read, false);
    let lines = inbox_lines_full(&w, &list, None, &read, InboxTab::Parked, &snoozed, &pinned);
    assert_eq!(lines[1], ListLine::Empty("Nichts geparkt.".into()));
    assert!(read.is_empty(), "no old read mark left either");
}

/// Review quick 2026-11-09, round 2: a tab with an old park that joins a pinned window later changes nothing while it is
/// pinned (still under Running, not under Parked, no way-back button on the open card), and unpinning ends that park.
#[test]
fn test_inbox_pin_outweighs_a_park_that_joins_later() {
    let mut w = vec![running_window(0, "Acme · Postfach")];
    let mut joined = w[0].sessions[0].clone();
    joined.id = "claude:joined".into();
    let mut snoozed = BTreeSet::from([joined.id.clone()]);
    let mut read = ReadAnswers::new();
    let mut pinned = vec![];
    set_pinned(&w[0], &mut pinned, &mut snoozed, &mut read, true);
    w[0].sessions.push(joined);
    let list = SideList {
        rect: Rect::new(0.0, 0.0, 430.0, 900.0),
        entries: vec![ListEntry::Working(0)],
        inbox: true,
    };
    let running = inbox_lines_full(
        &w,
        &list,
        Some(0),
        &read,
        InboxTab::Working,
        &snoozed,
        &pinned,
    );
    assert_eq!(running[1], ListLine::Group("Arbeitet".into()));
    assert!(
        matches!(running.get(2), Some(ListLine::Card(c)) if c.pinned && c.menu.starts_with(&[CardButton::Unpin, CardButton::Rename])),
        "still under Running, pinned"
    );
    assert!(
        !running
            .iter()
            .any(|l| matches!(l, ListLine::Buttons { .. })),
        "no way-back button on a pinned card"
    );
    let parked = inbox_lines_full(&w, &list, None, &read, InboxTab::Parked, &snoozed, &pinned);
    assert_eq!(parked[1], ListLine::Empty("Nichts geparkt.".into()));

    set_pinned(&w[0], &mut pinned, &mut snoozed, &mut read, false);
    assert!(snoozed.is_empty(), "unpinning ends it too");
}

/// The cards scroll, and with "Show scroll bars: Always" the scroller takes room from their text (review quick
/// 2026-11-10): a question that just fills the line without it gets a second line rather than losing its end.
#[test]
fn test_inbox_open_card_leaves_room_for_a_scroller() {
    let w = vec![inbox_window(0, true, "A · B", Some(&"a".repeat(41)))];
    let lines = inbox_lines(&w, &inbox_list(&[0], 900.0), Some(0), &ReadAnswers::new());
    assert_eq!(card(&lines[2]).lines, 2);
}

/// A long question gets more lines on the open card, a short one stays at one, and it never grows without end.
#[test]
fn test_inbox_open_card_height_follows_the_text() {
    let long = "Soll ich die Kampagne pausieren, das Budget halbieren oder alles lassen, wie es ist, bis die Zahlen da sind?";
    let w = vec![
        inbox_window(0, true, "A · B", Some("Ja?")),
        inbox_window(1, true, "A · C", Some(long)),
        inbox_window(2, true, "A · D", Some(&long.repeat(9))),
    ];
    let list = inbox_list(&[0, 1, 2], 2000.0);
    let lines_of = |m| {
        inbox_lines(&w, &list, Some(m), &ReadAnswers::new())
            .iter()
            .find_map(|l| match l {
                ListLine::Card(c) if c.open => Some(c.lines),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(lines_of(0), 1);
    assert!(lines_of(1) >= 2);
    assert_eq!(lines_of(2), 6);
}

// ---------- Overview and Planned (Kevin 2026-11-10, "H") ----------

fn plan_item(title: &str, day: NaiveDate, time: Option<&str>) -> PlannedItem {
    PlannedItem {
        title: title.into(),
        project: "studio".into(),
        day,
        time: time.map(|t| chrono::NaiveTime::parse_from_str(t, "%H:%M").unwrap()),
        detail: time.map(|_| "claude · sonnet".into()),
        problem: None,
        deadline: None,
        since_days: 0,
    }
}

fn plan_lines(
    w: &[TermWindow],
    list: &SideList,
    tab: InboxTab,
    plan: Option<&[PlannedItem]>,
    plan_day: NaiveDate,
) -> Vec<ListLine> {
    let (since, lookout, read, none) = (
        BTreeMap::new(),
        BTreeMap::new(),
        ReadAnswers::new(),
        BTreeSet::new(),
    );
    let planned: Vec<u64> = w.iter().map(|x| x.id).collect();
    let o = InboxInput {
        planned: &planned,
        windows: w,
        since: &since,
        lookout: &lookout,
        now: 1000.0,
        read: &read,
        main: None,
        tab,
        snoozed: &none,
        pinned: &[],
        plan,
        plan_day,
        today: TEST_DAY,
        teams: &BTreeSet::new(),
        cards: &[],
        lead_names: &[],
        team: None,
    };
    inbox_target(list, &o, 7).lines
}

fn tiles(lines: &[ListLine]) -> Vec<Vec<&InboxTile>> {
    lines
        .iter()
        .filter_map(|l| match l {
            ListLine::Tiles(t) => Some(t.iter().collect()),
            _ => None,
        })
        .collect()
}

/// The overview: Waiting across, amber with who waits; the other areas two by two; without a planner no Planned tile.
#[test]
fn test_inbox_overview_tiles() {
    let w = inbox_windows();
    let list = inbox_list(&[0, 1], 900.0);
    let lines = plan_lines(&w, &list, InboxTab::Overview, None, TEST_DAY);
    assert_eq!(lines[0], head(InboxTab::Overview, 2, 0, 0));
    let rows = tiles(&lines);
    assert_eq!(rows.iter().map(Vec::len).collect::<Vec<_>>(), [1, 2]);
    let wait = rows[0][0];
    assert!(wait.hot && wait.tab == InboxTab::Waiting && wait.count == 2);
    assert_eq!(wait.title, "Wartet auf dich");
    assert_eq!(wait.lines, ["? Italien", "✓ Spanien"]);
    assert_eq!(
        (rows[1][0].tab, rows[1][1].tab),
        (InboxTab::Working, InboxTab::Parked)
    );
    assert_eq!(rows[1][0].lines, ["Gerade arbeitet niemand."]);
    assert!(
        !lines.iter().any(|l| matches!(l, ListLine::WaitBar { .. })),
        "no strip on the overview"
    );

    let plan = vec![plan_item("Report", TEST_DAY, Some("10:00"))];
    let lines = plan_lines(&w, &list, InboxTab::Overview, Some(&plan), TEST_DAY);
    let rows = tiles(&lines);
    assert_eq!(rows.iter().map(Vec::len).collect::<Vec<_>>(), [1, 2, 1]);
    let planned = rows[1][1];
    assert_eq!(
        (planned.tab, planned.count, planned.hot),
        (InboxTab::Planned, 1, false)
    );
    assert_eq!(
        planned.lines,
        ["▶ heute 10:00 Report", "1 heute · 0 morgen"]
    );

    let nobody = plan_lines(
        &[],
        &inbox_list(&[], 900.0),
        InboxTab::Overview,
        None,
        TEST_DAY,
    );
    let wait = tiles(&nobody)[0][0];
    assert!(!wait.hot && wait.title == "Wartet" && wait.lines == ["Niemand wartet auf dich."]);
}

/// Planned: the week with dots, the chosen day's items with their marks, a start that did not happen in amber (the tile
/// too), a deadline on its day; without a planner the area is the overview.
#[test]
fn test_inbox_planned_area() {
    let day = |d: u32| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
    let mut failed = plan_item("Backup", day(9), Some("09:50"));
    failed.problem = Some("no window".into());
    let mut domain = plan_item("Domain", day(12), None);
    domain.deadline = Some(day(13));
    let mut old = plan_item("Alt", day(5), None);
    old.since_days = 5;
    let plan = vec![
        plan_item("Report", TEST_DAY, Some("10:00")),
        failed,
        domain,
        old,
    ];
    let list = inbox_list(&[], 900.0);

    let lines = plan_lines(&[], &list, InboxTab::Planned, Some(&plan), TEST_DAY);
    assert_eq!(
        lines[0],
        ListLine::InboxHead {
            tab: InboxTab::Planned,
            counts: InboxCounts {
                waiting: 0,
                working: 0,
                parked: 0,
                planned: Some(4)
            },
            team: None,
        }
    );
    let ListLine::Week(week) = &lines[1] else {
        panic!("no week: {:?}", lines[1])
    };
    assert_eq!(week.title, "KW 46 · 9. bis 15. November");
    assert_eq!(week.days.len(), 7);
    let tue = &week.days[1];
    assert!(tue.today && tue.selected && tue.number == 10 && tue.name == "Di");
    assert_eq!(
        tue.dots,
        [PlanMark::Problem, PlanMark::Start, PlanMark::Task],
        "overdue items stand on today"
    );
    assert_eq!(week.days[3].dots, [PlanMark::Task]);
    assert_eq!(week.days[4].dots, [PlanMark::Deadline]);
    assert_eq!(lines[2], ListLine::Group("Heute · Di 10.11.".into()));
    let marks: Vec<(String, Option<PlanMark>, Option<String>)> = lines[3..]
        .iter()
        .map(|l| (card(l).name.clone(), card(l).mark, card(l).since.clone()))
        .collect();
    assert_eq!(
        marks,
        [
            (
                "Backup".into(),
                Some(PlanMark::Problem),
                Some("09:50".into())
            ),
            ("Report".into(), Some(PlanMark::Start), Some("10:00".into())),
            ("Alt".into(), Some(PlanMark::Task), None),
        ]
    );
    assert_eq!(
        card(&lines[3]).what.as_deref(),
        Some("Nicht gestartet: no window")
    );
    assert!(
        card(&lines[4]).ids.is_empty() && card(&lines[4]).menu.is_empty(),
        "no session behind it"
    );

    let friday = plan_lines(&[], &list, InboxTab::Planned, Some(&plan), day(13));
    assert_eq!(friday[2], ListLine::Group("Fr 13.11.".into()));
    let deadline = card(&friday[3]);
    assert_eq!(
        (deadline.mark, deadline.name.as_str()),
        (Some(PlanMark::Deadline), "Domain")
    );
    assert_eq!(
        deadline.what.as_deref(),
        Some("Harte Frist Fr 13.11., steht am Do 12.11.")
    );
    let sunday = plan_lines(&[], &list, InboxTab::Planned, Some(&plan), day(15));
    assert_eq!(
        sunday[3],
        ListLine::Empty("An diesem Tag ist nichts geplant.".into())
    );

    let overview = plan_lines(&[], &list, InboxTab::Overview, Some(&plan), TEST_DAY);
    let planned = tiles(&overview)[1][1];
    assert!(planned.hot, "a start that did not happen wants Kevin");
    assert_eq!(planned.lines[0], "! Nicht gestartet: Backup");

    let gone = plan_lines(&[], &list, InboxTab::Planned, None, TEST_DAY);
    assert_eq!(
        gone[0],
        head(InboxTab::Overview, 0, 0, 0),
        "no planner any more: the overview"
    );
}

/// A dot per item the day lists, problems and deadlines first, so the cut at four never hides them (review quick
/// 2026-11-10: one dot for any number of deadlines, two for a task due on its own deadline, a deadline cut after four
/// starts).
#[test]
fn test_week_dots_one_per_item() {
    let day = |d: u32| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
    let task = |title: &str, on: u32, deadline: u32| PlannedItem {
        deadline: Some(day(deadline)),
        ..plan_item(title, day(on), None)
    };
    let mut plan = vec![task("A", 12, 13), task("B", 12, 13), task("Own", 11, 11)];
    plan.extend(["09:00", "10:00", "11:00", "12:00"].map(|t| plan_item("Start", day(14), Some(t))));
    plan.push(task("Late", 12, 14));
    let list = inbox_list(&[], 900.0);
    let lines = plan_lines(&[], &list, InboxTab::Planned, Some(&plan), TEST_DAY);
    let ListLine::Week(week) = &lines[1] else {
        panic!("no week: {:?}", lines[1])
    };
    let dots = |d: u32| {
        week.days
            .iter()
            .find(|w| w.date == day(d))
            .unwrap()
            .dots
            .clone()
    };
    assert_eq!(dots(11), [PlanMark::Task], "one item, one dot");
    assert_eq!(dots(12), [PlanMark::Task; 3]);
    assert_eq!(
        dots(13),
        [PlanMark::Deadline, PlanMark::Deadline],
        "two deadlines, two dots"
    );
    assert_eq!(
        dots(14),
        [
            PlanMark::Deadline,
            PlanMark::Start,
            PlanMark::Start,
            PlanMark::Start
        ],
        "the deadline is not cut"
    );
}

/// The Planned area stands on today until Kevin picks another day, also over midnight (review quick 2026-11-10: it stayed
/// on the day before, which showed nothing).
#[test]
fn test_plan_day_follows_today() {
    let day = |d: u32| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
    let (tue, wed, thu) = (day(10), day(11), day(12));
    let opened = plan_day_after(InboxNav::Open(InboxTab::Planned), Some(thu), tue);
    assert_eq!(
        opened.unwrap_or(wed),
        wed,
        "opened on Tuesday, open on Wednesday"
    );
    let tapped = plan_day_after(InboxNav::Day(tue), Some(thu), tue);
    assert_eq!(tapped.unwrap_or(wed), wed, "today tapped");
    assert_eq!(plan_day_after(InboxNav::Day(thu), None, tue), Some(thu));
    let back = plan_day_after(InboxNav::Week(-1), None, tue);
    assert_eq!(back, Some(day(2)));
    assert_eq!(
        plan_day_after(InboxNav::Week(1), back, tue),
        None,
        "back in this week: today"
    );
    assert_eq!(
        plan_day_after(InboxNav::Open(InboxTab::Waiting), Some(thu), tue),
        Some(thu),
        "another area keeps it"
    );
}

/// Kevin 2026-11-10: two fingers sideways in an area lead back to the overview, once per gesture, either direction; up and
/// down scrolls as ever, so does a mouse wheel; the rest of a swipe does not scroll the list.
#[test]
fn test_swipe_back_needs_a_sideways_gesture() {
    let start = (0.0, 0.0, false);
    // Sideways: 5, 20, 20 points -> back on the third event, the fourth is still the swipe's but does not lead again.
    let (s, mine, back) = swipe_step(start, true, true, true, 5.0, 0.0);
    assert!(!mine && !back);
    let (s, _, back) = swipe_step(s, false, true, true, 20.0, 1.0);
    assert!(!back);
    let (s, mine, back) = swipe_step(s, false, true, true, 20.0, 1.0);
    assert!(mine && back, "45 points sideways");
    let (s, mine, back) = swipe_step(s, false, true, true, 30.0, 0.0);
    assert!(mine && !back, "once per gesture");
    // A new gesture starts afresh, the other way round works too.
    let (s, _, _) = swipe_step(s, true, true, true, -30.0, 0.0);
    let (_, _, back) = swipe_step(s, false, true, true, -30.0, 2.0);
    assert!(back, "to the left as well");
    // Up and down, diagonal, a mouse wheel: no way back, the list scrolls.
    let mut s = start;
    for _ in 0..10 {
        let (n, mine, back) = swipe_step(s, false, true, true, 2.0, 30.0);
        assert!(!mine && !back);
        s = n;
    }
    let (_, _, back) = swipe_step(start, true, true, true, 50.0, 30.0);
    assert!(!back, "more down than half the way sideways");
    for trackpad in [false, true] {
        let (_, mine, back) = swipe_step(start, false, trackpad, false, 200.0, 0.0);
        assert!(
            !mine && !back,
            "a mouse wheel, or two fingers on the overview"
        );
    }
}

/// Once a swipe led back, the rest of it stays the swipe's, the glide after lifting the fingers too, though the overview
/// it opened is no area: the overview does not scroll with it (review quick 2026-11-10). A mouse wheel scrolls.
#[test]
fn test_swipe_back_keeps_the_rest_of_the_gesture() {
    let (s, _, back) = swipe_step((0.0, 0.0, false), true, true, true, 50.0, 2.0);
    assert!(back);
    let (s, mine, back) = swipe_step(s, false, true, false, 10.0, 6.0);
    assert!(mine && !back, "the rest of the swipe, on the overview now");
    let (s, mine, _) = swipe_step(s, false, true, false, 4.0, 3.0);
    assert!(mine, "its glide");
    let (_, mine, _) = swipe_step(s, false, false, false, 0.0, 20.0);
    assert!(!mine, "a mouse wheel scrolls");
    let (_, mine, _) = swipe_step(s, true, true, false, 0.0, 20.0);
    assert!(!mine, "a new gesture on the overview scrolls");
}
