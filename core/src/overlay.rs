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
use crate::layout::{
    LIST_HEADER_H, LIST_ROW_H, ReadAnswers, is_read, is_snoozed, pin_rank, screen_index,
};
use crate::mapping::{index_of_window, last_path_component};
use crate::model::{
    Arrangement, CardButton, Engine, LabelTone, LastBlock, ListEntry, LookoutLine, OnScreenWindow,
    Pid, Session, SideList, TermWindow, Time, graphemes, title_glyph, trim_ws,
};
use crate::motion::{WARP_BID, ax_origin, cocoa_to_ax, glide_path};
use crate::planned::{self, NaiveDate, PlannedItem};
use crate::team::{self, Team};
use chrono::Datelike;

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
    /// Its turn was broken off (Kevin 2026-11-10): the frame in the stopped colour.
    pub stopped: bool,
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
                stopped: w.is_stopped(),
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
    } else if s.is_stopped() {
        // Its own clock, like every state (review quick 2026-11-10, GLM: without it a stopped card showed no time).
        LabelTone::Stopped
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
    } else if w.is_stopped() {
        LabelTone::Stopped
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
    let name = s
        .and_then(|s| s.title.as_deref())
        .or_else(|| d.and_then(|d| d.name.as_deref()));
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
        // Broken off (Kevin 2026-11-10): where it stopped, so the nudge knows where to go on.
        LabelTone::Stopped => step(tr("angehalten bei: ", "stopped at: ")).or_else(text),
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
    let own = visible.and_then(|s| {
        s.title
            .as_deref()
            .or_else(|| s.digest.as_ref().and_then(|d| d.name.as_deref()))
    });
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
            // A stopped one needs a nudge from Kevin (Kevin 2026-11-10, "1a").
            LabelTone::Waiting | LabelTone::Stopped => MenuGroup::Waiting,
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

/// One session in the inbox (Kevin 2026-11-09, preview "Eingang"). `open`: its terminal is the one in front, so it shows
/// the whole question over `lines` lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InboxCard {
    pub ids: Vec<String>,
    pub name: String,
    pub project: Option<String>,
    pub since: Option<String>,
    pub what: Option<String>,
    pub tone: LabelTone,
    pub open: bool,
    pub lines: usize,
    /// What a right-click offers.
    pub menu: Vec<CardButton>,
    /// Pinned by Kevin (`pin_rank`): a pin before the name.
    pub pinned: bool,
    /// A planned item, not a session (Planned area): its mark replaces the status symbol; no ids, no menu.
    pub mark: Option<PlanMark>,
}

/// What a planned item is, on its card and as a dot in the week strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanMark {
    /// Opens a session by itself at its time.
    Start,
    /// A task due that day, started by hand.
    Task,
    /// A start that did not happen.
    Problem,
    /// A hard deadline that day.
    Deadline,
}

/// The counts of the inbox's areas. `planned`: `None` = no planner configured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InboxCounts {
    pub waiting: usize,
    pub working: usize,
    pub parked: usize,
    pub planned: Option<usize>,
}

/// One tile of the overview: an area with its count and up to two lines of what is in it. `hot`: somebody waits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InboxTile {
    pub tab: InboxTab,
    /// The team of a team's tile.
    pub team: Option<String>,
    pub title: String,
    pub count: usize,
    pub lines: Vec<String>,
    pub hot: bool,
}

/// The Planned area's week: its title and the seven days.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeekStrip {
    pub title: String,
    pub days: Vec<WeekDay>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeekDay {
    pub date: NaiveDate,
    /// "Di".
    pub name: String,
    pub number: u32,
    pub today: bool,
    pub selected: bool,
    /// A dot per card the day lists, at most four: problems and deadlines first, then starts, then tasks.
    pub dots: Vec<PlanMark>,
}

/// Where a click in the inbox leads (`nav_at`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InboxNav {
    Open(InboxTab),
    /// A team's area.
    Team(String),
    /// A day in the week strip.
    Day(NaiveDate),
    /// A week back (-1) or ahead (+1).
    Week(i64),
}

/// A list line, ready to draw. `ids`: the sessions of the window (for the click). The first three are the side lists,
/// the others the inbox.
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
    /// The inbox's head (Kevin 2026-11-10, "H"): on the overview its title, in an area a way back, the area's name and
    /// count; the + at the right.
    InboxHead {
        tab: InboxTab,
        counts: InboxCounts,
        /// In a team's area: its name and how many windows it has.
        team: Option<(String, usize)>,
    },
    /// In every area but Waiting while somebody waits: an amber strip with the first of them; a click opens Waiting.
    WaitBar {
        n: usize,
        text: String,
    },
    /// One row of the overview: one tile across or two side by side.
    Tiles(Vec<InboxTile>),
    /// The Planned area's week strip.
    Week(WeekStrip),
    Group(String),
    Card(InboxCard),
    /// The open card's buttons, side by side.
    Buttons {
        ids: Vec<String>,
        buttons: Vec<CardButton>,
    },
    /// The open card's note when the session asks.
    Hint(String),
    Empty(String),
}

/// What the inbox shows (Kevin 2026-11-10, "H"): the overview with one tile per area, or one area. The areas: who wants
/// something from Kevin, what works, what he put away, what is planned, and one per team (`InboxInput::team` says which).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InboxTab {
    #[default]
    Overview,
    Waiting,
    Working,
    Parked,
    Planned,
    Team,
}

/// The head: the way back to the overview from this x, this wide; the + for a new terminal window at the right.
pub const INBOX_BACK_X: f64 = 10.0;
pub const INBOX_BACK_W: f64 = 116.0;
pub const INBOX_PLUS_X: f64 = 370.0;
pub const INBOX_PLUS_W: f64 = 36.0;
/// The overview's tiles: margin and gap, and a tile's height (its row adds the gap below).
pub const INBOX_TILE_X: f64 = 10.0;
pub const INBOX_TILE_GAP: f64 = 8.0;
pub const INBOX_TILE_H: f64 = 86.0;
/// The week strip: arrows in its top row, the days below.
pub const INBOX_WEEK_H: f64 = 98.0;
pub const INBOX_ARROW_W: f64 = 30.0;
const INBOX_WEEK_DAYS_TOP: f64 = 36.0;
/// The buttons of an open card lie side by side from `INBOX_TEXT_X`.
pub const INBOX_BUTTON_W: f64 = 150.0;
pub const INBOX_BUTTON_GAP: f64 = 8.0;

/// Where the text of a card starts, from the card's top: name and project above it.
pub const INBOX_WHAT_TOP: f64 = 47.0;
/// One line of the question on an open card.
pub const INBOX_OPEN_LINE_H: f64 = 23.0;
/// Average width of a character of the open card's question (16.5 pt), on the high side so the text is never clipped.
const INBOX_OPEN_CHAR_W: f64 = 8.8;
/// Left of a card's text: status symbol. Right of it: margin.
pub const INBOX_TEXT_X: f64 = 50.0;
pub const INBOX_MARGIN: f64 = 16.0;
const INBOX_OPEN_LINES_MAX: usize = 6;

impl ListLine {
    pub fn height(&self) -> f64 {
        match self {
            ListLine::Header(_) | ListLine::Group(_) => LIST_HEADER_H,
            ListLine::Row { .. } | ListLine::More(_) => LIST_ROW_H,
            ListLine::InboxHead { .. } => 44.0,
            ListLine::WaitBar { .. } => 36.0,
            ListLine::Tiles(_) => INBOX_TILE_H + INBOX_TILE_GAP,
            ListLine::Week(_) => INBOX_WEEK_H,
            ListLine::Card(c) if c.open => {
                INBOX_WHAT_TOP + c.lines as f64 * INBOX_OPEN_LINE_H + 8.0
            }
            ListLine::Card(_) => 76.0,
            ListLine::Buttons { .. } => 40.0,
            ListLine::Hint(_) => 30.0,
            ListLine::Empty(_) => 160.0,
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
        if y >= top && y < top + line.height() {
            match line {
                ListLine::Row { ids, .. } => return Some(ids),
                ListLine::Card(c) => return Some(&c.ids),
                _ => {}
            }
        }
        top += line.height();
    }
    None
}

/// The inbox card at `y`; `None` off the cards.
pub fn card_at(lines: &[ListLine], y: f64) -> Option<&InboxCard> {
    let mut top = 0.0;
    for line in lines {
        if y >= top
            && y < top + line.height()
            && let ListLine::Card(c) = line
        {
            return Some(c);
        }
        top += line.height();
    }
    None
}

/// The tiles of a row across `width`: (x, width) each, one across or two side by side.
pub fn tile_spans(n: usize, width: f64) -> Vec<(f64, f64)> {
    let inner = width - 2.0 * INBOX_TILE_X;
    if n <= 1 {
        return vec![(INBOX_TILE_X, inner)];
    }
    let w = (inner - INBOX_TILE_GAP) / 2.0;
    vec![(INBOX_TILE_X, w), (INBOX_TILE_X + w + INBOX_TILE_GAP, w)]
}

/// The week strip's arrows (x, width): back at the left, ahead at the right.
pub fn week_arrows(width: f64) -> [(f64, f64); 2] {
    [
        (INBOX_TILE_X, INBOX_ARROW_W),
        (width - INBOX_TILE_X - INBOX_ARROW_W, INBOX_ARROW_W),
    ]
}

/// Day `k` (Monday 0) of the week strip: (x, width) and the y range below the arrows.
pub fn week_day_span(k: usize, width: f64) -> (f64, f64) {
    let w = (width - 2.0 * INBOX_TILE_X) / 7.0;
    (INBOX_TILE_X + k as f64 * w, w)
}

/// Where a click at `x`, `y` (points from the top left of the list) leads: back to the overview from the head, Waiting
/// from the amber strip, an area from its tile, another week or day in the week strip. `width`: the list's width.
pub fn nav_at(lines: &[ListLine], width: f64, x: f64, y: f64) -> Option<InboxNav> {
    let inside = |x: f64, (left, w): (f64, f64)| x >= left && x < left + w;
    let mut top = 0.0;
    for line in lines {
        let h = line.height();
        if y >= top && y < top + h {
            let dy = y - top;
            return match line {
                ListLine::InboxHead { tab, .. }
                    if *tab != InboxTab::Overview && inside(x, (INBOX_BACK_X, INBOX_BACK_W)) =>
                {
                    Some(InboxNav::Open(InboxTab::Overview))
                }
                ListLine::WaitBar { .. } => Some(InboxNav::Open(InboxTab::Waiting)),
                ListLine::Tiles(tiles) if dy < INBOX_TILE_H => tile_spans(tiles.len(), width)
                    .into_iter()
                    .zip(tiles)
                    .find(|(span, _)| inside(x, *span))
                    .map(|(_, t)| match &t.team {
                        Some(name) => InboxNav::Team(name.clone()),
                        None => InboxNav::Open(t.tab),
                    }),
                ListLine::Week(week) if dy < INBOX_WEEK_DAYS_TOP => {
                    let [back, ahead] = week_arrows(width);
                    if inside(x, back) {
                        Some(InboxNav::Week(-1))
                    } else if inside(x, ahead) {
                        Some(InboxNav::Week(1))
                    } else {
                        None
                    }
                }
                ListLine::Week(week) => (0..week.days.len())
                    .find(|&k| inside(x, week_day_span(k, width)))
                    .map(|k| InboxNav::Day(week.days[k].date)),
                _ => None,
            };
        }
        top += h;
    }
    None
}

/// How far two fingers must move sideways (points, more than twice as far as up or down) to lead back to the overview.
pub const SWIPE_BACK: f64 = 40.0;

/// One scroll event in the inbox (Kevin 2026-11-10: "mit zwei Fingern zurückwischen"), either direction. `so_far`: the
/// gesture's sideways and up-down distance and whether it already led back; `began`: this event starts a gesture;
/// `trackpad`: fingers on the trackpad or the glide after lifting them (a mouse wheel has neither); `gesture`: fingers on
/// the trackpad in an area. Returns the new state, whether the event is the swipe's (so the list does not scroll), and
/// whether to lead back now. Once it led back, the rest of the gesture and its glide stay the swipe's: the overview it
/// opened does not scroll with it (review quick 2026-11-10).
pub fn swipe_step(
    so_far: (f64, f64, bool),
    began: bool,
    trackpad: bool,
    gesture: bool,
    dx: f64,
    dy: f64,
) -> ((f64, f64, bool), bool, bool) {
    let so_far = if began { (0.0, 0.0, false) } else { so_far };
    if so_far.2 && trackpad {
        return (so_far, true, false);
    }
    if !gesture {
        return (so_far, false, false);
    }
    let (sx, sy) = (so_far.0 + dx, so_far.1 + dy);
    let back = sx.abs() > SWIPE_BACK && sx.abs() > 2.0 * sy.abs();
    ((sx, sy, back), back, back)
}

/// The day the Planned area stands on after `nav`, from `day`. `None` = today, so it moves on at midnight with the clock
/// (review quick 2026-11-10: it stayed on the day before, which showed nothing).
pub fn plan_day_after(
    nav: InboxNav,
    day: Option<NaiveDate>,
    today: NaiveDate,
) -> Option<NaiveDate> {
    let day = match nav {
        InboxNav::Open(InboxTab::Planned) => return None,
        InboxNav::Open(_) | InboxNav::Team(_) => return day,
        InboxNav::Day(d) => d,
        InboxNav::Week(k) => planned::shift_week(day.unwrap_or(today), k, today),
    };
    (day != today).then_some(day)
}

/// Is `x`, `y` on the inbox's +?
pub fn plus_at(lines: &[ListLine], x: f64, y: f64) -> bool {
    lines.first().is_some_and(|first| {
        matches!(first, ListLine::InboxHead { .. })
            && y < first.height()
            && (INBOX_PLUS_X..INBOX_PLUS_X + INBOX_PLUS_W).contains(&x)
    })
}

/// The button at `x`, `y` with its session ids; `None` off the buttons.
pub fn button_at(lines: &[ListLine], x: f64, y: f64) -> Option<(CardButton, &[String])> {
    let mut top = 0.0;
    for line in lines {
        if y >= top
            && y < top + line.height()
            && let ListLine::Buttons { ids, buttons } = line
        {
            let across = x - INBOX_TEXT_X;
            let step = INBOX_BUTTON_W + INBOX_BUTTON_GAP;
            let slot = (across / step).floor();
            if slot < 0.0 || across - slot * step > INBOX_BUTTON_W {
                return None;
            }
            return buttons.get(slot as usize).map(|b| (*b, ids.as_slice()));
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
            ListEntry::Working(_) | ListEntry::Lot(_) => None,
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

// ---------- Inbox (Kevin 2026-11-09) ----------

/// What the inbox looks at besides its plan.
#[derive(Clone, Copy, Debug)]
pub struct InboxInput<'a> {
    pub planned: &'a [u64],
    pub windows: &'a [TermWindow],
    pub since: &'a BTreeMap<String, SinceEntry>,
    pub lookout: &'a BTreeMap<String, LookoutLine>,
    pub now: Time,
    pub read: &'a ReadAnswers,
    /// Index in `windows` of the terminal in front: its card is open.
    pub main: Option<usize>,
    pub tab: InboxTab,
    /// Session ids Kevin parked (`is_snoozed`).
    pub snoozed: &'a BTreeSet<String>,
    /// Session ids Kevin pinned, in the order he pinned them (`pin_rank`).
    pub pinned: &'a [String],
    /// What is planned (`plannedFile`); `None` = no planner, no Planned area.
    pub plan: Option<&'a [PlannedItem]>,
    /// The day the Planned area shows, and today.
    pub plan_day: NaiveDate,
    pub today: NaiveDate,
    /// The teams that exist (`team::update_known`), the projects with an own card (Kevin 2026-11-11), `leadNames`, and the
    /// team or card whose area is open (`InboxTab::Team`).
    pub teams: &'a BTreeSet<String>,
    pub cards: &'a [String],
    pub lead_names: &'a [String],
    pub team: Option<&'a str>,
}

/// Room a scroller that takes space leaves out of the cards ("Show scroll bars: Always", measured 17 pt on macOS 27).
const INBOX_SCROLLER_ROOM: f64 = 17.0;

/// Lines of the question on the open card, counted as if the scroller took its room: an overlay scroller does not,
/// then the card may get a line more than it needs, never one less (review quick 2026-11-10).
/// ponytail: an average character width, no font metrics in the core; measure in the macOS layer if text gets clipped.
fn open_lines(what: &str, panel_w: f64) -> usize {
    let chars: usize = graphemes(what).map(width_of).sum();
    let wide = (panel_w - INBOX_TEXT_X - INBOX_MARGIN - INBOX_SCROLLER_ROOM).max(1.0);
    ((chars as f64 * INBOX_OPEN_CHAR_W / wide).ceil() as usize).clamp(1, INBOX_OPEN_LINES_MAX)
}

/// A card and, on the open one, its note or the way back. Everything else is on the right-click (Kevin 2026-11-09: no
/// buttons where they are not needed).
fn inbox_card(i: usize, panel_w: f64, o: &InboxInput) -> Vec<ListLine> {
    let w = &o.windows[i];
    let row = menu_row(w, o.since, o.lookout, o.now);
    let c = window_label(w, o.since, o.lookout, o.now);
    // Line ends collapse: the card wraps the text itself, and counts its height from one running text.
    let what = c.detail.map(|d| {
        d.trim_matches(['„', '“', '”'])
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    });
    let open = Some(i) == o.main;
    let ids: Vec<String> = w.sessions.iter().map(|s| s.id.clone()).collect();
    // A pinned card stays until unpinned: parking or hiding it is not offered, and an old mark does not count.
    let pinned = pin_rank(w, o.pinned).is_some();
    let parked = !pinned
        && (is_snoozed(w, o.snoozed, o.read) || (c.tone == LabelTone::Ready && is_read(w, o.read)));
    let mut menu = if pinned {
        vec![CardButton::Unpin]
    } else if parked {
        vec![CardButton::Unpark]
    } else if c.tone == LabelTone::Ready {
        vec![CardButton::Pin, CardButton::Park, CardButton::ReadNext]
    } else {
        vec![CardButton::Pin, CardButton::Park]
    };
    menu.push(CardButton::Rename);
    let mut out = vec![ListLine::Card(InboxCard {
        ids: ids.clone(),
        name: row.name,
        project: row.project,
        since: c.duration,
        lines: what.as_deref().map_or(1, |t| open_lines(t, panel_w)),
        what,
        tone: c.tone,
        open,
        menu,
        pinned,
        mark: None,
    })];
    if open && parked {
        out.push(ListLine::Buttons {
            ids,
            buttons: vec![CardButton::Unpark],
        });
    } else if open && c.tone == LabelTone::Waiting {
        out.push(ListLine::Hint(
            tr(
                "Rechts im Terminal antworten",
                "Answer in the terminal on the right",
            )
            .to_owned(),
        ));
    }
    out
}

/// The inbox window, one tab at a time: what Kevin pinned, questions, then done; or what works, then the parking lot; or
/// what Kevin parked and what he marked read. Every tab counts, whichever is shown. Group by the state right now, not the
/// plan's: between two plans a session can have got an answer. Every card is listed, however many: below the tabs the
/// window scrolls (Kevin 2026-11-10: "scrollbar", no "+N more (in the menu)").
///
/// Pinned (Kevin 2026-11-09): on top of Waiting in the order he pinned them, whatever the session does; never parked or
/// read while pinned (`set_pinned`), and one that works is listed under Running too. "Wartet" counts only those that
/// wait, a pinned one among them: the amber tab says that somebody waits.
pub fn inbox_target(list: &SideList, o: &InboxInput, window_number: i64) -> ListTarget {
    // Every planned window with whether it is on the lot, sorted into the groups by its state now.
    let planned: Vec<(usize, bool)> = list
        .entries
        .iter()
        .filter_map(|e| {
            let (i, on_lot) = match e {
                ListEntry::Row(i) | ListEntry::Working(i) => (*i, false),
                ListEntry::Lot(i) => (*i, true),
                _ => return None,
            };
            Some((current_index(o.planned, i, o.windows)?, on_lot))
        })
        .collect();
    let is_pin = |i: usize| pin_rank(&o.windows[i], o.pinned).is_some();
    // A pinned card is neither parked nor read here, also not through a tab that joined it with an old mark.
    let parked = |i: usize| !is_pin(i) && is_snoozed(&o.windows[i], o.snoozed, o.read);
    // "Read" is about a done card; a session that works again stays on its tab.
    let read_done = |i: usize| {
        !is_pin(i)
            && window_tone(&o.windows[i]) == LabelTone::Ready
            && is_read(&o.windows[i], o.read)
    };
    let group = |tones: &[LabelTone], lot: Option<bool>| -> Vec<usize> {
        planned
            .iter()
            .filter(|(i, on_lot)| {
                tones.contains(&window_tone(&o.windows[*i]))
                    && lot.is_none_or(|l| l == *on_lot)
                    && !parked(*i)
                    && !read_done(*i)
            })
            .map(|(i, _)| *i)
            .collect()
    };
    let mut pins: Vec<usize> = planned
        .iter()
        .map(|(i, _)| *i)
        .filter(|&i| is_pin(i))
        .collect();
    pins.sort_by_key(|&i| pin_rank(&o.windows[i], o.pinned));
    let unpinned =
        |g: Vec<usize>| -> Vec<usize> { g.into_iter().filter(|&i| !is_pin(i)).collect() };
    // A team's windows stay in its own area (Kevin 2026-11-11: "Die sollen nur noch bei sich drinnen sein"): not under
    // Waiting, Running or Parked; its tile turns amber when one asks. Pinned ones stay pinned.
    let indices: Vec<usize> = planned.iter().map(|(i, _)| *i).collect();
    let teams = team::teams(o.windows, &indices, o.teams, o.cards, o.lead_names);
    let in_team: BTreeSet<usize> = teams.iter().flat_map(Team::members).collect();
    let not_team =
        |g: Vec<usize>| -> Vec<usize> { g.into_iter().filter(|i| !in_team.contains(i)).collect() };
    let snoozed: Vec<usize> = not_team(
        planned
            .iter()
            .map(|(i, _)| *i)
            .filter(|&i| parked(i))
            .collect(),
    );
    let read: Vec<usize> = not_team(
        planned
            .iter()
            .map(|(i, _)| *i)
            .filter(|&i| read_done(i) && !parked(i))
            .collect(),
    );
    let running = [LabelTone::Running, LabelTone::Watching];
    let (asks, stopped, done) = (
        not_team(unpinned(group(&[LabelTone::Waiting], None))),
        not_team(unpinned(group(&[LabelTone::Stopped], None))),
        not_team(unpinned(group(&[LabelTone::Ready], None))),
    );
    let (working, lot) = (
        not_team(group(&running, Some(false))),
        not_team(group(&running, Some(true))),
    );
    // Who waits, in the order Waiting lists them: pinned ones that do not work, questions, stopped, done.
    let waiting: Vec<usize> = pins
        .iter()
        .copied()
        .filter(|&i| !running.contains(&window_tone(&o.windows[i])))
        .chain(asks.iter().copied())
        .chain(stopped.iter().copied())
        .chain(done.iter().copied())
        .collect();
    let counts = InboxCounts {
        waiting: waiting.len(),
        working: working.len() + lot.len(),
        parked: snoozed.len() + read.len(),
        planned: o.plan.map(|p| planned::count_this_week(p, o.today)),
    };
    // Without a planner (any more) the Planned area is the overview, so is the area of a team that is gone.
    let open_team = o
        .team
        .and_then(|name| teams.iter().find(|t| t.name == name));
    let tab = match (o.tab, o.plan, open_team) {
        (InboxTab::Planned, None, _) | (InboxTab::Team, _, None) => InboxTab::Overview,
        (tab, _, _) => tab,
    };
    let team_head = open_team
        .filter(|_| tab == InboxTab::Team)
        .map(|t| (t.name.clone(), t.members().count()));
    let mut lines = vec![ListLine::InboxHead {
        tab,
        counts,
        team: team_head,
    }];
    if !matches!(tab, InboxTab::Overview | InboxTab::Waiting)
        && let Some(&first) = waiting.first()
    {
        lines.push(ListLine::WaitBar {
            n: waiting.len(),
            text: waiting_text(first, o),
        });
    }
    let target = |lines| ListTarget {
        rect: list.rect,
        lines,
        window_number,
    };
    let (groups, nobody): (Vec<(&str, &Vec<usize>)>, &str) = match tab {
        InboxTab::Overview => {
            let parts = (working.len(), lot.len(), snoozed.len(), read.len());
            lines.extend(overview(o, counts, &waiting, parts, &teams));
            return target(lines);
        }
        InboxTab::Team => {
            if let Some(t) = open_team {
                lines.extend(team_view::area(o, t, list.rect.width()));
            }
            return target(lines);
        }
        InboxTab::Planned => {
            lines.extend(planned_area(o, o.plan.unwrap_or_default()));
            return target(lines);
        }
        InboxTab::Waiting => (
            vec![
                (tr("Angeheftet", "Pinned"), &pins),
                (tr("Fragen an dich", "Questions for you"), &asks),
                // Broken off, waiting for a nudge (Kevin 2026-11-10, "1a").
                (tr("Angehalten", "Stopped"), &stopped),
                (tr("Fertig", "Done"), &done),
            ],
            tr("Niemand wartet auf dich.", "Nobody is waiting for you."),
        ),
        InboxTab::Working => (
            vec![
                (tr("Arbeitet", "Working"), &working),
                (tr("Auf dem Parkplatz", "In the parking lot"), &lot),
            ],
            tr("Gerade arbeitet niemand.", "Nobody is working right now."),
        ),
        InboxTab::Parked => (
            vec![
                (tr("Von dir geparkt", "Parked by you"), &snoozed),
                (tr("Gelesen", "Read"), &read),
            ],
            tr("Nichts geparkt.", "Nothing parked."),
        ),
    };
    if groups.iter().all(|(_, g)| g.is_empty()) {
        lines.push(ListLine::Empty(nobody.to_owned()));
    }
    for (label, group) in groups.into_iter().filter(|(_, g)| !g.is_empty()) {
        lines.push(ListLine::Group(label.to_owned()));
        for &i in group {
            let mut card = inbox_card(i, list.rect.width(), o);
            // A window of a project gets "own card" (Kevin 2026-11-11, "3a"); team and card windows are in their area.
            if !in_team.contains(&i)
                && team::project(&o.windows[i]).is_some()
                && let Some(ListLine::Card(c)) = card.first_mut()
            {
                c.menu.push(CardButton::MakeCard);
            }
            lines.extend(card);
        }
    }
    target(lines)
}

/// The amber strip's text: the first who waits, with the question if there is one.
fn waiting_text(i: usize, o: &InboxInput) -> String {
    let w = &o.windows[i];
    let name = menu_row(w, o.since, o.lookout, o.now).name;
    match window_label(w, o.since, o.lookout, o.now).detail {
        Some(what) => format!("{name}: {}", what.trim_matches(['„', '“', '”'])),
        None => name,
    }
}

/// "1 arbeitet" / "3 arbeiten".
fn count_word(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The overview (Kevin 2026-11-10, "H"): Waiting across the top, then the other areas two by two, each tile with its
/// count and what is in it. `parts`: working, on the lot, parked by Kevin, read.
fn overview(
    o: &InboxInput,
    counts: InboxCounts,
    waiting: &[usize],
    (working, lot, snoozed, read): (usize, usize, usize, usize),
    teams: &[Team],
) -> Vec<ListLine> {
    let waits = counts.waiting > 0;
    let wait = InboxTile {
        tab: InboxTab::Waiting,
        team: None,
        title: if waits {
            tr("Wartet auf dich", "Waiting for you")
        } else {
            tr("Wartet", "Waiting")
        }
        .to_owned(),
        count: counts.waiting,
        lines: if waits {
            waiting
                .iter()
                .take(2)
                .map(|&i| {
                    let w = &o.windows[i];
                    format!(
                        "{} {}",
                        window_tone(w).symbol(),
                        menu_row(w, o.since, o.lookout, o.now).name
                    )
                })
                .collect()
        } else {
            vec![tr("Niemand wartet auf dich.", "Nobody is waiting for you.").to_owned()]
        },
        hot: waits,
    };
    let run = InboxTile {
        tab: InboxTab::Working,
        team: None,
        title: tr("Läuft", "Running").to_owned(),
        count: counts.working,
        // Two short lines: a half tile is about 200 pt wide (live 2026-11-10: one line was cut off).
        lines: if counts.working == 0 {
            vec![tr("Gerade arbeitet niemand.", "Nobody is working right now.").to_owned()]
        } else {
            vec![
                count_word(
                    working,
                    tr("arbeitet", "working"),
                    tr("arbeiten", "working"),
                ),
                format!("{lot} {}", tr("auf dem Parkplatz", "in the parking lot")),
            ]
        },
        hot: false,
    };
    let park = InboxTile {
        tab: InboxTab::Parked,
        team: None,
        title: tr("Geparkt", "Parked").to_owned(),
        count: counts.parked,
        lines: vec![if counts.parked == 0 {
            tr("Nichts geparkt.", "Nothing parked.").to_owned()
        } else {
            format!(
                "{snoozed} {}, {read} {}",
                tr("von dir geparkt", "parked by you"),
                tr("gelesen", "read")
            )
        }],
        hot: false,
    };
    // The teams right below Waiting, two side by side (Kevin 2026-11-10, mockup "Teams").
    let mut lines = vec![ListLine::Tiles(vec![wait])];
    let team_tiles: Vec<InboxTile> = teams.iter().map(|t| team_view::tile(o, t)).collect();
    lines.extend(
        team_tiles
            .chunks(2)
            .map(|row| ListLine::Tiles(row.to_vec())),
    );
    match o.plan {
        Some(plan) => lines.extend([
            ListLine::Tiles(vec![run, plan_tile(plan, counts, o)]),
            ListLine::Tiles(vec![park]),
        ]),
        None => lines.push(ListLine::Tiles(vec![run, park])),
    }
    lines
}

/// "heute" / "morgen" / "Do 12.11." before a time.
fn when(day: NaiveDate, today: NaiveDate) -> String {
    if day == today {
        tr("heute", "today").to_owned()
    } else if today.succ_opt() == Some(day) {
        tr("morgen", "tomorrow").to_owned()
    } else {
        planned::short_date(day)
    }
}

/// The Planned tile: a start that did not happen first (then the tile is amber, Kevin has to act), else the next start;
/// below, how many items stand today and tomorrow.
fn plan_tile(plan: &[PlannedItem], counts: InboxCounts, o: &InboxInput) -> InboxTile {
    let shown = |it: &PlannedItem| planned::shown_day(it, o.today);
    let failed = plan.iter().find(|it| it.problem.is_some());
    let next = plan
        .iter()
        .filter(|it| it.problem.is_none() && it.time.is_some())
        .min_by_key(|it| (shown(it), it.time));
    let mut lines = Vec::new();
    if let Some(it) = failed {
        lines.push(format!(
            "! {} {}",
            tr("Nicht gestartet:", "Did not start:"),
            it.title
        ));
    } else if let Some(it) = next {
        let at = it.time.map(planned::clock).unwrap_or_default();
        lines.push(format!("▶ {} {at} {}", when(shown(it), o.today), it.title));
    }
    let tomorrow = o.today.succ_opt().unwrap_or(o.today);
    lines.push(format!(
        "{} {} · {} {}",
        planned::items_on(plan, o.today, o.today).len(),
        tr("heute", "today"),
        planned::items_on(plan, tomorrow, o.today).len(),
        tr("morgen", "tomorrow")
    ));
    InboxTile {
        tab: InboxTab::Planned,
        team: None,
        title: tr("Geplant", "Planned").to_owned(),
        count: counts.planned.unwrap_or(0),
        lines,
        hot: failed.is_some(),
    }
}

/// The Planned area (Kevin 2026-11-10, variant 1): the week of the chosen day with dots, below it that day's items.
fn planned_area(o: &InboxInput, plan: &[PlannedItem]) -> Vec<ListLine> {
    let monday = planned::monday_of(o.plan_day);
    let days = monday
        .iter_days()
        .take(7)
        .map(|date| {
            // A dot per card the day lists (review quick 2026-11-10), what wants Kevin first so the cut keeps it.
            let mut dots: Vec<PlanMark> = planned::items_on(plan, date, o.today)
                .into_iter()
                .map(plan_mark)
                .chain(
                    planned::deadlines_on(plan, date, o.today)
                        .into_iter()
                        .map(|_| PlanMark::Deadline),
                )
                .collect();
            dots.sort_by_key(|m| match m {
                PlanMark::Problem | PlanMark::Deadline => 0,
                PlanMark::Start => 1,
                PlanMark::Task => 2,
            });
            dots.truncate(4);
            WeekDay {
                date,
                name: planned::weekday_short(date).to_owned(),
                number: date.day(),
                today: date == o.today,
                selected: date == o.plan_day,
                dots,
            }
        })
        .collect();
    let mut lines = vec![
        ListLine::Week(WeekStrip {
            title: planned::week_title(monday),
            days,
        }),
        ListLine::Group(planned::day_title(o.plan_day, o.today)),
    ];
    let on = planned::items_on(plan, o.plan_day, o.today);
    let due = planned::deadlines_on(plan, o.plan_day, o.today);
    if on.is_empty() && due.is_empty() {
        lines.push(ListLine::Empty(
            tr(
                "An diesem Tag ist nichts geplant.",
                "Nothing planned that day.",
            )
            .to_owned(),
        ));
    }
    lines.extend(on.into_iter().map(|it| plan_card(it, None, o.today)));
    lines.extend(due.into_iter().map(|it| {
        let text = format!(
            "{} {}, {} {}",
            tr("Harte Frist", "Hard deadline"),
            planned::short_date(o.plan_day),
            tr("steht am", "listed on"),
            planned::short_date(planned::shown_day(it, o.today))
        );
        plan_card(it, Some((text, PlanMark::Deadline)), o.today)
    }));
    lines
}

/// What a planned item is on its own card and its dot.
fn plan_mark(it: &PlannedItem) -> PlanMark {
    if it.problem.is_some() {
        PlanMark::Problem
    } else if it.time.is_some() {
        PlanMark::Start
    } else {
        PlanMark::Task
    }
}

/// A planned item as a card: its mark instead of a status, its time at the right, no ids and no right-click.
fn plan_card(it: &PlannedItem, deadline: Option<(String, PlanMark)>, today: NaiveDate) -> ListLine {
    let (what, mark) = deadline.unwrap_or_else(|| (planned::item_text(it, today), plan_mark(it)));
    ListLine::Card(InboxCard {
        ids: vec![],
        name: it.title.clone(),
        project: (!it.project.is_empty()).then(|| it.project.clone()),
        since: it.time.map(planned::clock),
        what: Some(what),
        tone: match mark {
            PlanMark::Start => LabelTone::Ready,
            PlanMark::Task => LabelTone::Watching,
            PlanMark::Problem | PlanMark::Deadline => LabelTone::Waiting,
        },
        open: false,
        lines: 1,
        menu: vec![],
        pinned: false,
        mark: Some(mark),
    })
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

mod team_view;

#[cfg(test)]
mod tests;
