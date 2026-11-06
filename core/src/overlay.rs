//! What the overlays show (wave B2): glow targets, labels, lists, whereabouts, menu lines. Pure decisions only;
//! drawing is in the macOS crate. Ported from Overlay.swift, with the pure timing of Animator.swift.
//!
//! Design rule (Kevin 2026-09-18): "smoothe Prozesse, die den Geist calm halten, aber informieren": a calm mint instead
//! of red, a thin line with a soft halo, slow breathing (4.5 s) just below notice, every frame out of step with the
//! others, soft fades. Every AI window except the active one carries a label: status and duration, topic, step.
//!
//! The Lookout line a session writes about itself (Swift `Digest.line`, status logic) comes in as data: the caller maps
//! session ids to their lines, like the since entries.

use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;

use crate::geom::{Rect, Size};
use crate::i18n::tr;
use crate::layout::{LIST_HEADER_H, LIST_ROW_H, screen_index};
use crate::mapping::{index_of_window, last_path_component};
use crate::model::{
    Arrangement, Engine, LabelTone, LastBlock, ListEntry, LookoutLine, OnScreenWindow, Pid,
    Session, SideList, TermWindow, Time, graphemes, title_glyph, trim_ws,
};
use crate::motion::{WARP_BID, ax_origin, cocoa_to_ax, glide_path};

// ---------- Glow frames ----------

/// Times in seconds, deliberately long: quick changes draw the eye, slow ones inform.
pub const GLOW_FADE_IN: f64 = 0.55;
pub const GLOW_FADE_OUT: f64 = 0.4;
pub const GLOW_BREATH_PERIOD: f64 = 4.5;
pub const GLOW_ALPHA_BASE: f64 = 0.62;
pub const GLOW_ALPHA_SWING: f64 = 0.13;
/// Breathing offset added per new frame: an odd step, so no two frames breathe in unison (that reads as an alarm).
pub const GLOW_PHASE_STEP: f64 = 0.37;
/// Swift's reference date (2001-01-01) in UNIX seconds: the breathing phase counts from there.
const REFERENCE_DATE: Time = 978_307_200.0;

/// Opacity of a glow frame born at `born_at`, fading out since `dying_at`, `phase` its breathing offset. `None` = faded
/// out, the panel can go. The caller clamps below at 0 (Swift `max(0, a)`).
pub fn glow_alpha(now: Time, born_at: Time, dying_at: Option<Time>, phase: f64) -> Option<f64> {
    let breath = GLOW_ALPHA_BASE
        + GLOW_ALPHA_SWING
            * (((now - REFERENCE_DATE) / GLOW_BREATH_PERIOD + phase) * 2.0 * PI).sin();
    // Both clamped below at 0: the wall clock can step back, and smoothstep below 0 is brighter than ever (review quick
    // 2026-11-06, Muse; Swift clamped only above).
    if let Some(dying_at) = dying_at {
        let t = ((now - dying_at) / GLOW_FADE_OUT).max(0.0);
        return if t >= 1.0 {
            None
        } else {
            Some(breath * (1.0 - t))
        };
    }
    // Soft fade-in (smoothstep), no linear jolt.
    let t = ((now - born_at) / GLOW_FADE_IN).clamp(0.0, 1.0);
    Some(breath * t * t * (3.0 - 2.0 * t))
}

/// A glow target: stable id, window rectangle (Cocoa) and cause. The cause picks the colour: "done" and "waits for your
/// answer" must not look alike, else the frame is mere decoration.
#[derive(Clone, Debug, PartialEq)]
pub struct GlowTarget {
    pub id: String,
    pub rect: Rect,
    pub waiting: bool,
    /// CGWindowID of the terminal window; the frame is ordered right above it. `None` = unknown (window list not
    /// readable): in front on the normal level, as a fallback.
    pub window_number: Option<i64>,
}

/// Does the panel already lie directly above its window, with at most own panels between? Only then reordering is
/// skipped: every `order(.above, relativeTo:)` on another app's window costs WindowServer a rights check with an error
/// line (284/min measured; on 2026-09-27 it helped overload logd). `stack`: windows of all apps, front to back; `None` =
/// unknown -> no, order again. `own`: Lookout's process.
pub fn sits_above(panel: i64, target: i64, stack: Option<&[OnScreenWindow]>, own: Pid) -> bool {
    let Some(stack) = stack else {
        return false;
    };
    let position = |n: i64| stack.iter().position(|w| w.number == n);
    match (position(panel), position(target)) {
        (Some(p), Some(t)) if p < t => stack[p + 1..t].iter().all(|w| w.pid == own),
        _ => false,
    }
}

// ---------- On screen? ----------

/// Does the window practically fill its screen? Then the user looks at it and a frame only disturbs (Kevin 2026-09-18:
/// "sobald ein Fenster maximiert wurde sollte der Rahmen weg sein"). 95 %: a maximized window usually leaves a few points,
/// a tiled one clearly more.
pub fn is_maximized(w: &TermWindow, screens: &[Rect]) -> bool {
    if screens.is_empty() {
        return false;
    }
    let vis = screens[screen_index(w.frame, screens).min(screens.len() - 1)];
    vis.width() > 0.0
        && vis.height() > 0.0
        && w.frame.width() >= vis.width() * 0.95
        && w.frame.height() >= vis.height() * 0.95
}

/// Is a window on the visible space right now? Matched by app and rectangle: the window list knows no AX identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnScreenMatch {
    /// No window list (query failed).
    Unknown,
    /// Not on this space (other desktop, minimized). Swift `.none`.
    Absent,
    /// Exactly one hit: its CGWindowID.
    Unique(i64),
    /// Two windows of the same app on (almost) the same rectangle.
    Ambiguous,
}

/// How far window list and window frame may differ, in points.
const ON_SCREEN_TOLERANCE: f64 = 6.0;

pub fn locate_on_screen(
    w: &TermWindow,
    on_screen: Option<&[OnScreenWindow]>,
    screen_h: f64,
) -> OnScreenMatch {
    let Some(on_screen) = on_screen else {
        return OnScreenMatch::Unknown;
    };
    let hits: Vec<&OnScreenWindow> = on_screen_hits(w, on_screen, screen_h).collect();
    match hits[..] {
        [] => OnScreenMatch::Absent,
        [one] => OnScreenMatch::Unique(one.number),
        _ => OnScreenMatch::Ambiguous,
    }
}

/// Windows of the same app on (almost) the same rectangle, in the order of the window list.
fn on_screen_hits<'a>(
    w: &TermWindow,
    on_screen: &'a [OnScreenWindow],
    screen_h: f64,
) -> impl Iterator<Item = &'a OnScreenWindow> {
    let ax = cocoa_to_ax(w.frame, ax_origin(&w.bundle_id), screen_h);
    let pid = w.owner_pid;
    let near = |a: f64, b: f64| (a - b).abs() <= ON_SCREEN_TOLERANCE;
    on_screen.iter().filter(move |o| {
        o.pid == pid
            && near(o.rect.min_x(), ax.min_x())
            && near(o.rect.min_y(), ax.min_y())
            && near(o.rect.width(), ax.width())
            && near(o.rect.height(), ax.height())
    })
}

/// Window number of the main window for the list (plan R2-1): parked windows lie congruent behind it, so
/// `locate_on_screen` is ambiguous. The window list comes front to back [Modellwissen, checked live]: the first match of
/// this app is the main window once it was raised.
pub fn frontmost_number(
    w: &TermWindow,
    on_screen: &[OnScreenWindow],
    screen_h: f64,
) -> Option<i64> {
    on_screen_hits(w, on_screen, screen_h)
        .next()
        .map(|o| o.number)
}

// ---------- Which windows get an overlay ----------

static NOTHING_PARKED: BTreeSet<usize> = BTreeSet::new();

/// What the overlays look at. `OverlayInput::new` sets Swift's defaults: no active session, nothing parked.
#[derive(Clone, Copy, Debug)]
pub struct OverlayInput<'a> {
    pub windows: &'a [TermWindow],
    /// Terminal windows on the visible space, front to back; `None` = list not readable.
    pub on_screen: Option<&'a [OnScreenWindow]>,
    /// Height of the primary screen (AX coordinates).
    pub screen_h: f64,
    /// Visible frames of the screens.
    pub screens: &'a [Rect],
    /// The active session (Swift `focusedIndex`): the user works in it.
    pub focused: Option<usize>,
    /// Parked windows of the last plan (`Arrangement::parked`, the parking lot included unless it is peeked at).
    pub parked: &'a BTreeSet<usize>,
}

impl<'a> OverlayInput<'a> {
    pub fn new(
        windows: &'a [TermWindow],
        on_screen: Option<&'a [OnScreenWindow]>,
        screen_h: f64,
        screens: &'a [Rect],
    ) -> OverlayInput<'a> {
        OverlayInput {
            windows,
            on_screen,
            screen_h,
            screens,
            focused: None,
            parked: &NOTHING_PARKED,
        }
    }
}

/// Over which windows may an overlay lie? Visible, on the CURRENT space, and never the active session: the user works
/// in it, between two answers it is technically "done" (Kevin 2026-09-18). The glow takes the done ones of these, the
/// label all with a session. The space test is needed because the panels join all spaces: without it the overlay
/// floated on EVERY space at the same spot. Excluded windows keep their overlays: excluding only concerns arranging
/// (Kevin 2026-09-18). Gives index + CGWindowID (`None` = window list not readable: do not hide wrongly, just without
/// level). `skip_maximized`: the maximized rule is for the glow (Kevin 2026-09-18); a label also needs a window that
/// Lookout itself tiled to the full screen (only running session on the MacBook, Kimi 2026-09-25).
pub fn overlay_candidates(o: &OverlayInput, skip_maximized: bool) -> Vec<(usize, Option<i64>)> {
    o.windows
        .iter()
        .enumerate()
        .filter_map(|(i, w)| {
            // Parked (plan 1d, GP 2): never glow or label, in any terminal. Being congruent is not enough, it only holds
            // within the same app (a Ghostty window behind a Warp main window would get both).
            if Some(i) == o.focused
                || o.parked.contains(&i)
                || w.minimized
                || w.fullscreen
                || (skip_maximized && is_maximized(w, o.screens))
            {
                return None;
            }
            match locate_on_screen(w, o.on_screen, o.screen_h) {
                OnScreenMatch::Unknown => Some((i, None)),
                OnScreenMatch::Unique(n) => Some((i, Some(n))),
                // Ambiguous: rather no overlay than one above the wrong window (review quick 2026-09-18). Not on this
                // space: none.
                OnScreenMatch::Absent | OnScreenMatch::Ambiguous => None,
            }
        })
        .collect()
}

pub fn glow_targets(o: &OverlayInput) -> Vec<GlowTarget> {
    overlay_candidates(o, true)
        .into_iter()
        .filter(|&(i, _)| o.windows[i].is_ready())
        .map(|(i, window_number)| {
            let w = &o.windows[i];
            // Id by session, not by position: re-sorting keeps the same panel at the same window, no fade-in again.
            let id = w
                .sessions
                .first()
                .map_or_else(|| format!("win{i}"), |s| s.id.clone());
            GlowTarget {
                id,
                rect: w.frame,
                waiting: w.is_waiting(),
                window_number,
            }
        })
        .collect()
}

// ---------- Where is a window? ----------

/// Where an AI window is, for the window list in the menu (Kevin 2026-09-18: "kevin-05 sehe ich nicht, aber er zeigt
/// an, dass da ein Fenster sei": it lay minimized in the Dock, and hovering lit an outline on an empty spot).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowWhereabouts {
    Visible,
    Minimized,
    Fullscreen,
    OtherSpace,
    Immovable,
}

impl WindowWhereabouts {
    pub fn label(self) -> &'static str {
        match self {
            WindowWhereabouts::Visible => "",
            WindowWhereabouts::Minimized => tr("minimiert (im Dock)", "minimized (in the Dock)"),
            WindowWhereabouts::Fullscreen => tr("Vollbild", "full screen"),
            WindowWhereabouts::OtherSpace => tr("anderer Schreibtisch", "other desktop"),
            WindowWhereabouts::Immovable => tr("nicht beweglich", "not movable"),
        }
    }
}

/// `on_screen`: is the window on the visible space (`locate_on_screen` not `Absent`)?
pub fn whereabouts(w: &TermWindow, on_screen: bool) -> WindowWhereabouts {
    if w.minimized {
        WindowWhereabouts::Minimized
    } else if w.fullscreen {
        WindowWhereabouts::Fullscreen
    } else if !on_screen {
        WindowWhereabouts::OtherSpace
    } else if w.movable {
        WindowWhereabouts::Visible
    } else {
        WindowWhereabouts::Immovable
    }
}

// ---------- Labels (design 2026-09-21) ----------
//
// Kevin 2026-09-21: "rechts erkenne ich gar nicht, was da genau passiert". Every AI window except the active session
// carries a calm label at the top: status and duration, topic, current step or result.

pub fn session_tone(s: &Session) -> LabelTone {
    if s.waiting {
        LabelTone::Waiting
    } else if s.ready {
        LabelTone::Ready
    } else if s.watching {
        LabelTone::Watching
    } else {
        LabelTone::Running
    }
}

/// State of a window as its label shows it (visible tab).
pub fn window_tone(w: &TermWindow) -> LabelTone {
    if w.is_waiting() {
        LabelTone::Waiting
    } else if w.is_ready() {
        LabelTone::Ready
    } else if w.is_watching() {
        LabelTone::Watching
    } else {
        LabelTone::Running
    }
}

/// Since when is a session in its state? `since` `None` = unknown (start not seen).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SinceEntry {
    pub tone: LabelTone,
    pub since: Option<Time>,
}

/// Carried forward every session tick: same state -> stays; change -> now; seen for the first time -> from the
/// transcript (done/question: last answer; running: the user's last prompt if it is in the tail read). Vanished
/// sessions drop out.
pub fn advance_since(
    old: &BTreeMap<String, SinceEntry>,
    sessions: &[Session],
    now: Time,
) -> BTreeMap<String, SinceEntry> {
    let mut next = BTreeMap::new();
    for s in sessions {
        let tone = session_tone(s);
        let entry = match old.get(&s.id) {
            Some(prev) if prev.tone == tone => *prev,
            Some(_) => SinceEntry {
                tone,
                since: Some(now),
            },
            None => SinceEntry {
                tone,
                since: s.digest.as_ref().and_then(|d| {
                    if tone == LabelTone::Running {
                        d.prompt_at
                    } else {
                        d.answer_at
                    }
                }),
            },
        };
        next.insert(s.id.clone(), entry);
    }
    next
}

/// "gerade eben" · "6 Min." · "1 Std. 5 Min." · "2 Std.", spelled out so the label reads like a sentence ("FERTIG  seit
/// 6 Min.", Kevin 2026-09-26).
pub fn duration_text(seconds: f64) -> String {
    // Swift `max(0, Int(seconds))`: truncates toward zero (a clock twitch is never negative).
    let s = (seconds as i64).max(0);
    if s < 60 {
        return tr("gerade eben", "just now").to_owned();
    }
    let (h, m) = (s / 3600, s % 3600 / 60);
    let (de, en) = match (h, m) {
        (0, m) => (format!("{m} Min."), format!("{m} min")),
        (h, 0) => (format!("{h} Std."), format!("{h} h")),
        (h, m) => (format!("{h} Std. {m} Min."), format!("{h} h {m} min")),
    };
    tr(&de, &en).to_owned()
}

/// First name of the greeting that `first_sentence` removes (GP 9): at start `set_greeting_name(&first_name(full user
/// name))`. Unit tests pin it to "Kevin" per test thread, like Swift's test runner. Empty = remove nothing.
pub fn set_greeting_name(name: &str) {
    greeting::set(name);
}

pub fn greeting_name() -> String {
    greeting::get()
}

#[cfg(not(test))]
mod greeting {
    use std::sync::{PoisonError, RwLock};

    static NAME: RwLock<String> = RwLock::new(String::new());

    pub fn set(name: &str) {
        *NAME.write().unwrap_or_else(PoisonError::into_inner) = name.to_owned();
    }

    pub fn get() -> String {
        NAME.read().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

#[cfg(test)]
mod greeting {
    use std::cell::RefCell;

    thread_local! {
        static NAME: RefCell<String> = RefCell::new("Kevin".to_owned());
    }

    pub fn set(name: &str) {
        NAME.with(|n| *n.borrow_mut() = name.to_owned());
    }

    pub fn get() -> String {
        NAME.with(|n| n.borrow().clone())
    }
}

/// First word of the full macOS user name: "Kevin Theobald" -> "Kevin" (the short user name would be "kevin").
pub fn first_name(full_name: &str) -> String {
    full_name
        .split(char::is_whitespace)
        .find(|p| !p.is_empty())
        .unwrap_or("")
        .to_owned()
}

/// Longest first sentence a label shows, in characters (Swift's `limit`, never changed by a caller).
const SENTENCE_LIMIT: usize = 160;

/// First sentence of an answer, fit for a label: markdown gone, lines joined, the greeting "Kevin, …" gone (nearly every
/// answer starts so, Kevin's rule), at most `SENTENCE_LIMIT` characters.
pub fn first_sentence(text: &str) -> String {
    // Markdown at the line start ("## ", "- ", "> ") only with a space after it: "#12" and "-5" stay (2026-09-27).
    let lines: Vec<&str> = text
        .split(is_newline)
        .map(|raw| {
            let line = trim_ws(raw);
            let rest = line.trim_start_matches(['#', '>', '-', '*']);
            if rest.len() < line.len() && (rest.is_empty() || rest.starts_with(' ')) {
                trim_ws(rest)
            } else {
                line
            }
        })
        .filter(|l| !l.is_empty())
        .collect();
    let mut s = lines.join(" ").replace("**", "").replace('`', "");
    if let Some(rest) = after_greeting(&s, &greeting_name()) {
        let first = graphemes(rest).next().unwrap_or("");
        s = format!("{}{}", first.to_uppercase(), &rest[first.len()..]);
    }
    // One pass, up to the first sentence end ("." "!" "?" before a space or the end) and never further than the limit
    // needs: before, two full character lists of the whole text were built on every label refresh (review quick
    // 2026-11-06, Codex).
    let (kept, count) = {
        let (mut kept, mut count) = (0, 0);
        let mut chars = graphemes(&s).peekable();
        while let Some(c) = chars.next() {
            kept += c.len();
            count += 1;
            let end = matches!(c, "." | "!" | "?") && chars.peek().is_none_or(|n| *n == " ");
            if end || count > SENTENCE_LIMIT {
                break;
            }
        }
        (kept, count)
    };
    if count > SENTENCE_LIMIT {
        let cut: usize = graphemes(&s).take(SENTENCE_LIMIT - 1).map(str::len).sum();
        return format!("{}…", &s[..cut]);
    }
    s.truncate(kept);
    s
}

/// The text after a greeting: the name at the very start (case-insensitive) and the next non-blank character neither
/// letter nor digit. "Kevin, …" and "Kevin 👋 …" go, "Kevins Plan" and "Will this work?" (name Will) stay. What follows
/// starts at the first letter, digit or "#".
fn after_greeting<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    if name.is_empty() {
        return None;
    }
    let target: String = name.chars().map(fold_char).collect();
    let mut folded = String::new();
    let mut end = None;
    for (i, c) in s.char_indices() {
        folded += &fold_char(c);
        if folded == target {
            end = Some(i + c.len_utf8());
            break;
        }
        if !target.starts_with(&folded) {
            return None;
        }
    }
    let rest = &s[end?..];
    let next = rest.chars().find(|c| !c.is_whitespace())?;
    if next.is_alphabetic() || next.is_numeric() {
        return None;
    }
    let skip: usize = graphemes(rest)
        .take_while(|c| *c != "#" && c.chars().next().is_some_and(|f| !f.is_alphanumeric()))
        .map(str::len)
        .sum();
    Some(&rest[skip..])
}

/// One character case-folded like Foundation's `.caseInsensitive` (lowercase, "ß" as "ss", see `model::fold_case`).
fn fold_char(c: char) -> String {
    c.to_lowercase().collect::<String>().replace('ß', "ss")
}

/// Swift `Character.isNewline`.
fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\u{B}' | '\u{C}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// What a label says. One pure function, so label, list and menu say the same.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelContent {
    pub tone: LabelTone,
    pub duration: Option<String>,
    pub topic: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
}

impl LabelContent {
    /// Lines of the label: status (+ duration), topic, step or result, hint.
    pub fn lines(&self) -> Vec<String> {
        let status = format!(
            "{} {}{}",
            self.tone.symbol(),
            self.tone.word(),
            self.duration
                .as_ref()
                .map_or_else(String::new, |d| format!("  {d}"))
        );
        let mut out = vec![status, self.topic.clone()];
        out.extend(self.detail.iter().chain(&self.hint).cloned());
        out
    }
}

/// `entry`: since entry of the visible session, only counted if it means the same state. `line`: the Lookout line of the
/// visible session (Swift `Digest.line`). If the visible tab is not recognisable, the topic comes from the window title
/// and there is no step: the stored one may belong to the hidden tab.
pub fn label_content(
    w: &TermWindow,
    entry: Option<&SinceEntry>,
    line: Option<&LookoutLine>,
    now: Time,
) -> LabelContent {
    let tone = window_tone(w);
    let s = w.visible_session();
    let d = s.and_then(|s| s.digest.as_ref());
    let duration = held(entry, tone, now).map(|secs| {
        if secs < 60.0 {
            duration_text(secs)
        } else {
            format!("{}{}", tr("seit ", "for "), duration_text(secs))
        }
    });
    // v4: the own name ("Project · task", naming hook) carries the project already, no folder after it. Kevin 2026-10-12
    // ("immer klar sehen, welche Session welche ist"): never Claude's topic, it keeps changing ("Ja mach das ·
    // ClaudeCode"). Without an own name a Claude session shows only its folder until the hook names it.
    let name = d.and_then(|d| d.name.as_deref());
    let folder = if name.is_none() {
        last_path_component(s.map_or("", |s| s.cwd.as_str()))
    } else {
        ""
    };
    let own = name.unwrap_or_else(|| {
        if s.is_some_and(|s| s.engine == Engine::Claude) {
            ""
        } else {
            title_glyph(&w.title).1
        }
    });
    let mut parts: Vec<&str> = vec![];
    for p in [own, folder] {
        if !p.is_empty() && !parts.contains(&p) {
            parts.push(p);
        }
    }
    // Lookout line (Kevin 2026-10-12): what the session says about its own state goes before the first sentence.
    let reason = d.and(line).map(|l| l.reason.clone());
    let text = || d.and_then(|d| d.text.as_deref()).map(first_sentence);
    let step = |prefix: &str| {
        d.and_then(|d| d.step.as_ref())
            .map(|st| format!("{prefix}{st}"))
    };
    let detail = match tone {
        LabelTone::Waiting => d
            .and_then(|d| d.question.as_ref())
            .map(|q| tr(&format!("„{q}“"), &format!("“{q}”")).to_owned())
            .or(reason)
            .or_else(|| step(tr("Freigabe: ", "Approval: ")))
            .or_else(text),
        LabelTone::Ready | LabelTone::Watching => reason.or_else(text),
        // No "denkt nach" alternating with the step, it flickered every 6 s. Kevin 2026-10-12 evening: the step line is
        // back (the calm version of the midday was taken back, without it one no longer saw what the session does).
        LabelTone::Running => {
            if d.is_some_and(|d| d.last == LastBlock::Text) {
                text()
            } else {
                step(tr("jetzt: ", "now: "))
            }
        }
    };
    let fallback = w
        .sessions
        .first()
        .map_or(w.app.as_str(), |s| s.engine.raw());
    LabelContent {
        tone,
        duration,
        topic: if parts.is_empty() {
            fallback.to_owned()
        } else {
            parts.join(" · ")
        },
        detail,
        hint: w
            .hidden_question()
            .then(|| tr("Tab dahinter hat eine Frage", "A hidden tab has a question").to_owned()),
    }
}

/// Seconds the window has been in `tone`: only a since entry of that same state counts.
fn held(entry: Option<&SinceEntry>, tone: LabelTone, now: Time) -> Option<f64> {
    entry
        .filter(|e| e.tone == tone)
        .and_then(|e| e.since)
        .map(|start| now - start)
}

/// `label_content` with the since entry and Lookout line of the visible session.
fn window_label(
    w: &TermWindow,
    since: &BTreeMap<String, SinceEntry>,
    lookout: &BTreeMap<String, LookoutLine>,
    now: Time,
) -> LabelContent {
    let visible = w.visible_session();
    label_content(
        w,
        visible.and_then(|s| since.get(&s.id)),
        visible.and_then(|s| lookout.get(&s.id)),
        now,
    )
}

/// Height of the tab or title bar; the label sits right below it so the tabs stay visible (Kevin 2026-09-21). Warp
/// measured 2026-09-21: traffic lights 12 pt below the window's top, 16 pt high, so the bar is ~40 pt. Ghostty and others
/// [ANNAHME]: native title bar 28 pt.
pub fn label_top_inset(bundle_id: &str) -> f64 {
    if bundle_id == WARP_BID { 40.0 } else { 28.0 }
}

pub const LABEL_PADDING: f64 = 9.0;
/// Nearly opaque (Kevin 2026-09-26): at 58 % the status line's contrast over light terminal text fell to 2.8:1, from
/// 92 % it stays at least 9.5:1 everywhere (WCAG formula, recalculated).
pub const LABEL_BG_ALPHA: f64 = 0.92;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontWeight {
    Regular,
    Semibold,
    Bold,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelLineStyle {
    pub size: f64,
    pub weight: FontWeight,
    pub height: f64,
}

/// Font per label line (Kevin 2026-09-26, proposal B "größer und klarer"; before all lines 14 pt): status bold and name
/// 16 pt each, step/result/question and hint below them smaller.
pub fn label_line_style(i: usize) -> LabelLineStyle {
    match i {
        0 => LabelLineStyle {
            size: 16.0,
            weight: FontWeight::Bold,
            height: 22.0,
        },
        1 => LabelLineStyle {
            size: 16.0,
            weight: FontWeight::Semibold,
            height: 22.0,
        },
        _ => LabelLineStyle {
            size: 13.5,
            weight: FontWeight::Regular,
            height: 18.0,
        },
    }
}

/// Label rectangle (Cocoa): full window width minus a margin, right below the bar.
pub fn label_rect(frame: Rect, bundle_id: &str, lines: usize) -> Rect {
    let h = LABEL_PADDING * 2.0 + (0..lines).map(|i| label_line_style(i).height).sum::<f64>();
    Rect::new(
        frame.min_x() + 6.0,
        frame.max_y() - label_top_inset(bundle_id) - h - 2.0,
        (frame.width() - 12.0).max(0.0),
        h,
    )
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabelTarget {
    pub id: String,
    /// Cocoa.
    pub rect: Rect,
    pub window_number: Option<i64>,
    pub content: LabelContent,
}

/// Labels: every AI window of `overlay_candidates` (so never the active session), done or not, also fully tiled ones.
/// Windows too small for a label below the bar get none. `lookout`: session id -> its Lookout line.
pub fn label_targets(
    o: &OverlayInput,
    since: &BTreeMap<String, SinceEntry>,
    lookout: &BTreeMap<String, LookoutLine>,
    now: Time,
) -> Vec<LabelTarget> {
    overlay_candidates(o, false)
        .into_iter()
        .filter_map(|(i, window_number)| {
            let w = &o.windows[i];
            let first = w.sessions.first()?;
            let content = window_label(w, since, lookout, now);
            let rect = label_rect(w.frame, &w.bundle_id, content.lines().len());
            (rect.width() >= 220.0 && rect.min_y() >= w.frame.min_y() + 20.0).then(|| LabelTarget {
                id: first.id.clone(),
                rect,
                window_number,
                content,
            })
        })
        .collect()
}

// ---------- Menu and list lines (plan 1d, GP 6) ----------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionLine {
    pub title: String,
    pub subtitle: Option<String>,
    pub tone: LabelTone,
}

/// Menu and list line of a window: the same as its label (Kevin 2026-09-21), the step below it.
pub fn session_line(
    w: &TermWindow,
    since: &BTreeMap<String, SinceEntry>,
    lookout: &BTreeMap<String, LookoutLine>,
    now: Time,
) -> SessionLine {
    let c = window_label(w, since, lookout, now);
    let sub: Vec<&str> = c.detail.iter().chain(&c.hint).map(String::as_str).collect();
    let duration = c
        .duration
        .as_ref()
        .map_or_else(String::new, |d| format!(" — {d}"));
    SessionLine {
        title: format!("{} {}{duration}{}", c.tone.symbol(), c.topic, tabs_note(w)),
        subtitle: (!sub.is_empty()).then(|| sub.join(" · ")),
        tone: c.tone,
    }
}

/// "  (+1 Tab)" behind the name of a window with more than one session.
fn tabs_note(w: &TermWindow) -> String {
    let more = w.sessions.len().saturating_sub(1);
    if more == 0 {
        return String::new();
    }
    format!(
        "  (+{more} {})",
        tr("Tab", if more == 1 { "tab" } else { "tabs" })
    )
}

/// Longest line below a name in the menu. AppKit wraps a subtitle at about 290 pt whatever the menu's width (measured
/// 2026-11-06 on macOS 27: 48 characters fit, from 52 the row grew); one line keeps every row the same height.
pub const MENU_SUBTITLE_LIMIT: usize = 46;
/// Rows of the parking lot in the menu before "… and N more".
pub const MENU_LOT_ROWS: usize = 3;
/// Rows of the other sections together, as before the sections: 25 two-line rows would outgrow a MacBook screen.
pub const MENU_ROWS_MAX: usize = 20;

/// A session row in the menu (design A "nach Status gruppiert", Kevin 2026-11-06 evening): the name in front; below it
/// the project and what the session does or asks; on the right how long it has been so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    /// The task of the own name "Project · task" (naming hook), else the label's topic; "(+1 Tab)" for more tabs.
    pub name: String,
    pub project: Option<String>,
    /// "überwacht" for a watching one (it stands with the working ones), then the label's step, question or result and
    /// the hidden-tab hint.
    pub state: Vec<String>,
    /// Without "seit": a badge on its own says it.
    pub duration: Option<String>,
    pub tone: LabelTone,
}

impl MenuRow {
    /// Below the name in the session list: project, `notes` (where the window is), state.
    pub fn subtitle(&self, notes: &[String]) -> Option<String> {
        one_line(self.project.iter().chain(notes).chain(&self.state))
    }

    /// Below the name in the window lists of the submenus: project and `notes`; the dot shows the state.
    pub fn project_line(&self, notes: &[String]) -> Option<String> {
        one_line(self.project.iter().chain(notes))
    }

    /// A compact row of the parking lot: "task · project".
    pub fn compact(&self) -> String {
        match &self.project {
            Some(p) => format!("{} · {p}", self.name),
            None => self.name.clone(),
        }
    }
}

/// The non-empty parts joined by " · ", at most `MENU_SUBTITLE_LIMIT` wide (`width_of`).
fn one_line<'a>(parts: impl Iterator<Item = &'a String>) -> Option<String> {
    let line = parts
        .filter(|p| !p.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" · ");
    if line.is_empty() {
        return None;
    }
    if graphemes(&line).map(width_of).sum::<usize>() <= MENU_SUBTITLE_LIMIT {
        return Some(line);
    }
    let (mut kept, mut used) = (String::new(), 0);
    for g in graphemes(&line) {
        used += width_of(g);
        if used > MENU_SUBTITLE_LIMIT - 1 {
            break;
        }
        kept.push_str(g);
    }
    Some(format!("{}…", kept.trim_end()))
}

/// Width of a character in Latin letters: East Asian wide letters and emoji take about two (review quick 2026-11-06).
/// ponytail: the main Unicode blocks, no font metrics in the core; measure in the macOS layer if a script still wraps.
fn width_of(g: &str) -> usize {
    let wide = g.chars().next().is_some_and(|c| {
        matches!(u32::from(c),
            0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x1F300..=0x1FAFF | 0x20000..=0x3FFFD)
    });
    if wide { 2 } else { 1 }
}

/// The menu row of a window: the words of its label (`label_content`), arranged for the menu.
pub fn menu_row(
    w: &TermWindow,
    since: &BTreeMap<String, SinceEntry>,
    lookout: &BTreeMap<String, LookoutLine>,
    now: Time,
) -> MenuRow {
    let visible = w.visible_session();
    let entry = visible.and_then(|s| since.get(&s.id));
    let c = label_content(w, entry, visible.and_then(|s| lookout.get(&s.id)), now);
    let own = visible
        .and_then(|s| s.digest.as_ref())
        .and_then(|d| d.name.as_deref());
    let (project, name) = match own.and_then(|n| n.split_once(" · ")) {
        Some((project, task)) => (Some(project.to_owned()), task.to_owned()),
        None => (None, c.topic),
    };
    let watching = (c.tone == LabelTone::Watching).then(|| c.tone.word().to_lowercase());
    MenuRow {
        name: name + &tabs_note(w),
        project,
        state: watching.into_iter().chain(c.detail).chain(c.hint).collect(),
        duration: held(entry, c.tone, now).map(duration_text),
        tone: c.tone,
    }
}

/// Sections of the session list in the menu (design A): waiting for Kevin, done, working (running and watching), and
/// the parking lot apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuGroup {
    Waiting,
    Ready,
    Working,
    Lot,
}

impl MenuGroup {
    /// The section's head with its count: AppKit draws no badge on a section header (measured 2026-11-06).
    pub fn header(self, count: usize) -> String {
        let title = match self {
            MenuGroup::Waiting => LabelTone::Waiting.word(),
            MenuGroup::Ready => LabelTone::Ready.word(),
            MenuGroup::Working => LabelTone::Running.word(),
            MenuGroup::Lot => tr("AUF DEM PARKPLATZ", "IN THE PARKING LOT"),
        };
        format!("{title} · {count}")
    }
}

/// The windows of `order` (attention order) by section, keeping that order; `lot`: the windows on the parking lot now
/// (only running or watching ones park, `plan_arrangement`). Empty sections are left out.
pub fn menu_groups(
    windows: &[TermWindow],
    order: &[usize],
    lot: &BTreeSet<usize>,
) -> Vec<(MenuGroup, Vec<usize>)> {
    let group = |i: usize| {
        if lot.contains(&i) {
            return MenuGroup::Lot;
        }
        match window_tone(&windows[i]) {
            LabelTone::Waiting => MenuGroup::Waiting,
            LabelTone::Ready => MenuGroup::Ready,
            LabelTone::Running | LabelTone::Watching => MenuGroup::Working,
        }
    };
    [
        MenuGroup::Waiting,
        MenuGroup::Ready,
        MenuGroup::Working,
        MenuGroup::Lot,
    ]
    .into_iter()
    .map(|g| {
        (
            g,
            order.iter().copied().filter(|&i| group(i) == g).collect(),
        )
    })
    .filter(|(_, rows): &(MenuGroup, Vec<usize>)| !rows.is_empty())
    .collect()
}

/// Rows a section shows of its `n` within `cap`: all up to one more than the cap (a single rest row costs the same
/// line as the hint), else `cap` and then `more_rows_text`.
pub fn rows_shown(n: usize, cap: usize) -> usize {
    if n <= cap + 1 { n } else { cap }
}

/// The last line of a cut section.
pub fn more_rows_text(n: usize) -> String {
    format!("… {} {n} {}", tr("und", "and"), tr("weitere", "more"))
}

/// A list line, ready to draw. `ids`: the sessions of the window (for the click).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListLine {
    Header(String),
    Row {
        ids: Vec<String>,
        title: String,
        detail: Option<String>,
        tone: LabelTone,
    },
    More(usize),
}

impl ListLine {
    pub fn height(&self) -> f64 {
        if matches!(self, ListLine::Header(_)) {
            LIST_HEADER_H
        } else {
            LIST_ROW_H
        }
    }
}

/// Text of the line "+N more".
pub fn more_text(n: usize) -> String {
    format!("+{n} {}", tr("weitere (im Menü)", "more (in the menu)"))
}

/// Session ids of the row at `y` (points from the top of the list), for a click; `None` off the rows.
pub fn row_at(lines: &[ListLine], y: f64) -> Option<&[String]> {
    let mut top = 0.0;
    for line in lines {
        if y >= top
            && y < top + line.height()
            && let ListLine::Row { ids, .. } = line
        {
            return Some(ids);
        }
        top += line.height();
    }
    None
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListTarget {
    /// Cocoa.
    pub rect: Rect,
    pub lines: Vec<ListLine>,
    /// CGWindowID of the main window: the list lies right above it, other windows in front cover it too.
    pub window_number: i64,
}

/// Index of the last plan -> index in the current snapshot. `planned`: window ids in plan order.
pub fn current_index(planned: &[u64], i: usize, windows: &[TermWindow]) -> Option<usize> {
    planned.get(i).and_then(|&id| index_of_window(id, windows))
}

/// Content of a list panel (Main.swift `refreshLists`): a side list of the last plan with the current windows; a row
/// whose window is gone is left out.
pub fn list_target(
    list: &SideList,
    planned: &[u64],
    windows: &[TermWindow],
    since: &BTreeMap<String, SinceEntry>,
    lookout: &BTreeMap<String, LookoutLine>,
    now: Time,
    window_number: i64,
) -> ListTarget {
    let lines = list
        .entries
        .iter()
        .filter_map(|e| match e {
            ListEntry::Header(t) => Some(ListLine::Header(t.clone())),
            ListEntry::More(n) => Some(ListLine::More(*n)),
            ListEntry::Row(i) => {
                let w = &windows[current_index(planned, *i, windows)?];
                let line = session_line(w, since, lookout, now);
                Some(ListLine::Row {
                    ids: w.sessions.iter().map(|s| s.id.clone()).collect(),
                    title: line.title,
                    detail: line.subtitle,
                    tone: line.tone,
                })
            }
        })
        .collect();
    ListTarget {
        rect: list.rect,
        lines,
        window_number,
    }
}

// ---------- Preview outlines while hovering in the menu (Kevin 2026-09-18) ----------

/// Shows where the real windows WOULD land before the user switches.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewBox {
    /// Cocoa.
    pub rect: Rect,
    pub title: String,
    pub detail: String,
}

/// Outlines of a planned arrangement: every placed window, in the label's words. Parked windows (GP 2) give no pile of
/// outlines on the main window but one box "List (N)" where the list goes. `active`: index of the main window now.
pub fn preview_boxes(
    a: &Arrangement,
    windows: &[TermWindow],
    active: Option<usize>,
) -> Vec<PreviewBox> {
    let mut boxes: Vec<PreviewBox> = a
        .frames
        .iter()
        .filter(|(i, _)| !a.parked.contains(i))
        .filter_map(|(&i, &rect)| {
            let w = windows.get(i)?;
            let state = if Some(i) == active {
                tr("aktiv", "active").to_owned()
            } else {
                window_tone(w).word().to_lowercase()
            };
            let engine = w
                .sessions
                .first()
                .map_or(w.app.as_str(), |s| s.engine.raw());
            Some(PreviewBox {
                rect,
                title: w
                    .sessions
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" · "),
                detail: format!("{engine} · {state}"),
            })
        })
        .collect();
    boxes.extend(a.lists.iter().map(|l| {
        PreviewBox {
            rect: l.rect,
            title: format!("{} ({})", tr("Liste", "List"), l.count()),
            detail: tr(
                "Fenster liegen hinter der Mitte",
                "windows lie behind the center",
            )
            .to_owned(),
        }
    }));
    boxes
}

// ---------- Focus ----------

/// Events that arrive during an own move: noted, handled once after the move. Clicks were lost otherwise, the focus
/// report came during the glide and was dropped as an own move (Kevin 2026-09-21: "hat eine laengere Zeit gedauert, bis
/// ich es in die Mitte bekommen konnte"). Since 2026-09-25 also arranging: during the glide the remembered frames are
/// wrong, a new plan let windows jump back to their start (GLM + Kimi).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MoveGate {
    pending: bool,
}

impl MoveGate {
    pub fn pending(self) -> bool {
        self.pending
    }

    /// A focus report. true = handle it now; false = noted (Lookout moves windows itself).
    pub fn event(&mut self, self_move: bool) -> bool {
        self.pending = self_move;
        !self_move
    }

    /// Move ended or cancelled. true = handle the noted click now.
    pub fn move_ended(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }
}

/// Whose focus counts? The frontmost terminal. While no active session is known yet (start), also a terminal with AI
/// windows that is not in front: its last focused window is the session the user sat at last (AX gives it in the
/// background too). Live 2026-09-25 after the rebuild: Chrome was in front and some window stood with a mint frame in
/// the middle until the user clicked a terminal, with autostart after every login.
pub fn focus_source(
    front_pid: Option<Pid>,
    terminal_pids: &[Pid],
    ai_terminal_pids: &[Pid],
    active_known: bool,
) -> Option<Pid> {
    match front_pid {
        Some(f) if terminal_pids.contains(&f) => Some(f),
        _ if active_known => None,
        _ => ai_terminal_pids.first().copied(),
    }
}

// ---------- Moving (Animator.swift) ----------
//
// Design rule (Kevin 2026-09-18): calm but informing. Time-based instead of fixed steps (AX writes cost milliseconds to
// tenths under load: the steps get coarser, the duration stays). Ease-in-out, a staggered start per window (six windows
// jumping at once are chaos, a running wave reads as one calm movement), no overshoot, and on the way only moving
// (`glide_path`): shrink at the old place when its own move starts, grow at the target on arrival.

/// Total duration of a move: slow enough to follow, short enough not to feel sluggish.
pub const MOVE_DURATION: f64 = 0.45;
/// Start offset per window in order, capped so the wave does not sprawl.
pub const MOVE_STAGGER: f64 = 0.045;
pub const MOVE_STAGGER_MAX: f64 = 0.27;
/// Frame interval. If AX cannot keep up, the steps get coarser instead of slower.
pub const FRAME_INTERVAL: f64 = 1.0 / 60.0;

/// Cubic ease-in-out: soft start, soft arrival, no overshoot.
pub fn eased(t: f64) -> f64 {
    // Swift `min(1, max(0, t))` makes NaN 0; `clamp` would keep it.
    let c = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    if c < 0.5 {
        4.0 * c * c * c
    } else {
        1.0 - (-2.0 * c + 2.0).powf(3.0) / 2.0
    }
}

pub fn lerp(a: Rect, b: Rect, p: f64) -> Rect {
    Rect::new(
        a.min_x() + (b.min_x() - a.min_x()) * p,
        a.min_y() + (b.min_y() - a.min_y()) * p,
        a.width() + (b.width() - a.width()) * p,
        a.height() + (b.height() - a.height()) * p,
    )
}

/// One step for window `i` of the moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GlideStep {
    /// Its move starts: shrink in place (top-left stays) to the size it glides in.
    Shrink(usize, Size),
    /// On the way: move it to this rectangle (Cocoa); its frame follows (Kevin 2026-09-25).
    Move(usize, Rect),
    /// Arrived: set the target frame exactly and read it back (convention 4).
    Arrive(usize),
}

/// Timing of one move of several windows. Each window shrinks at ITS start, glides, and is set exactly at ITS arrival:
/// all at once at the start or end would be one jolt that breaks the wave (review quick). A new move replaces a running
/// one (the caller's generation token).
#[derive(Clone, Debug, PartialEq)]
pub struct Glide {
    froms: Vec<Rect>,
    paths: Vec<(Rect, Rect)>,
    delays: Vec<f64>,
    duration: f64,
    started: BTreeSet<usize>,
    finished: BTreeSet<usize>,
}

impl Glide {
    /// `moves`: (from, to) per window, Cocoa; `duration` usually `MOVE_DURATION`.
    pub fn new(moves: &[(Rect, Rect)], duration: f64) -> Glide {
        Glide {
            froms: moves.iter().map(|m| m.0).collect(),
            paths: moves
                .iter()
                .map(|&(from, to)| glide_path(from, to))
                .collect(),
            delays: (0..moves.len())
                .map(|i| (i as f64 * MOVE_STAGGER).min(MOVE_STAGGER_MAX))
                .collect(),
            duration: duration.max(0.05),
            started: BTreeSet::new(),
            finished: BTreeSet::new(),
        }
    }

    /// Steps of the frame `elapsed` seconds after the start, in window order.
    pub fn frame(&mut self, elapsed: f64) -> Vec<GlideStep> {
        let mut steps = vec![];
        for i in 0..self.paths.len() {
            if self.finished.contains(&i) {
                continue;
            }
            let local = (elapsed - self.delays[i]) / self.duration;
            // This window still waits for its start.
            if local.is_nan() || local <= 0.0 {
                continue;
            }
            if local >= 1.0 {
                self.finished.insert(i);
                steps.push(GlideStep::Arrive(i));
                continue;
            }
            let (start, end) = self.paths[i];
            if self.started.insert(i) && start.size() != self.froms[i].size() {
                steps.push(GlideStep::Shrink(i, start.size()));
            }
            steps.push(GlideStep::Move(i, lerp(start, end, eased(local))));
        }
        steps
    }

    /// Every window arrived.
    pub fn done(&self) -> bool {
        self.finished.len() == self.paths.len()
    }
}

#[cfg(test)]
mod tests;
