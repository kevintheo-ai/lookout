//! Session status (wave B1), ported from Watcher.swift: which AI session runs, and is it done?
//!
//! - `claude`: Claude sessions from the status files per configuration folder, `claude agents --json` every 5 minutes
//!   and as a cross-check (`ClaudeStatus`), the "without window" list.
//! - `transcript`: what Claude's transcript tells (topic, step, answer, turn end, peers, own timers).
//! - `words`: what an answer text says: the session's own Lookout line and the word lists for "needs nothing".
//! - `watch`: done despite a background command (`BackgroundWatch`, plan 1e) and waits only for another session or a
//!   timer (`Standby`): "watching" instead of done.
//! - `logs`: Codex, Grok and Kimi report their turn end in their session logs (`LogSessionFinder`).
//!
//! macOS-only sampling stays out (stage 2 passes it in): process start (`sysctl KERN_PROC_PID`), whether a pid lives
//! (`kill(pid, 0)`), start arguments (`KERN_PROCARGS2`), the nettop and lsof calls. Log lines go to `report` closures;
//! their German text is Swift's, byte for byte.

mod claude;
mod logs;
mod regex;
mod transcript;
mod watch;
mod words;

#[cfg(test)]
mod tests;

pub use claude::*;
pub use logs::*;
pub use transcript::*;
pub use watch::*;
pub use words::*;

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::model::Time;

/// A log sink (Swift `report: (String) -> Void`, by default `log`).
pub type Report = Box<dyn FnMut(&str) + Send>;

type Obj = Map<String, Value>;

/// `Date()`.
fn now() -> Time {
    secs(SystemTime::now())
}

fn secs(t: SystemTime) -> Time {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    }
}

// ---------- JSON values as JSONSerialization + `as?` bridge them (measured 2026-11-06) ----------

/// `as? Int` and `as? Bool`: the same bridging the config reader uses.
use crate::config::{json_bool as as_bool, json_int as as_int};

/// `as? Double`: numbers that a double holds exactly (2^53 + 1 does not), booleans as 0 and 1.
fn as_double(v: &Value) -> Option<f64> {
    match v {
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Number(n) => {
            let exact = |f: f64, back: i128| (f as i128 == back).then_some(f);
            if let Some(i) = n.as_i64() {
                exact(i as f64, i128::from(i))
            } else if let Some(u) = n.as_u64() {
                exact(u as f64, i128::from(u))
            } else {
                n.as_f64()
            }
        }
        _ => None,
    }
}

/// `as? [[String: Any]]`: an array of objects only; one other element and the cast fails.
fn objects(v: &Value) -> Option<Vec<&Obj>> {
    v.as_array()?.iter().map(Value::as_object).collect()
}

// ---------- Strings and paths with Foundation's semantics ----------

/// Swift `split(separator: "\n")`: non-empty pieces between line feeds. "\r\n" is one character in Swift and never a
/// separator (measured).
fn split_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        loop {
            if rest.is_empty() {
                return None;
            }
            let b = rest.as_bytes();
            let cut = (0..b.len()).find(|&i| b[i] == b'\n' && (i == 0 || b[i - 1] != b'\r'));
            let (piece, next) = match cut {
                Some(i) => (&rest[..i], &rest[i + 1..]),
                None => (rest, ""),
            };
            rest = next;
            if !piece.is_empty() {
                return Some(piece);
            }
        }
    })
}

/// `removingPercentEncoding`: `None` for a '%' without two hex digits or bytes that are no UTF-8 (measured).
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s
                .get(i + 1..i + 3)
                .filter(|h| h.bytes().all(|c| c.is_ascii_hexdigit()))?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `contentsOfDirectory(atPath:)` names; unreadable = none.
fn dir_names(path: &str) -> Vec<String> {
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}
