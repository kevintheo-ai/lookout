//! Claude's transcript: what is the session doing right now? (design "Schilder" 2026-09-21)
//!
//! Claude writes one JSONL transcript per session: `ai-title` (the topic, rewritten as it goes), `assistant` (answer
//! text, tool steps, thinking blocks), `user` (Kevin's prompt or tool results). Only the tail is read; the start of a
//! long turn lies before it, then `prompt_at` stays empty.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::sync::LazyLock;
use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local, LocalResult, NaiveDate, Offset, TimeDelta, TimeZone};
use serde_json::{Map, Value};

use super::regex::Regex;
use super::words::trim_ws_nl;
use super::{Obj, as_bool, as_double, as_int, now, objects, split_lines};
use crate::i18n::tr;
use crate::mapping::last_path_component;
use crate::model::{Digest, LastBlock, Pid, Time};

/// Bytes read from the end of a transcript or engine log.
pub const TAIL_BYTES: u64 = 262_144;

/// `ISO8601DateFormatter` (internet date time, with and without fractional seconds) for the timestamps Claude writes,
/// "2026-09-21T12:40:00.000Z". Like Foundation (measured 2026-11-06): fractions cut to milliseconds, "z" and "+0200"
/// taken, spaces around trimmed. ponytail: strict field widths; Foundation also rolls over "2026-02-30" and takes
/// "2026-9-21".
pub(crate) fn parse_timestamp(s: &str) -> Option<Time> {
    let s = s.trim_matches(' ');
    let digits = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = s.get(r)?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let b = s.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    let (hour, minute, second) = (digits(11..13)?, digits(14..16)?, digits(17..19)?);
    let mut rest = &s[19..];
    let mut millis = 0;
    if let Some(f) = rest.strip_prefix('.') {
        let n = f.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return None;
        }
        millis = format!("{:0<3}", &f[..n.min(3)]).parse::<i64>().ok()?;
        rest = &f[n..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let hm = rest
                .get(1..)?
                .replacen(':', "", usize::from(rest.len() == 6));
            if hm.len() != 4 || !hm.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            sign * (hm[..2].parse::<i64>().ok()? * 3600 + hm[2..].parse::<i64>().ok()? * 60)
        }
    };
    let at = NaiveDate::from_ymd_opt(
        i32::try_from(year).ok()?,
        u32::try_from(month).ok()?,
        u32::try_from(day).ok()?,
    )?
    .and_hms_opt(
        u32::try_from(hour).ok()?,
        u32::try_from(minute).ok()?,
        u32::try_from(second).ok()?,
    )?;
    let secs = at.and_utc().timestamp() - offset;
    Some((secs * 1000 + millis) as f64 / 1000.0)
}

fn transcript_date(obj: &Obj) -> Option<Time> {
    parse_timestamp(obj.get("timestamp")?.as_str()?)
}

fn str_of<'a>(obj: &'a Obj, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str)
}

fn obj_of<'a>(obj: &'a Obj, key: &str) -> Option<&'a Obj> {
    obj.get(key).and_then(Value::as_object)
}

fn first_question(input: &Obj) -> Option<&str> {
    objects(input.get("questions")?)?
        .first()?
        .get("question")?
        .as_str()
}

/// Tool step in readable form. Most frequent tools (sample 2026-09-21, 25 transcripts): Bash, Read, Edit, Write,
/// AskUserQuestion, WebFetch, Agent, Skill.
pub fn step_label(tool: &str, input: &Obj) -> String {
    let s = |k: &str| str_of(input, k).unwrap_or("");
    let file = last_path_component(s("file_path"));
    match tool {
        "Bash" => {
            let d = s("description");
            if d.is_empty() {
                split_lines(s("command"))
                    .next()
                    .unwrap_or("Bash")
                    .to_owned()
            } else {
                d.to_owned()
            }
        }
        "Read" => format!("{}{file}", tr("liest ", "reads ")),
        "Edit" | "NotebookEdit" => format!("{}{file}", tr("ändert ", "edits ")),
        "Write" => format!("{}{file}", tr("schreibt ", "writes ")),
        "Agent" | "Task" => format!("Agent: {}", s("description")),
        "Skill" => format!("Skill {}", s("skill")),
        "WebSearch" => format!(
            "{}{}",
            tr("sucht im Web: ", "searches the web: "),
            s("query")
        ),
        "WebFetch" => format!(
            "{}{}",
            tr("liest Webseite ", "reads web page "),
            url_host(s("url"))
        ),
        "AskUserQuestion" => format!(
            "{}{}",
            tr("Frage: ", "Question: "),
            first_question(input).unwrap_or("")
        ),
        _ => {
            // mcp__<server>__<tool> -> "<server>: <tool>"
            let parts: Vec<&str> = tool.split("__").collect();
            if parts.len() == 3 && parts[0] == "mcp" {
                format!("{}: {}", parts[1], parts[2])
            } else {
                tool.to_owned()
            }
        }
    }
}

/// `URLComponents(string:)?.host`: the authority after "//" without user and port, percent-decoded; IPv6 keeps its
/// brackets (measured 2026-11-06). ponytail: no IDNA decoding (Foundation shows "xn--…" hosts as Unicode) and only a
/// rough validity check (white space or control characters = no URL).
fn url_host(url: &str) -> String {
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return String::new();
    }
    let scheme = |s: &str| {
        s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    };
    let after_scheme = match url.split_once(':') {
        Some((s, rest)) if scheme(s) => rest,
        _ => url,
    };
    let Some(rest) = after_scheme.strip_prefix("//") else {
        return String::new();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if host_port.starts_with('[') {
        host_port.find(']').map_or(host_port, |i| &host_port[..=i])
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    super::percent_decode(host).unwrap_or_default()
}

/// Kevin's prompt: not a tool result, no system insert (`isMeta`) and not the summary after a compaction
/// (`isCompactSummary`, Kevin 2026-10-10: two waiting windows stood DONE after /compact).
fn is_prompt(obj: &Obj) -> bool {
    if obj.get("isMeta").and_then(as_bool) == Some(true)
        || obj.get("isCompactSummary").and_then(as_bool) == Some(true)
    {
        return false;
    }
    let Some(msg) = obj_of(obj, "message") else {
        return false;
    };
    let content = msg.get("content");
    if content.is_some_and(Value::is_string) {
        return true;
    }
    let Some(blocks) = content.and_then(objects) else {
        return false;
    };
    let typed = |t: &str| blocks.iter().any(|b| str_of(b, "type") == Some(t));
    typed("text") && !typed("tool_result")
}

/// Text of a prompt: the string or the text blocks.
fn prompt_text(obj: &Obj) -> Option<String> {
    let content = obj_of(obj, "message").and_then(|m| m.get("content"));
    if let Some(s) = content.and_then(Value::as_str) {
        return Some(s.to_owned());
    }
    let texts: Vec<&str> = content
        .and_then(objects)
        .unwrap_or_default()
        .into_iter()
        .filter(|b| str_of(b, "type") == Some("text"))
        .filter_map(|b| str_of(b, "text"))
        .collect();
    (!texts.is_empty()).then(|| texts.join("\n"))
}

/// Tool calls whose result decides about wake sources (id -> name, input).
type Pending = BTreeMap<String, (String, Obj)>;

const WAKE_TOOLS: [&str; 4] = ["ScheduleWakeup", "CronCreate", "CronDelete", "SendMessage"];
/// Only Claude's own session sockets ("uds:/tmp/cc-socks/<pid>.sock"), not any (review quick, Codex).
static SOCKET_PID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/cc-socks/(\d+)\.sock$", false, false).expect("valid pattern"));
static NAME_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\[[^\]]*\]$", false, false).expect("valid pattern"));

/// Evaluates a transcript piece. Lines split ONLY at "\n" (Python's splitlines() also split at U+2028 inside JSON in
/// the spike). The first line of a tail is usually cut and simply fails to parse.
pub fn digest_transcript(text: &str) -> Digest {
    let mut d = Digest::default();
    let mut pending = Pending::new();
    for line in split_lines(text) {
        let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match str_of(&obj, "type") {
            Some("ai-title") => {
                if let Some(t) = str_of(&obj, "aiTitle").filter(|t| !t.is_empty()) {
                    d.title = Some(t.to_owned());
                }
            }
            Some("custom-title") => {
                if let Some(t) = str_of(&obj, "customTitle").filter(|t| !t.is_empty()) {
                    d.name = Some(t.to_owned());
                }
            }
            Some("user") => {
                // review quick 2026-09-21 (Kimi): with 1 of 211 questions the result of a tool started in parallel came
                // first; it must not close the question.
                let results = obj_of(&obj, "message")
                    .and_then(|m| m.get("content"))
                    .and_then(objects)
                    .unwrap_or_default();
                let answered = d
                    .question_id
                    .as_deref()
                    .is_some_and(|id| results.iter().any(|r| str_of(r, "tool_use_id") == Some(id)));
                let prompt = is_prompt(&obj);
                if answered || prompt {
                    d.question = None;
                    d.question_id = None;
                }
                if prompt {
                    d.prompt_at = transcript_date(&obj).or(d.prompt_at);
                    d.turn_ended = false;
                    // Only Kevin's own message makes the line stale: a turn after another session's message or a
                    // timer without new text keeps the last self-report (review quick, Grok).
                    let kind = obj_of(&obj, "origin").and_then(|o| str_of(o, "kind"));
                    if kind.is_none_or(|k| k == "human") {
                        d.text_current = false;
                    }
                }
                note_wake_sources(&obj, &results, &mut pending, &mut d);
            }
            Some("system") => {
                let subtype = str_of(&obj, "subtype");
                if subtype == Some("turn_duration") {
                    d.turn_ended = true;
                }
                // The wake prompt after it is a system insert (isMeta, without origin); the turn is not Kevin's
                // (review quick 2026-10-01, GLM).
                if subtype == Some("scheduled_task_fire") {
                    note_fire(&obj, &mut d);
                    d.user_turn = Some(false);
                }
            }
            Some("attachment") => {
                // What Kevin writes into a running turn comes as an attachment (CLI 2.1.283+): the answer is then his.
                let a = obj_of(&obj, "attachment");
                let human = a
                    .and_then(|a| obj_of(a, "origin"))
                    .and_then(|o| str_of(o, "kind"))
                    == Some("human");
                if a.and_then(|a| str_of(a, "commandMode")) == Some("prompt") && human {
                    d.user_turn = Some(true);
                    d.user_prompt = None; // mid-turn is always typed, never a start prompt
                    d.user_prompt_at = None;
                    d.text_current = false; // the line before does not answer his new message (review quick, Codex)
                }
            }
            Some("assistant") => {
                d.answer_at = transcript_date(&obj).or(d.answer_at);
                d.turn_ended = false;
                let blocks = obj_of(&obj, "message")
                    .and_then(|m| m.get("content"))
                    .and_then(objects)
                    .unwrap_or_default();
                for b in blocks {
                    match str_of(b, "type") {
                        Some("text") => {
                            let Some(t) = str_of(b, "text").filter(|t| !trim_ws_nl(t).is_empty())
                            else {
                                continue;
                            };
                            d.text = Some(t.to_owned());
                            d.text_current = true;
                            d.last = LastBlock::Text;
                        }
                        Some("tool_use") => {
                            let name = str_of(b, "name").unwrap_or("");
                            let input = obj_of(b, "input").cloned().unwrap_or_default();
                            d.step = Some(step_label(name, &input));
                            d.last = LastBlock::Tool;
                            if name == "AskUserQuestion" {
                                d.question = first_question(&input).map(str::to_owned);
                                d.question_id = str_of(b, "id").map(str::to_owned);
                            }
                            if WAKE_TOOLS.contains(&name) {
                                if let Some(id) = str_of(b, "id") {
                                    pending.insert(id.to_owned(), (name.to_owned(), input));
                                }
                            }
                        }
                        Some("thinking") => d.last = LastBlock::Thinking,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    d
}

/// Wake sources from a `user` entry: another session's message starts a conversation with it; results of timer and
/// SendMessage calls set or delete timers or name the other session. Kevin's prompt does not end the conversation (R11,
/// Kevin 2026-10-05); whether the other session still runs is checked by `Standby`.
fn note_wake_sources(obj: &Obj, results: &[&Obj], pending: &mut Pending, d: &mut Digest) {
    let origin = obj_of(obj, "origin");
    let kind = origin.and_then(|o| str_of(o, "kind"));
    if is_prompt(obj) {
        d.user_turn = Some(kind.is_none_or(|k| k == "human"));
        // A start prompt can only be the session's first prompt (`turnPosition.promptIndex` 1); the same text sent again
        // later is typed, without a position too when in doubt (review quick 2026-10-01, Codex).
        let first = obj_of(obj, "turnPosition")
            .and_then(|t| t.get("promptIndex"))
            .and_then(as_int)
            == Some(1);
        d.user_prompt = if d.user_turn == Some(true) && first {
            prompt_text(obj)
        } else {
            None
        };
        d.user_prompt_at = d.user_prompt.as_ref().and_then(|_| transcript_date(obj));
    }
    // Another session (socket "uds:…", checked by Claude), not the report of an own subagent (`handback`).
    if let Some(o) = origin {
        if kind == Some("peer")
            && str_of(o, "from").is_some_and(|f| f.starts_with("uds:"))
            && o.get("handback").and_then(as_bool) != Some(true)
        {
            d.peer_pid = o.get("verifiedPeerPid").and_then(as_int).and_then(pid);
            d.peer_name = str_of(o, "name").map(str::to_owned);
            d.peer_at = transcript_date(obj);
            d.user_turn = Some(false);
            if let Some(name) = &d.peer_name {
                d.partners.insert(name.clone());
            }
        }
    }
    let empty = Obj::new();
    let outcome = obj_of(obj, "toolUseResult").unwrap_or(&empty);
    for r in results
        .iter()
        .filter(|r| str_of(r, "type") == Some("tool_result"))
    {
        let Some((name, input)) = str_of(r, "tool_use_id").and_then(|id| pending.remove(id)) else {
            continue;
        };
        match name.as_str() {
            "ScheduleWakeup" => {
                if outcome.get("stopped").and_then(as_bool) == Some(true) {
                    d.wakeups.remove("loop");
                } else if let Some(ms) = outcome
                    .get("scheduledFor")
                    .and_then(as_double)
                    .filter(|ms| *ms > 0.0)
                {
                    d.wakeups.insert("loop".into(), Some(ms / 1000.0));
                }
            }
            "CronCreate" => {
                let Some(cid) = str_of(outcome, "id") else {
                    continue;
                };
                // When in doubt one-shot, and one-shot without a readable time does not count; otherwise it would stand
                // pending forever after firing (review quick 2026-09-27, Kimi). Recurring stays pending while the
                // session lives.
                let recurring = outcome
                    .get("recurring")
                    .and_then(as_bool)
                    .or_else(|| input.get("recurring").and_then(as_bool))
                    .unwrap_or(false);
                if recurring {
                    d.wakeups.insert(cid.to_owned(), None);
                } else {
                    let expr = str_of(outcome, "humanSchedule")
                        .or_else(|| str_of(&input, "cron"))
                        .unwrap_or("");
                    if let Some(at) =
                        one_shot_cron_date(expr, transcript_date(obj).unwrap_or_else(now))
                    {
                        d.wakeups.insert(cid.to_owned(), Some(at));
                    }
                }
            }
            "CronDelete" => {
                if let Some(cid) = str_of(outcome, "id").or_else(|| str_of(&input, "id")) {
                    d.wakeups.remove(cid);
                }
            }
            "SendMessage" => {
                let to = str_of(&input, "to");
                // To a socket address ("uds:/tmp/cc-socks/<pid>.sock") it is always another session; its answer then
                // names no name (Kevin 2026-10-10, screenshot 23:33). Only when delivered (`success` true, no
                // `is_error`); `Standby` checks the pid against the running sessions and their start.
                let delivered = outcome.get("success").and_then(as_bool) == Some(true)
                    && r.get("is_error").and_then(as_bool) != Some(true);
                if let Some(p) = to
                    .filter(|t| t.starts_with("uds:") && delivered)
                    .and_then(socket_pid)
                {
                    if Some(p) != d.peer_pid {
                        d.peer_name = None; // an answer to the same session keeps its name
                    }
                    d.peer_pid = Some(p);
                    d.peer_at = transcript_date(obj);
                    continue;
                }
                // Only to another running session ("another Claude session on this machine"), not to a subagent.
                let said = str_of(outcome, "message")
                    .or_else(|| str_of(r, "content"))
                    .unwrap_or("");
                let Some(to) = to.filter(|_| said.contains("another Claude session")) else {
                    continue;
                };
                let chars: Vec<char> = to.chars().collect();
                let name: String = match NAME_TAG.find_at(&chars, 0) {
                    Some(m) => chars[..m.start].iter().chain(&chars[m.end..]).collect(),
                    None => to.to_owned(),
                };
                if d.peer_name.as_deref() != Some(name.as_str()) {
                    d.peer_pid = None;
                }
                d.partners.insert(name.clone());
                d.peer_name = Some(name);
            }
            _ => {}
        }
    }
}

fn pid(v: i64) -> Option<Pid> {
    Pid::try_from(v).ok()
}

/// The pid in "uds:/tmp/cc-socks/<pid>.sock".
fn socket_pid(to: &str) -> Option<Pid> {
    let chars: Vec<char> = to.chars().collect();
    let m = SOCKET_PID.find_at(&chars, 0)?;
    let digits: String = chars[m.start + "/cc-socks/".len()..m.end - ".sock".len()]
        .iter()
        .collect();
    digits.parse().ok()
}

/// A timer fired: a one-shot cron is gone, a recurring one stays; the loop timer is gone when the firing fits its time
/// (an earlier run of another job leaves it standing).
fn note_fire(obj: &Obj, d: &mut Digest) {
    if let Some(tid) = str_of(obj, "taskId") {
        if matches!(d.wakeups.get(tid), Some(Some(_))) {
            d.wakeups.remove(tid);
        }
    }
    if let (Some(Some(loop_at)), Some(fired)) = (d.wakeups.get("loop"), transcript_date(obj)) {
        if fired >= loop_at - 60.0 {
            d.wakeups.remove("loop");
        }
    }
}

/// Time of a one-shot cron "M H D Mo *" in local time, the next after `after`. `None` = another pattern or no real
/// date (Calendar would silently turn 31 February into 3 March; review quick 2026-09-27, Kimi).
pub fn one_shot_cron_date(expr: &str, after: Time) -> Option<Time> {
    let f: Vec<&str> = expr.split(' ').filter(|s| !s.is_empty()).collect();
    if f.len() != 5 || f[4] != "*" {
        return None;
    }
    let n = |s: &str| s.parse::<i64>().ok();
    let (minute, hour, day, month) = (n(f[0])?, n(f[1])?, n(f[2])?, n(f[3])?);
    if !(0..=59).contains(&minute) || !(0..=23).contains(&hour) {
        return None;
    }
    let year = local(after)?.year();
    for y in [year, year + 1] {
        let Some(at) = local_time(y, month, day, hour, minute) else {
            continue;
        };
        if at > after - 60.0 {
            return Some(at);
        }
    }
    None
}

pub(crate) fn local(t: Time) -> Option<DateTime<Local>> {
    let secs = t.floor();
    let nanos = ((t - secs) * 1e9) as u32;
    Some(DateTime::from_timestamp(secs as i64, nanos)?.with_timezone(&Local))
}

/// Local wall-clock time to a moment, like `Calendar.date(from:)` (measured 2026-11-06, Europe/Zurich): an ambiguous
/// time (autumn) is the earlier one; a time in the spring gap moves forward by the gap (02:30 becomes 03:30).
fn local_time(year: i32, month: i64, day: i64, hour: i64, minute: i64) -> Option<Time> {
    let naive =
        NaiveDate::from_ymd_opt(year, u32::try_from(month).ok()?, u32::try_from(day).ok()?)?
            .and_hms_opt(u32::try_from(hour).ok()?, u32::try_from(minute).ok()?, 0)?;
    let at = match Local.from_local_datetime(&naive) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => t.timestamp(),
        LocalResult::None => {
            let before = Local
                .from_local_datetime(&(naive - TimeDelta::days(1)))
                .earliest()?;
            let offset = i64::from(before.offset().fix().local_minus_utc());
            naive.and_utc().timestamp() - offset
        }
    };
    Some(at as f64)
}

// ---------- Transcripts per session ----------

struct Cached {
    size: u64,
    modified: Option<SystemTime>,
    digest: Digest,
}

/// Finds and remembers transcripts per session. The project folder is not always the current cwd (worktree sessions):
/// search all project folders once, remember the path. Read only when size or modification time change.
pub struct TranscriptReader {
    config_dirs: Vec<String>,
    tail_bytes: u64,
    paths: BTreeMap<String, String>,
    cache: BTreeMap<String, Cached>,
}

impl TranscriptReader {
    /// `tail_bytes`: Swift default `TAIL_BYTES`.
    pub fn new(config_dirs: Vec<String>, tail_bytes: u64) -> TranscriptReader {
        TranscriptReader {
            config_dirs,
            tail_bytes,
            paths: BTreeMap::new(),
            cache: BTreeMap::new(),
        }
    }

    /// Digest for a Claude session id (without "claude:"). `None` = no transcript (yet).
    pub fn digest(&mut self, session_id: &str) -> Option<Digest> {
        let path = match self.paths.get(session_id) {
            Some(p) => p.clone(),
            None => self.locate(session_id)?,
        };
        // `attributesOfItem` does not follow a symbolic link: the link's own size and time, as in Swift.
        let Ok(meta) = fs::symlink_metadata(&path) else {
            self.paths.remove(session_id); // file gone: search again next time
            self.cache.remove(session_id);
            return None;
        };
        let (size, modified) = (meta.len(), meta.modified().ok());
        if let Some(c) = self.cache.get(session_id) {
            if c.size == size && c.modified == modified {
                return Some(c.digest.clone());
            }
        }
        let text = tail_string(&path, self.tail_bytes)?;
        let mut d = digest_transcript(&text);
        let seen = self.cache.get(session_id).map(|c| (c.size, &c.digest));
        if let (None, Some((seen_size, seen))) = (d.user_turn, seen) {
            d.user_turn = seen.user_turn;
            // Grew the file since the last read by more than the tail, Kevin's message mid-turn may lie in between:
            // then the remembered start prompt no longer holds (review quick 2026-10-01, Codex).
            let gap = size > seen_size.saturating_add(self.tail_bytes);
            d.user_prompt = if gap { None } else { seen.user_prompt.clone() };
            d.user_prompt_at = if gap { None } else { seen.user_prompt_at };
        }
        if let Some((_, seen)) = seen {
            d.partners.extend(seen.partners.iter().cloned()); // older ones fall out of the tail
            if d.peer_pid.is_none() && d.peer_name.is_none() {
                // After a long turn the other session's message falls out of the tail: keep the last one seen; whether
                // it still runs is checked by `Standby` (review quick 2026-10-10, GLM).
                d.peer_pid = seen.peer_pid;
                d.peer_name = seen.peer_name.clone();
                d.peer_at = seen.peer_at;
            }
        }
        if d.name.is_none() || d.title.is_none() {
            // After a long turn name and topic are missing from the tail: keep the last ones seen, search backwards once
            // on the first read.
            let (found_name, found_title) = match seen {
                None => last_titles(&path, LAST_TITLES_CHUNK, LAST_TITLES_MAX),
                Some(_) => (None, None),
            };
            d.name = d
                .name
                .or_else(|| seen.and_then(|(_, s)| s.name.clone()))
                .or(found_name);
            d.title = d
                .title
                .or_else(|| seen.and_then(|(_, s)| s.title.clone()))
                .or(found_title);
        }
        self.cache.insert(
            session_id.to_owned(),
            Cached {
                size,
                modified,
                digest: d.clone(),
            },
        );
        Some(d)
    }

    /// Keep only live sessions; otherwise the memory grows with every ended session.
    pub fn forget(&mut self, alive: &BTreeSet<String>) {
        self.paths.retain(|k, _| alive.contains(k));
        self.cache.retain(|k, _| alive.contains(k));
    }

    fn locate(&mut self, session_id: &str) -> Option<String> {
        for dir in &self.config_dirs {
            let projects = format!("{dir}/projects");
            for p in super::dir_names(&projects) {
                let candidate = format!("{projects}/{p}/{session_id}.jsonl");
                if fs::metadata(&candidate).is_ok() {
                    self.paths.insert(session_id.to_owned(), candidate.clone());
                    return Some(candidate);
                }
            }
        }
        None
    }
}

// ---------- File helpers ----------

/// The last `bytes` bytes of a file, invalid UTF-8 replaced. `None` = not readable or nothing to read (Foundation's
/// `readToEnd` gives nil for an empty file, measured).
pub fn tail_string(path: &str, bytes: u64) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let end = f.seek(SeekFrom::End(0)).ok()?;
    let _ = f.seek(SeekFrom::Start(end.saturating_sub(bytes)));
    let mut data = Vec::new();
    // At most `bytes`, even if the session appends while this reads (review quick 2026-11-06, Codex).
    f.take(bytes).read_to_end(&mut data).ok()?;
    (!data.is_empty()).then(|| String::from_utf8_lossy(&data).into_owned())
}

pub const LAST_TITLES_CHUNK: u64 = 1 << 20;
pub const LAST_TITLES_MAX: u64 = 32 << 20;
const NAME_MARK: &[u8] = br#""custom-title""#;
const TITLE_MARK: &[u8] = br#""ai-title""#;

fn contains_bytes(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Last own name and topic of a transcript, searched backwards in blocks. Claude writes both only with each message;
/// after a long turn they lie up to ~5.5 MB before the end (measured 2026-09-21 on 57 transcripts), far before the
/// tail. `max_bytes` limits the search.
pub fn last_titles(path: &str, chunk: u64, max_bytes: u64) -> (Option<String>, Option<String>) {
    let (mut name, mut title) = (None, None);
    let Ok(mut f) = File::open(path) else {
        return (name, title);
    };
    let Ok(size) = f.seek(SeekFrom::End(0)) else {
        return (name, title);
    };
    let floor = size.saturating_sub(max_bytes);
    let mut pos = size;
    let mut carry: Vec<u8> = vec![];
    while pos > floor && (name.is_none() || title.is_none()) {
        let start = floor.max(pos.saturating_sub(chunk));
        let mut block = vec![0; (pos - start) as usize];
        if f.seek(SeekFrom::Start(start)).is_err() || f.read_exact(&mut block).is_err() {
            break;
        }
        block.extend_from_slice(&carry);
        let mut data = block;
        pos = start;
        // The first line may begin in the earlier block: it moves on as rest (without a line end the whole block is the
        // rest of one long line).
        if start > floor {
            let Some(nl) = data.iter().position(|&b| b == b'\n') else {
                carry = data;
                continue;
            };
            carry = data[..nl].to_vec();
            data.drain(..=nl);
        }
        // Only take apart blocks with a line still sought: old sessions without an own name cost ~0.8 s on the first
        // read otherwise (163 MB transcript, measured).
        if !((name.is_none() && contains_bytes(&data, NAME_MARK))
            || (title.is_none() && contains_bytes(&data, TITLE_MARK)))
        {
            continue;
        }
        let text = String::from_utf8_lossy(&data);
        for line in split_lines(&text)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .filter(|l| l.contains("-title\""))
        {
            let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let kind = str_of(&obj, "type");
            if name.is_none() && kind == Some("custom-title") {
                name = str_of(&obj, "customTitle")
                    .filter(|t| !t.is_empty())
                    .map(str::to_owned);
            }
            if title.is_none() && kind == Some("ai-title") {
                title = str_of(&obj, "aiTitle")
                    .filter(|t| !t.is_empty())
                    .map(str::to_owned);
            }
        }
    }
    (name, title)
}

/// The first line of a file as a JSON object (reads at most `bytes`).
pub fn first_json_line(path: &str, bytes: u64) -> Option<Map<String, Value>> {
    let f = File::open(path).ok()?;
    let mut data = Vec::new();
    f.take(bytes).read_to_end(&mut data).ok()?;
    let text = String::from_utf8_lossy(&data);
    match serde_json::from_str(split_lines(&text).next()?) {
        Ok(Value::Object(obj)) => Some(obj),
        _ => None,
    }
}
