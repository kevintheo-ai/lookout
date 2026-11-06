//! Settings from `~/.config/lookout.json`. Missing and invalid keys keep their default. Reading the file is the
//! platform layer's job; this module interprets its bytes. Values are read the way Swift's `JSONSerialization`
//! bridges them (`as? Int` takes 5.0 and `true`, `as? Bool` takes 0 and 1), measured 2026-11-06.

use serde_json::{Map, Value};

use crate::model::trim_ws;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppConfig {
    /// Start arrangement until one is chosen in the menu (then the stored choice).
    pub layout: String,
    pub stack_side: String,
    pub ready_threshold_sec: u64,
    pub poll_sec: u64,
    /// `claude agents --json` costs ~0.25 s CPU per call (measured 2026-09-18), so it runs less often than the cheap
    /// AX tick.
    pub session_poll_sec: u64,
    pub hotkeys: bool,
    pub glow: bool,
    /// Done glow as hex. Default mint: "done", not "alarm" (Kevin 2026-09-18).
    pub glow_color: String,
    /// Own colour for "waits for your answer": warm amber, no alarm red.
    pub glow_color_waiting: String,
    /// Labels in the windows (design 2026-09-21); only the start value, then the menu item counts.
    pub labels: bool,
    /// "de" or "en"; `None` = the Mac's language (`prefers_german`).
    pub language: Option<String>,
    /// Claude configurations whose sessions Lookout shows (one per `CLAUDE_CONFIG_DIR`); missing ones are skipped.
    pub claude_config_dirs: Vec<String>,
    /// Own names (`--name`, `/rename`; no auto title) of sessions that lead other sessions: with a resting watcher only
    /// a question makes them done, not their report about other windows (Kevin 2026-10-09).
    pub lead_names: Vec<String>,
    /// Name parts of windows where Kevin answers OTHER sessions' questions: a "?" Lookout line that only sends him there
    /// does not bring this window forward (Kevin 2026-10-13).
    pub other_window_names: Vec<String>,
    /// Name parts of worker sessions: running or watching, their windows go to the parking lot behind the main window
    /// (Kevin 2026-11-06).
    pub worker_names: Vec<String>,
}

impl AppConfig {
    /// Defaults; `home` is the user's home folder (Swift `NSHomeDirectory()`).
    pub fn new(home: &str) -> AppConfig {
        AppConfig {
            layout: "reading".into(),
            stack_side: "right".into(),
            ready_threshold_sec: 90,
            poll_sec: 3,
            session_poll_sec: 6,
            hotkeys: true,
            glow: true,
            glow_color: "#5ECFA0".into(),
            glow_color_waiting: "#E8B25C".into(),
            labels: true,
            language: None,
            claude_config_dirs: vec![format!("{home}/.claude")],
            lead_names: vec![],
            other_window_names: vec![],
            worker_names: vec![],
        }
    }
}

/// The configuration file's bytes; anything but a JSON object gives the defaults. A UTF-8 byte order mark is skipped
/// like `JSONSerialization` does. ponytail: UTF-16 files (Foundation reads them) and duplicate keys (Foundation keeps
/// the first, serde_json the last) are not matched.
pub fn config_from_json(data: &[u8], home: &str) -> AppConfig {
    let data = data.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(data);
    match serde_json::from_slice::<Value>(data) {
        Ok(Value::Object(obj)) => parse_config(&obj, home),
        _ => AppConfig::new(home),
    }
}

/// Settings from the configuration's JSON object.
pub fn parse_config(obj: &Map<String, Value>, home: &str) -> AppConfig {
    let mut c = AppConfig::new(home);
    let string = |k: &str| obj.get(k).and_then(Value::as_str);
    let positive = |k: &str| {
        obj.get(k)
            .and_then(json_int)
            .filter(|v| *v > 0)
            .and_then(|v| u64::try_from(v).ok())
    };
    let flag = |k: &str| obj.get(k).and_then(json_bool);
    if let Some(v) = string("layout") {
        c.layout = v.into();
    }
    if let Some(v) = string("stackSide").filter(|v| *v == "left" || *v == "right") {
        c.stack_side = v.into();
    }
    if let Some(v) = positive("readyThresholdSec") {
        c.ready_threshold_sec = v;
    }
    if let Some(v) = positive("pollSec") {
        c.poll_sec = v;
    }
    if let Some(v) = positive("sessionPollSec") {
        c.session_poll_sec = v;
    }
    if let Some(v) = flag("hotkeys") {
        c.hotkeys = v;
    }
    if let Some(v) = flag("glow") {
        c.glow = v;
    }
    if let Some(v) = string("glowColor") {
        c.glow_color = v.into();
    }
    if let Some(v) = string("glowColorWaiting") {
        c.glow_color_waiting = v.into();
    }
    if let Some(v) = flag("labels") {
        c.labels = v;
    }
    if let Some(v) = string("language").filter(|v| *v == "de" || *v == "en") {
        c.language = Some(v.into());
    }
    // `~` resolved, "~/.claude/" like "~/.claude": whether `fetchAgents` asks without CLAUDE_CONFIG_DIR depends on the
    // default folder. Empty and relative entries are no folder and drop out (review quick, Kimi).
    let dirs: Vec<String> = strings(obj, "claudeConfigDirs")
        .map(|s| standardize_path(trim_ws(s), home))
        .filter(|p| p.starts_with('/'))
        .collect();
    if !dirs.is_empty() {
        c.claude_config_dirs = dirs;
    }
    c.lead_names = names(obj, "leadNames");
    c.other_window_names = names(obj, "otherWindowNames");
    c.worker_names = names(obj, "workerNames");
    c
}

/// The strings of an array value; other values and other element types are skipped.
fn strings<'a>(obj: &'a Map<String, Value>, key: &str) -> impl Iterator<Item = &'a str> {
    obj.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

/// Name list: trimmed, empty ones dropped.
fn names(obj: &Map<String, Value>, key: &str) -> Vec<String> {
    strings(obj, key)
        .map(trim_ws)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Swift `as? Int` on a JSON value: integers in range, integral floats, booleans as 0 and 1.
fn json_int(v: &Value) -> Option<i64> {
    match v {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n.as_i64().or_else(|| {
            let f = n.as_f64()?;
            // 2^63 itself is not representable as i64.
            (f.fract() == 0.0
                && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&f))
            .then_some(f as i64)
        }),
        _ => None,
    }
}

/// Swift `as? Bool` on a JSON value: booleans, and numbers that are exactly 0 or 1.
fn json_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => {
            let f = n.as_f64()?;
            if f == 0.0 {
                Some(false)
            } else if f == 1.0 {
                Some(true)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// `NSString.standardizingPath` for configuration folders: `~` and `~/…` expanded, "//", "/./", ".." and a trailing
/// slash resolved; relative paths unchanged. ponytail: lexical only. Foundation also expands `~user`, resolves ".."
/// through symlinks and strips a leading "/private" when the result exists; that needs the file system.
fn standardize_path(path: &str, home: &str) -> String {
    let expanded = match path.strip_prefix('~') {
        Some("") => home.to_owned(),
        Some(rest) if rest.starts_with('/') => format!("{home}{rest}"),
        _ => path.to_owned(),
    };
    if !expanded.starts_with('/') {
        return expanded;
    }
    let mut parts: Vec<&str> = vec![];
    for part in expanded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

/// Hex colour -> sRGB components 0…1. Unreadable falls back to mint, never to red.
pub fn color_from_hex(hex: &str) -> (f64, f64, f64) {
    let s = trim_ws(hex);
    let s = s.strip_prefix('#').unwrap_or(s);
    // Swift's `UInt32(_:radix:)` takes a sign too; "-00000" is 0.
    match i64::from_str_radix(s, 16) {
        Ok(v) if s.chars().count() == 6 && v >= 0 => {
            let c = |shift: u32| ((v >> shift) & 0xFF) as f64 / 255.0;
            (c(16), c(8), c(0))
        }
        _ => (0.369, 0.812, 0.627),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HOME: &str = "/Users/test";

    fn parse(v: Value) -> AppConfig {
        match v {
            Value::Object(obj) => parse_config(&obj, HOME),
            _ => panic!("test config must be an object"),
        }
    }

    /// Settings from `~/.config/lookout.json`: every key, plus `language` and `claudeConfigDirs`.
    #[test]
    fn test_parse_config() {
        let home = HOME;
        let d = parse(json!({}));
        assert!(
            d.language.is_none() && d.claude_config_dirs == [format!("{home}/.claude")],
            "default: Mac language"
        );
        assert!(
            d.layout == "reading"
                && d.stack_side == "right"
                && d.ready_threshold_sec == 90
                && d.poll_sec == 3
                && d.session_poll_sec == 6
                && d.hotkeys
                && d.glow
                && d.labels
                && d.glow_color == "#5ECFA0"
                && d.glow_color_waiting == "#E8B25C",
            "previous defaults"
        );
        let c = parse(
            json!({"language": "en", "claudeConfigDirs": ["~/.claude", "~/.claude-work/", "/opt/claude", 7],
                             "layout": "focus2", "stackSide": "left", "readyThresholdSec": 120, "pollSec": 5,
                             "sessionPollSec": 10, "hotkeys": false, "glow": false, "glowColor": "#FFFFFF",
                             "glowColorWaiting": "#000000", "labels": false}),
        );
        assert_eq!(c.language.as_deref(), Some("en"));
        assert_eq!(
            c.claude_config_dirs,
            [
                format!("{home}/.claude"),
                format!("{home}/.claude-work"),
                "/opt/claude".into()
            ],
            "~ resolved, trailing slash gone, unreadable dropped"
        );
        assert!(
            c.layout == "focus2"
                && c.stack_side == "left"
                && c.ready_threshold_sec == 120
                && c.poll_sec == 5
                && c.session_poll_sec == 10
                && !c.hotkeys
                && !c.glow
                && c.glow_color == "#FFFFFF"
                && c.glow_color_waiting == "#000000"
                && !c.labels,
            "every key arrives"
        );
        assert!(
            parse(json!({"language": "fr"})).language.is_none(),
            "unknown language: Mac language"
        );
        assert_eq!(
            parse(json!({"claudeConfigDirs": []})).claude_config_dirs,
            [format!("{home}/.claude")],
            "empty"
        );
        assert_eq!(
            parse(json!({"claudeConfigDirs": "~/.claude-work"})).claude_config_dirs,
            [format!("{home}/.claude")],
            "not an array"
        );
        // review quick R2 (2026-10-01, Kimi): empty and relative entries are no folder; [""] must not push the default out.
        assert_eq!(
            parse(json!({"claudeConfigDirs": ["", "  ", "relativ/claude"]})).claude_config_dirs,
            [format!("{home}/.claude")],
            "only unusable entries: default"
        );
        assert_eq!(
            parse(json!({"claudeConfigDirs": ["", " ~/.claude-work "]})).claude_config_dirs,
            [format!("{home}/.claude-work")],
            "spaces at the edge gone, empty dropped"
        );
        assert_eq!(
            parse(json!({"pollSec": 0, "stackSide": "oben"})).poll_sec,
            3,
            "invalid: default"
        );
        assert!(
            d.lead_names.is_empty()
                && parse(json!({"leadNames": [" Beta · Lead ", "", 7]})).lead_names
                    == ["Beta · Lead"],
            "lead names: edges trimmed, empty and unreadable dropped"
        );
        assert!(
            d.other_window_names.is_empty()
                && parse(json!({"otherWindowNames": [" front desk ", "", 7]})).other_window_names
                    == ["front desk"],
            "other windows: edges trimmed, empty and unreadable dropped"
        );
        // GP 14: the key wins over the Mac's language.
        use crate::i18n::prefers_german;
        let langs = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(
            prefers_german(Some("de"), &langs(&["en-US"]))
                && !prefers_german(Some("en"), &langs(&["de-CH"]))
        );
        assert!(
            prefers_german(None, &langs(&["de-CH", "en"]))
                && !prefers_german(None, &langs(&["en-US", "de"]))
        );
        assert!(!prefers_german(None, &[]), "without any: English");
    }

    #[test]
    fn worker_names_parse_like_other_window_names() {
        assert!(parse(json!({})).worker_names.is_empty());
        assert_eq!(
            parse(json!({"workerNames": [" Worker ", "", 7, "\tbuild\u{A0}"]})).worker_names,
            ["Worker", "build"]
        );
        assert!(
            parse(json!({"workerNames": "worker"}))
                .worker_names
                .is_empty(),
            "not an array"
        );
    }

    /// Swift `JSONSerialization` + `as?`, measured 2026-11-06.
    #[test]
    fn values_bridge_like_foundation() {
        let c = parse(
            json!({"pollSec": 5.0, "readyThresholdSec": true, "sessionPollSec": 5.5, "hotkeys": 0,
                             "glow": 2, "labels": 1.0}),
        );
        assert!(c.poll_sec == 5 && c.ready_threshold_sec == 1 && c.session_poll_sec == 6);
        assert!(!c.hotkeys && c.glow && c.labels);
        assert_eq!(
            parse(json!({"pollSec": 9_223_372_036_854_775_808u64})).poll_sec,
            3,
            "beyond Int: default"
        );
        assert_eq!(
            config_from_json(b"[1]", HOME),
            AppConfig::new(HOME),
            "not an object: defaults"
        );
        assert_eq!(config_from_json(br#"{"pollSec": 4}"#, HOME).poll_sec, 4);
        assert_eq!(
            config_from_json(b"\xEF\xBB\xBF{\"pollSec\": 4}", HOME).poll_sec,
            4,
            "UTF-8 BOM"
        );
        let dirs = parse(
            json!({"claudeConfigDirs": ["~", "~/", "/a//b/./c/", "/a/b/../c", "/..", "~other/x"]}),
        );
        assert_eq!(
            dirs.claude_config_dirs,
            [HOME, HOME, "/a/b/c", "/a/c", "/"],
            "~user needs the system: dropped"
        );
    }

    #[test]
    fn hex_colors() {
        assert_eq!(color_from_hex(" #FF0080 "), (1.0, 0.0, 128.0 / 255.0));
        assert_eq!(
            color_from_hex("red"),
            (0.369, 0.812, 0.627),
            "unreadable: mint"
        );
        assert_eq!(color_from_hex("#FFF"), (0.369, 0.812, 0.627));
        assert_eq!(color_from_hex("-00000"), (0.0, 0.0, 0.0));
    }
}
