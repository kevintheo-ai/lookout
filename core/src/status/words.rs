//! What an answer text says about its session, without a model: the session's own Lookout line ("◌ / ? / ✓
//! Lookout: …", Kevin 2026-10-12) and the frozen word lists for "needs nothing from Kevin" (Kevin 2026-09-27 to
//! 2026-10-10). The patterns are copied verbatim from Watcher.swift; the measurements behind them are in the comments
//! there and in docs/TECH.md (R11-R13).

use std::sync::LazyLock;

use super::regex::{Regex, escape};
use crate::model::{
    Digest, Engine, LineStatus, LookoutLine, Session, Time, drop_first_character, fold_case,
    is_whitespace, trim_ws,
};
use crate::overlay::greeting_name;

// ---------- Text helpers with Foundation's semantics ----------

/// Foundation's `CharacterSet.whitespacesAndNewlines`: the `whitespaces` plus line feed to carriage return, U+0085,
/// U+2028 and U+2029 (measured 2026-11-06).
fn is_ws_or_newline(c: char) -> bool {
    is_whitespace(c) || is_newline(c)
}

pub(crate) fn trim_ws_nl(s: &str) -> &str {
    s.trim_matches(is_ws_or_newline)
}

/// Swift `Character.isNewline`.
fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\u{B}' | '\u{C}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

fn regex(pattern: &str, caseless: bool, lines: bool) -> Regex {
    Regex::new(pattern, caseless, lines).expect("the word-list patterns are valid")
}

fn caseless(pattern: &str) -> Regex {
    regex(pattern, true, false)
}

fn has(r: &Regex, text: &str) -> bool {
    r.is_match(text)
}

/// Replaces the matches with a space; `keep` keeps single matches.
fn blank(r: &Regex, text: &str, keep: impl Fn(&str) -> bool) -> String {
    let t: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut last = 0;
    for m in r.find_all(&t) {
        let hit: String = t[m.clone()].iter().collect();
        if keep(&hit) {
            continue;
        }
        out.extend(&t[last..m.start]);
        out.push(' ');
        last = m.end;
    }
    out.extend(&t[last..]);
    out
}

/// Like `blank`, but sentence by sentence: sentences with an `unless` match stay as they are.
fn blank_by_sentence(r: &Regex, text: &str, unless: &Regex, span: &Regex) -> String {
    let t: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut last = 0;
    for m in span.find_all(&t) {
        let sentence: String = t[m.clone()].iter().collect();
        if has(unless, &sentence) {
            continue;
        }
        out.extend(&t[last..m.start]);
        out.push_str(&blank(r, &sentence, |_| false));
        last = m.end;
    }
    out.extend(&t[last..]);
    out
}

/// The text after the last match of `r`, or all of it.
fn after_last<'a>(r: &Regex, text: &'a str) -> &'a str {
    let t: Vec<char> = text.chars().collect();
    let end = r.find_all(&t).last().map_or(0, |m| m.end);
    &text[char_offset(text, end)..]
}

/// The text before the first match of `r`, or all of it.
fn before_first<'a>(r: &Regex, text: &'a str) -> &'a str {
    let t: Vec<char> = text.chars().collect();
    let start = r.find_at(&t, 0).map_or(t.len(), |m| m.start);
    &text[..char_offset(text, start)]
}

/// Byte offset of the character at index `n`.
fn char_offset(text: &str, n: usize) -> usize {
    text.char_indices().nth(n).map_or(text.len(), |(i, _)| i)
}

// ---------- Lookout line: the session says itself whether it needs Kevin (Kevin 2026-10-12) ----------

static HERE: LazyLock<Regex> =
    LazyLock::new(|| caseless(r"\b(here|hier)\b|this window|diese[mn]? Fenster"));
/// Status word at the start without brackets ("Bitte Freigabe für den Merge …", seen in the sample of 2026-10-12).
static LINE_WORD: LazyLock<Regex> = LazyLock::new(|| {
    caseless(r"^(braucht nichts|bitte|fertig|nothing needed|request|done)\b[\s:,.–-]*")
});

/// The last non-empty line, if it is a Lookout line. Markdown around it (`**`, backticks) does not count; the mark
/// decides, not the language. Neither does a text variation selector after the check mark (✓︎).
pub fn lookout_line(text: &str) -> Option<LookoutLine> {
    let last = text
        .split(is_newline)
        .filter(|l| !l.is_empty())
        .rfind(|l| !l.chars().all(char::is_whitespace))?;
    let line = last.trim_matches(|c| is_whitespace(c) || matches!(c, '`' | '*' | '_'));
    let status = match line.chars().next()? {
        '\u{25CC}' => LineStatus::Nothing,
        '?' => LineStatus::Ask,
        '\u{2713}' | '\u{2714}' => LineStatus::Done,
        _ => return None,
    };
    let rest = trim_ws(drop_first_character(line));
    let said = trim_ws(rest.strip_prefix("Lookout:")?);
    // Nothing after it (cut off?): no line, Lookout decides as before (review quick, Muse).
    if said.is_empty() {
        return None;
    }
    // First the status word, then a bracket that spans exactly the whole rest ("(a) oder (b)" stays as it is — review
    // quick, GLM). If nothing is left, the status word stands.
    let chars: Vec<char> = said.chars().collect();
    let mut tail = LINE_WORD
        .find_at(&chars, 0)
        .map_or(said, |m| &said[char_offset(said, m.end)..]);
    tail = tail.trim_matches(['.', '!', ' ']); // "(Merge #126)." (review quick, Grok)
    let tail = if tail.starts_with('(') && closes_at_end(tail) {
        trim_ws(&tail[1..tail.len() - 1])
    } else {
        tail
    };
    Some(LookoutLine {
        status,
        reason: if tail.is_empty() {
            said.trim_matches([' ', '(', ')']).to_owned()
        } else {
            tail.to_owned()
        },
    })
}

/// Does the bracket at the start close only with the last character?
fn closes_at_end(s: &str) -> bool {
    let n = s.chars().count();
    let mut depth = 0;
    for (i, c) in s.chars().enumerate() {
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
        }
        if depth == 0 {
            return i == n - 1;
        }
    }
    false
}

impl Digest {
    /// What the session says about itself at the end of its last answer (`lookout_line`). Not after Kevin's next
    /// message: the old line would stand on the label, say over an approval question. A request that only sends Kevin
    /// to another window (`other_window_names`, without "here") counts as "needs nothing": Ultivor sessions wrote "? …
    /// in the key account" despite the instruction (Kevin 2026-10-13, 28 of 32 such lines did not need him here). In
    /// the named window itself it stays a request.
    pub fn line(&self, other_window_names: &[String]) -> Option<LookoutLine> {
        if !self.text_current {
            return None;
        }
        let l = lookout_line(self.text.as_deref()?)?;
        // Foundation's `range(of:options: .caseInsensitive)`; an empty name is never found.
        let named = |s: &str| {
            other_window_names
                .iter()
                .any(|n| !n.is_empty() && fold_case(s).contains(&fold_case(n)))
        };
        if l.status != LineStatus::Ask
            || !named(&l.reason)
            || has(&HERE, &l.reason)
            || named(self.name.as_deref().unwrap_or(""))
        {
            return Some(l);
        }
        Some(LookoutLine {
            status: LineStatus::Nothing,
            reason: l.reason,
        })
    }
}

/// A Lookout line at the end of the turn that asks for something puts the window in front as "waiting for you", even
/// when only a background command still runs. `busy` (the model works) stays as reported, so does an open question.
pub fn apply_lookout_lines(sessions: Vec<Session>, other_window_names: &[String]) -> Vec<Session> {
    sessions
        .into_iter()
        .map(|mut s| {
            let asks = s.engine == Engine::Claude
                && !s.waiting
                && (s.ready || s.background_only || s.watching)
                && s.digest.as_ref().is_some_and(|d| {
                    d.turn_ended
                        && d.line(other_window_names)
                            .is_some_and(|l| l.status == LineStatus::Ask)
                });
            if asks {
                s.waiting = true;
                s.ready = true;
                s.watching = false;
            }
            s
        })
        .collect()
}

// ---------- Waits for another session or a wake-up: does the answer need Kevin? (Kevin 2026-09-27) ----------
//
// Rule without a model, frozen 2026-09-27 before the check on fresh cases, sharpened 2026-10-01, R11 (2026-10-05) and
// 2026-10-10. Cautious by design: when in doubt it stays "done". Languages other than German and English never match.

static NOTHING_NEEDED: LazyLock<Regex> = LazyLock::new(|| {
    caseless(concat!(
        r"nothing (is |more )?needed|no action (is )?needed|nothing for you|",
        r"nichts (zu tun )?für dich|für dich (gerade )?nichts|nichts von dir|noch nichts zu entscheiden"
    ))
});
/// "No question for you" since Kevin 2026-10-10 (2 of 3,016 turn ends since 2026-10-05), not to third parties
/// ("nothing to ask the lead", also across the line break). Weaker than "nothing needed": only on the whole text.
static NOTHING_TO_ASK: LazyLock<Regex> = LazyLock::new(|| {
    caseless(r"nothing (more |else )?to ask( you)?(?!\s+\w)|nichts (mehr )?zu fragen")
});
/// Done message, part 1: Kevin can close the window or it is handed over. Always blocks.
static CLOSE_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    caseless(concat!(
        r"close this window|can close|kannst (das|dieses) fenster|schlie(ß|ss)en|stood down|",
        r"nothing (is )?left|handed over|übergeben|",
        r"until you (close|end)|for you to (end|close)|stays open|leave (this|the) (session|window) open|closed out"
    ))
});
/// Done message, part 2: progress words. They do not block when the session says itself that it waits.
const PROGRESS_DONE_PATTERN: &str = concat!(
    r"erledigt|\bfinished\b|\bdone\b|abgeschlossen|",
    r"\b(is|are) (now )?(done|closed|merged|finished|complete|completed)\b"
);
static PROGRESS_DONE: LazyLock<Regex> = LazyLock::new(|| caseless(PROGRESS_DONE_PATTERN));
/// Only these hide while waiting: without a following clause ("The summary is finished — reply with any corrections"
/// can start an order; review quick 2026-10-10 round 2, GLM).
static PROGRESS_DONE_PLAIN: LazyLock<Regex> = LazyLock::new(|| {
    caseless(&format!(
        "({PROGRESS_DONE_PATTERN}){}",
        r"(?!\s*[,;—–]|\s+-\s)"
    ))
});
/// The session waits itself ("I hold until #258's run", "I'm waiting for the lead", "ich warte auf …"); "hold" only
/// with "until" in the sentence, not negated (review quick 2026-10-10, Codex + GLM).
static HOLDS: LazyLock<Regex> = LazyLock::new(|| {
    caseless(concat!(
        r"\bI(['’]m| am|['’]ll| will)? (now |still )?(wait|waiting|(hold|holding)(?=[^.!?]*\b(until|till)\b))\b",
        r"(?!\s+(\w+\s+)?((for|on)\s+)?(not|no longer|nothing)\b)|",
        r"\b(ich warte|warte ich)\b(?!\s+(\w+\s+)?(auf\s+)?(nicht|nichts)\b)"
    ))
});
/// Kevin in the sentence, also "dir/dich" and his first name (`greeting_name`: "I hold until Kevin reviews the PR"),
/// the name only capitalised ("Will" is not "I will wait").
const MENTIONS_KEVIN_PATTERN: &str = r"you|your|du|dein\w*|dich|dir";

fn mentions_kevin() -> Regex {
    let greeting = greeting_name();
    let name = if greeting.is_empty() {
        String::new()
    } else {
        format!("|(?-i:{})", escape(&greeting))
    };
    Regex::new(
        &format!(r"\b({MENTIONS_KEVIN_PATTERN}{name})\b"),
        true,
        false,
    )
    .unwrap_or_else(|_| caseless(&format!(r"\b({MENTIONS_KEVIN_PATTERN})\b")))
}

static ASKS_KEVIN: LazyLock<Regex> = LazyLock::new(|| {
    caseless(concat!(
        r"your (call|go|answer|decision|pick|merge|word)|for you to (merge|review|test|look|decide)|",
        r"ready for you|waits? (only )?for your|need(s)? (your|from you)|deine (entscheidung|antwort|freigabe|wahl)|",
        r"brauche ich|soll ich|willst du|möchtest du|beyond the|",
        r"your next|(waiting|wait|waits) (only )?(on|for) (you|your)\b|\bexcept\b|\bafter you\b|",
        r"\bonce you\b|\bclick(s|ed|ing)?\b|",
        r"\bbitte\b|\bau(ß|ss)er\b|sobald du|\bdu bist dran|\bdein (klick|wort)|",
        // "auf deine" only as a whole word: "auf deinen/deinem Mac" is a place (Kevin 2026-10-06).
        r"\bwarte\w*( nur| noch)* auf (dich|dein)|auf deine\b|",
        // "please" is a request (review quick 2026-10-05, Codex).
        r"\bplease\b"
    ))
});
/// R11 C (Kevin 2026-10-05): an earlier question to Kevin is still open; blocks even behind "nothing needed".
static OPEN_QUESTION: LazyLock<Regex> = LazyLock::new(|| {
    caseless(concat!(
        r"questions? (above |on [^.\n]{0,40})?(still )?stands?\b|(still )?open with you|",
        r"beyond what my last|weiter offen|darauf warte ich|\bfragen?\b[^.\n]{0,40}\b(noch|weiter) offen"
    ))
});
/// R11 A' (Kevin 2026-10-05): places where "done/finished/erledigt" is NOT a done message of this session: negated,
/// condition or future without Kevin, a partial step, and "Done:" in a list that also has "Still to do:".
macro_rules! done_word {
    () => {
        r"(done|finished|complete|completed|erledigt|abgeschlossen)"
    };
}
/// Condition and partial step end at sentence and clause borders (review quick 2026-10-05, Codex).
macro_rules! in_clause {
    () => {
        r"[^.!?\n,;:—–]"
    };
}
static NOT_DONE: LazyLock<Regex> = LazyLock::new(|| {
    caseless(concat!(
        r"(\bnot|n['’]t)\s+((yet|quite)\s+){0,2}",
        done_word!(),
        r"\b|\bnoch nicht\s+",
        done_word!(),
        r"\b"
    ))
});
static DONE_LATER: LazyLock<Regex> = LazyLock::new(|| {
    caseless(concat!(
        r"\b(when|once|after|until|till|before|as soon as)\b",
        in_clause!(),
        r"{0,60}?\b",
        r"(is|are|['’]s|has been|have been|gets?)\s+(now\s+)?",
        done_word!(),
        r"\b(?!\s*[,;:—–])|\bonce done\b(?!\s*[,;:—–])|",
        r"\b(sobald|wenn|bis|nachdem)\b",
        in_clause!(),
        r"{0,60}?\b",
        done_word!(),
        r"\s+(ist|sind|wird|werden)\b(?!\s*[,;:—–])"
    ))
});
static PART_DONE: LazyLock<Regex> = LazyLock::new(|| {
    caseless(concat!(
        r"\b(part|step|phase|stage|round|call|teil|schritt|runde)\s+",
        r"(\d+|one|two|three|[a-c]|i{1,3}|first|second|third)\b",
        in_clause!(),
        r"{0,40}?\b(is|are|ist|sind)\s+(now\s+|jetzt\s+)?",
        done_word!(),
        r"\b"
    ))
});
static DONE_HEADING: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"^[ \t]*[-*•]?[ \t]*\**[ \t]*(done|erledigt)[ \t]*(:\**|\**[ \t]*:)",
        true,
        true,
    )
});
/// Only clear remainder lists, not "Next:"/"Open:" (review quick 2026-10-05, Kimi); the heading needs content in the
/// same line, "Still to do: nothing" is no interim state (Codex).
static OPEN_HEADING: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        concat!(
            r"^[ \t]*[-*•]?[ \t]*\**[ \t]*(still to do|to do|todo|remaining|offen|noch offen|noch zu tun)",
            r"[ \t]*(:\**|\**[ \t]*:)[ \t*_]*(?!(nothing|none|nichts|keine[ns]?|n/?a)\b)[^\s—–-]"
        ),
        true,
        true,
    )
});
/// Names the same sentence a last step ("Step 2 is done — that was the last one"), "Part/Step N is done" stays a done
/// message (review quick 2026-10-05, GLM; per sentence, Kimi).
static FINAL_PART: LazyLock<Regex> = LazyLock::new(|| {
    caseless(concat!(
        r"\b(last|final|letzte[mnrs]?|finale[mnrs]?)\b|\ball (steps|parts|rounds|phases)\b|",
        r"\balle (schritte|teile|runden|phasen)\b"
    ))
});
/// A condition on Kevin ("when you're done testing") stays a done place.
static ADDRESSES_KEVIN: LazyLock<Regex> = LazyLock::new(|| caseless(r"\b(you|your|du|dein\w*)\b"));
static SENTENCE_SPAN: LazyLock<Regex> = LazyLock::new(|| regex(r"[^.!?\n]+", false, false));
/// A sentence across line breaks ("I hold until\nyou approve …"); a full stop without following white space
/// ("README.md", "v1.2") does not end it (review quick 2026-10-10, Codex).
static SENTENCE_ACROSS_LINES: LazyLock<Regex> =
    LazyLock::new(|| regex(r"(?:[^.!?]|[.!?](?=\S))+", false, false));

/// Text without the places where "done/finished/erledigt" is no done message of this session (only for the done
/// check; requests are checked on the whole text).
pub fn without_progress_notes(text: &str) -> String {
    let t = blank(&NOT_DONE, text, |_| false);
    let t = blank(&DONE_LATER, &t, |m| has(&ADDRESSES_KEVIN, m));
    let t = blank_by_sentence(&PART_DONE, &t, &FINAL_PART, &SENTENCE_SPAN);
    if has(&OPEN_HEADING, &t) {
        blank(&DONE_HEADING, &t, |_| false)
    } else {
        t
    }
}

/// Promise to report back by itself, first person only ("Sag Bescheid", "Melde mich … an" are requests to Kevin).
static PROMISES_REPORT: LazyLock<Regex> = LazyLock::new(|| {
    caseless(concat!(
        r"\b(ich melde|melde ich) mich\b|\b(ich (gebe|sage|sag)|(gebe|sage|sag) ich) (dir )?bescheid\b|",
        r"\bI['’]ll (report|check|get|come) back\b|\bI['’]ll (let you know|ping you|update you)\b|",
        r"\bI will (report back|let you know)\b"
    ))
});
/// A request or condition on Kevin anywhere in the answer: then no promise counts ("Please approve …", "wenn du …").
static PROMISE_BLOCKS: LazyLock<Regex> =
    LazyLock::new(|| caseless(r"\bplease\b|\b(wenn|falls|nachdem) du\b|\b(if|when) you\b"));
/// Sentence end for the promise, narrower than `SENTENCE_END`: "."/"!" plus white space when a letter follows in the
/// same line, also lowercase or behind an emoji or number ("… is green. merge PR #12"), or a line break.
static PROMISE_SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r#"[.!][*_"'`“”’‘»«›‹)\]]*\s+(?=[^\p{L}\n]*\p{L})|\n"#,
        false,
        false,
    )
});
/// Second clause after the promise: dash, colon, semicolon ("… durch ist — merge danach PR #12").
static CLAUSE_BREAK: LazyLock<Regex> = LazyLock::new(|| regex(r"\s[—–-]\s|[:;]\s", false, false));
static DIRECT_ASK: LazyLock<Regex> = LazyLock::new(|| {
    caseless(concat!(
        r"\bplease\b|\bbitte\b|\bI need (you|your)\b|\bich brauche (dich|dein\w*)\b|",
        r"ready for you|for you to (merge|review|test|look|decide|approve|check)"
    ))
});
/// Sentence end: "." or "!" (also with closing markdown, quotes, guillemets), then white space, then, after any
/// characters that are neither letter nor digit, an uppercase letter or a digit ("e.g. the" does not end), or a line
/// break (review quick 2026-10-01, Codex rounds 1, 3, 4).
static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r#"[.!][*_"'`“”’‘»«›‹)\]]*(?=\s+[^\p{L}\p{N}\s]*[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{N}])|\n"#,
        false,
        false,
    )
});

/// Does the answer want nothing from Kevin: no question, no request to him, no earlier question still open, no done
/// message? `holding`: the session waits itself; then progress words only count in sentences to Kevin or before a
/// further clause.
pub fn asks_nothing(text: &str, holding: bool) -> bool {
    !text.contains('?')
        && !reports_done(text, holding)
        && !has(&ASKS_KEVIN, text)
        && !has(&OPEN_QUESTION, text)
}

/// Done message in the text: closing/handover always, progress words with `holding` as in `asks_nothing`.
fn reports_done(text: &str, holding: bool) -> bool {
    let mut rest = without_progress_notes(text);
    // Closing/handover before blanking: "is closed out" would lose its "is closed" (review quick, Codex).
    if has(&CLOSE_WORDS, &rest) {
        return true;
    }
    if holding {
        rest = blank_by_sentence(
            &PROGRESS_DONE_PLAIN,
            &rest,
            &mentions_kevin(),
            &SENTENCE_ACROSS_LINES,
        );
    }
    has(&PROGRESS_DONE, &rest)
}

/// Does the session say in a sentence without Kevin that it waits itself, and in none that it waits for him?
pub fn holds_itself(text: &str) -> bool {
    let kevin = mentions_kevin();
    let t: Vec<char> = text.chars().collect();
    let holds: Vec<String> = SENTENCE_ACROSS_LINES
        .find_all(&t)
        .into_iter()
        .map(|m| t[m].iter().collect::<String>())
        .filter(|s| has(&HOLDS, s))
        .collect();
    !holds.is_empty() && !holds.iter().any(|s| has(&kevin, s))
}

/// Does the answer say explicitly that Kevin need not do anything, without question, request or done message? When
/// it says itself that it waits, progress words like "is merged" no longer block (Kevin 2026-10-10).
pub fn says_nothing_needed(text: &str) -> bool {
    (has(&NOTHING_NEEDED, text) || has(&NOTHING_TO_ASK, text))
        && asks_nothing(text, holds_itself(text))
}

/// Does the answer end with the promise to report back by itself ("Ich melde mich, sobald v3 durch ist", "I'll report
/// back")? With the watcher, Kevin's own message then no longer counts (Kevin 2026-10-03). When in doubt no promise:
/// only the last sentence counts and only as one clause, and a request or condition on Kevin anywhere blocks.
pub fn promises_report(text: &str) -> bool {
    if has(&PROMISE_BLOCKS, text) {
        return false;
    }
    let last = after_last(&PROMISE_SENTENCE_END, trim_ws_nl(text));
    has(&PROMISES_REPORT, last) && !has(&CLAUSE_BREAK, last)
}

/// Does the answer open explicitly with "nothing needed": first sentence without question, request or done message,
/// and no question mark anywhere? For the watcher the rest then does not count (Kevin 2026-10-01 evening). An earlier
/// question still open blocks in the whole text (R11 C, Kevin 2026-10-05).
pub fn opens_with_nothing_needed(text: &str) -> bool {
    if text.contains('?') || has(&OPEN_QUESTION, text) {
        return false;
    }
    let first = before_first(&SENTENCE_END, trim_ws_nl(text));
    has(&NOTHING_NEEDED, first) && says_nothing_needed(first)
}

/// Needs a session that only waits for another session or a wake-up nothing from Kevin (`Standby`)? The whole text
/// says "nothing needed", or the first sentence says it and the session says itself that it waits: then requests in
/// the rest do not count. Still blocking: a "?", an open earlier question, a done message as in R13 and a direct
/// request (Kevin 2026-10-10, screenshot 23:33).
pub fn waits_with_nothing_needed(text: &str) -> bool {
    says_nothing_needed(text)
        || (opens_with_nothing_needed(text)
            && holds_itself(text)
            && !reports_done(text, true)
            && !has(&DIRECT_ASK, text))
}

// ---------- Start prompt: a handover is not Kevin's turn (Kevin 2026-10-01 evening) ----------

/// Free-text options of the Claude CLI (`claude --help`, 2.1.287): their value is no start prompt.
const TEXT_OPTIONS: [&str; 4] = ["-n", "--name", "--system-prompt", "--append-system-prompt"];

/// Did the prompt come with the start (`claude "…"`, say a handover from another session)? Then it is exactly one of
/// the start arguments. A part of one does not count, nor an unquoted `claude mach das`; under 12 characters never.
pub fn launch_prompt(prompt: &str, arguments: &[String]) -> bool {
    let p = trim_ws_nl(prompt);
    let args = arguments.get(1..).unwrap_or(&[]);
    // A start prompt is a sentence: paths, ids and model names have no white space (review quick round 4, Codex).
    if !p.chars().any(char::is_whitespace) {
        return false;
    }
    p.chars().count() >= 12
        && args.iter().enumerate().any(|(i, a)| {
            trim_ws_nl(a) == p && (i == 0 || !TEXT_OPTIONS.contains(&args[i - 1].as_str()))
        })
}

/// Kevin's turn whose prompt came with the start is not his. The prompt must come within 30 s after the process start
/// (after `/clear` the same process begins a new conversation; measured: start prompt 3 s after start). Arguments and
/// start are read only when it matters; unreadable = Kevin's turn as before.
pub fn launch_corrected(
    d: Digest,
    arguments: impl FnOnce() -> Option<Vec<String>>,
    started: impl FnOnce() -> Option<Time>,
) -> Digest {
    let handover = d.user_turn == Some(true)
        && d.user_prompt.as_deref().is_some_and(|p| {
            d.user_prompt_at.is_some_and(|at| {
                started().is_some_and(|s| (-5.0..30.0).contains(&(at - s)))
                    && arguments().is_some_and(|a| launch_prompt(p, &a))
            })
        });
    Digest {
        user_turn: if handover { Some(false) } else { d.user_turn },
        ..d
    }
}
