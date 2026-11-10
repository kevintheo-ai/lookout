//! Sessions that can come back after a crash or shutdown (Kevin 2026-11-09, idea from T3 Code, which also relaunches
//! with a resume instead of keeping anything alive). Lookout keeps a list of the open Claude sessions on disk. A
//! session leaves the list when its window closes while its terminal runs: Kevin closed it. If the terminal itself is
//! gone (crash, shutdown, quit), the session stays and the menu offers to reopen it with the real `claude --resume`.
//!
//! Warp only: Ghostty did not run the start command reliably (2026-10-23/24), so its sessions are not offered.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::Path;

use serde_json::{Value, json};

use crate::model::{Engine, Pid, TermWindow};
use crate::motion::WARP_BID;

/// An open session as the list remembers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenSession {
    /// "claude:<uuid>".
    pub id: String,
    pub cwd: String,
    pub name: String,
    pub owner_pid: Pid,
}

/// The Claude sessions in Warp windows right now, sorted, so an unchanged set compares equal.
pub fn open_sessions(windows: &[TermWindow]) -> Vec<OpenSession> {
    let mut out: Vec<OpenSession> = windows
        .iter()
        .filter(|w| w.bundle_id == WARP_BID)
        .flat_map(|w| {
            w.sessions
                .iter()
                .filter(|s| s.engine == Engine::Claude && s.alive && uuid_of(&s.id).is_some())
                .map(|s| OpenSession {
                    id: s.id.clone(),
                    cwd: s.cwd.clone(),
                    name: s
                        .digest
                        .as_ref()
                        .and_then(|d| d.name.clone())
                        .unwrap_or_else(|| s.name.clone()),
                    owner_pid: w.owner_pid,
                })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    out
}

/// The next list: what is open now, plus what vanished without its terminal still having windows. A terminal counts as
/// running only by its windows and not by its process id, which macOS can hand to another program after a restart.
/// ponytail: closing the very last window of a terminal that keeps running leaves that session offered; "clear" in the
/// menu takes it off.
pub fn remember(
    old: &[OpenSession],
    live: Vec<OpenSession>,
    windows: &[TermWindow],
) -> Vec<OpenSession> {
    let terminal_runs = |o: &OpenSession| {
        windows
            .iter()
            .any(|w| w.bundle_id == WARP_BID && w.owner_pid == o.owner_pid)
    };
    let kept: Vec<OpenSession> = old
        .iter()
        .filter(|o| !live.iter().any(|l| l.id == o.id) && !terminal_runs(o))
        .cloned()
        .collect();
    let mut out = live;
    out.extend(kept);
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// What the menu offers: on the list and not running anywhere. Resuming a running session in a second terminal would
/// mix both into one transcript.
pub fn restorable<'a>(list: &'a [OpenSession], running: &BTreeSet<&str>) -> Vec<&'a OpenSession> {
    list.iter()
        .filter(|o| !running.contains(o.id.as_str()))
        .collect()
}

/// The uuid of "claude:<uuid>". It goes into a shell command, so only hex digits and hyphens pass.
pub fn uuid_of(id: &str) -> Option<&str> {
    let uuid = id.strip_prefix("claude:")?;
    (uuid.len() == 36 && uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-')).then_some(uuid)
}

/// The Claude folder (of `config_dirs`) that holds this session's transcript; `None`: it is gone, nothing to resume.
pub fn config_dir_of<'a>(config_dirs: &'a [String], uuid: &str) -> Option<&'a str> {
    config_dirs.iter().map(String::as_str).find(|dir| {
        let projects = format!("{dir}/projects");
        fs::read_dir(&projects).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|e| e.path().join(format!("{uuid}.jsonl")).is_file())
        })
    })
}

/// The command that resumes it: the template set for its Claude folder (`resumeCommands`), else plain `claude` pointed
/// at the folder that holds the transcript.
pub fn resume_command(
    templates: &BTreeMap<String, String>,
    config_dir: &str,
    uuid: &str,
) -> String {
    match templates.get(config_dir) {
        Some(template) => template.replace("{id}", uuid),
        None => format!(
            "CLAUDE_CONFIG_DIR='{}' claude --resume {uuid}",
            config_dir.replace('\'', "'\\''")
        ),
    }
}

/// Warp launch configuration: one window, one tab. `name` must equal the file's stem, or Warp opens nothing (measured
/// 2026-10-24); no tab title, Warp would freeze it and Lookout recognises windows by the title the session sets. JSON
/// strings are valid YAML double-quoted scalars once the characters YAML does not take raw are escaped too
/// (`yaml_quoted`). `&& exit`: the window closes with a session that ends normally, and stays open with the error if
/// the start fails.
pub fn warp_launch_yaml(name: &str, cwd: &str, command: &str) -> String {
    let q = yaml_quoted;
    format!(
        "---\nname: {}\nactive_window_index: 0\nwindows:\n  - active_tab_index: 0\n    tabs:\n      - layout:\n          cwd: {}\n          commands:\n            - exec: {}\n",
        q(name),
        q(cwd),
        q(&format!("{command} && exit"))
    )
}

/// A YAML double-quoted scalar: JSON escapes the control characters below U+0020, and every other character outside
/// YAML's printable set goes in as `\uXXXX` too: DEL, the C1 block, U+FFFE/U+FFFF, and the line and paragraph
/// separators that YAML 1.1 reads as line breaks (plan review 2026-11-10: a reader rejects them raw).
fn yaml_quoted(s: &str) -> String {
    Value::from(s)
        .to_string()
        .chars()
        .map(|ch| match ch {
            '\u{7F}'..='\u{9F}' | '\u{2028}' | '\u{2029}' | '\u{FFFE}' | '\u{FFFF}' => {
                format!("\\u{:04X}", u32::from(ch))
            }
            _ => ch.to_string(),
        })
        .collect()
}

/// The URL for a new Warp window in folder `path`, the folder percent-encoded: in a URL `&`, `#` or a space would cut it.
pub fn warp_new_window_url(path: &str) -> String {
    let encoded: String = path
        .bytes()
        .map(|b| match b {
            b'/' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ if b.is_ascii_alphanumeric() => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("warp://action/new_window?path={encoded}")
}

/// A session read back from the file: its terminal is unknown. Process numbers are not kept, because after a restart
/// macOS can give a new Warp the number of the old one, and the list would take it for the same terminal.
pub const NO_TERMINAL: Pid = 0;

/// The list as a file. Nothing readable: an empty list.
pub fn load(path: &Path) -> Vec<OpenSession> {
    let Some(value) = fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    else {
        return vec![];
    };
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let text = |k: &str| v.get(k)?.as_str().map(str::to_owned);
            Some(OpenSession {
                id: text("id").filter(|id| uuid_of(id).is_some())?,
                cwd: text("cwd")?,
                name: text("name")?,
                owner_pid: NO_TERMINAL,
            })
        })
        .collect()
}

/// Written whole and flushed to the disk before it replaces the old file: the list must survive the very crash it is for.
pub fn save(path: &Path, list: &[OpenSession]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let value: Vec<Value> = list
        .iter()
        .map(|o| json!({"id": o.id, "cwd": o.cwd, "name": o.name}))
        .collect();
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(Value::from(value).to_string().as_bytes())?;
    file.sync_all()?;
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fake_window;

    const A: &str = "claude:aaaaaaaa-1111-2222-3333-444444444444";
    const B: &str = "claude:bbbbbbbb-1111-2222-3333-444444444444";

    fn window(id: &str, pid: Pid) -> TermWindow {
        let mut w = fake_window(false, 1.0);
        w.owner_pid = pid;
        w.sessions[0].id = id.to_owned();
        w.sessions[0].cwd = "/work".to_owned();
        w.sessions[0].name = "name".to_owned();
        w
    }

    fn open(id: &str, pid: Pid) -> OpenSession {
        OpenSession {
            id: id.to_owned(),
            cwd: "/work".to_owned(),
            name: "name".to_owned(),
            owner_pid: pid,
        }
    }

    #[test]
    fn open_sessions_are_the_claude_ones_in_warp() {
        let mut ghostty = window(B, 7);
        ghostty.bundle_id = "com.mitchellh.ghostty".to_owned();
        assert_eq!(
            open_sessions(&[window(A, 7), ghostty]),
            [open(A, 7)],
            "Ghostty sessions are not offered"
        );
        assert!(
            open_sessions(&[window("codex:x", 7)]).is_empty(),
            "only Claude sessions can be resumed this way"
        );
    }

    /// Closed by Kevin while the terminal runs: gone. The terminal itself gone: stays on the list.
    #[test]
    fn remember_keeps_only_what_the_terminal_took_with_it() {
        let old = [open(A, 7), open(B, 8)];
        let still = [window("claude:cccccccc-1111-2222-3333-444444444444", 7)];
        let next = remember(&old, open_sessions(&still), &still);
        let ids: Vec<&str> = next.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(
            ids,
            [B, "claude:cccccccc-1111-2222-3333-444444444444"],
            "A was closed in a running terminal, B's terminal (pid 8) is gone"
        );

        assert_eq!(
            remember(&old, vec![], &[]),
            old,
            "after a crash nothing runs: everything stays"
        );
    }

    /// Back after a restart: same id, new terminal. It is one entry again, and no longer offered.
    #[test]
    fn a_resumed_session_is_live_again() {
        let now = [window(A, 99)];
        let next = remember(&[open(A, 7)], open_sessions(&now), &now);
        assert_eq!(next, [open(A, 99)]);
        let running: BTreeSet<&str> = BTreeSet::from([A]);
        assert!(restorable(&next, &running).is_empty());
        assert_eq!(restorable(&next, &BTreeSet::new()).len(), 1);
    }

    #[test]
    fn only_a_real_uuid_reaches_the_command() {
        assert_eq!(uuid_of(A), Some("aaaaaaaa-1111-2222-3333-444444444444"));
        assert_eq!(uuid_of("codex:aaaaaaaa-1111-2222-3333-444444444444"), None);
        assert_eq!(uuid_of("claude:$(rm -rf x)"), None);
        assert_eq!(uuid_of("claude:aaaaaaaa-1111-2222-3333-44444444444;"), None);
    }

    #[test]
    fn the_folder_decides_the_command() {
        let templates = BTreeMap::from([(
            "/u/.claude-acme".to_owned(),
            "/x/launch acme claude --resume {id}".to_owned(),
        )]);
        assert_eq!(
            resume_command(&templates, "/u/.claude-acme", "abc"),
            "/x/launch acme claude --resume abc"
        );
        assert_eq!(
            resume_command(&templates, "/u/.claude", "abc"),
            "CLAUDE_CONFIG_DIR='/u/.claude' claude --resume abc",
            "without a template Claude still has to look in the folder that holds the transcript"
        );
        assert_eq!(
            resume_command(&templates, "/u/it's mine", "abc"),
            "CLAUDE_CONFIG_DIR='/u/it'\\''s mine' claude --resume abc"
        );
    }

    #[test]
    fn the_transcript_finds_its_folder() {
        let root = std::env::temp_dir().join(format!("lookout-restore-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (one, two) = (root.join("one"), root.join("two"));
        fs::create_dir_all(one.join("projects/p")).unwrap();
        fs::create_dir_all(two.join("projects/q")).unwrap();
        fs::write(two.join("projects/q/abc.jsonl"), "{}").unwrap();
        let dirs = [one, two.clone()].map(|p| p.to_string_lossy().into_owned());
        assert_eq!(config_dir_of(&dirs, "abc"), Some(dirs[1].as_str()));
        assert_eq!(config_dir_of(&dirs, "missing"), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn new_window_url_encodes_the_folder() {
        assert_eq!(
            warp_new_window_url("/Users/a.b-c_d"),
            "warp://action/new_window?path=/Users/a.b-c_d"
        );
        assert_eq!(
            warp_new_window_url("/x y&z#ü"),
            "warp://action/new_window?path=/x%20y%26z%23%C3%BC"
        );
    }

    #[test]
    fn launch_file_quotes_what_it_is_given() {
        let yaml = warp_launch_yaml("n", "/a \"b\"/c", "claude --resume x");
        assert!(yaml.contains("name: \"n\"\n"));
        assert!(yaml.contains("cwd: \"/a \\\"b\\\"/c\"\n"));
        assert!(yaml.contains("exec: \"claude --resume x && exit\"\n"));
        assert!(!yaml.contains("title"), "a tab title would freeze Warp's");
    }

    /// Nothing outside YAML's printable characters goes in raw (plan review 2026-11-10), and the escapes read back as
    /// the text (`\uXXXX` means the same in JSON and YAML).
    #[test]
    fn launch_file_escapes_what_yaml_does_not_take_raw() {
        let text = "a\u{7F}b\u{85}c\u{9F}d\u{2028}e\u{FFFE}f\u{FFFF}g\u{1}h ä😀";
        let quoted = yaml_quoted(text);
        let printable = |ch: char| {
            matches!(u32::from(ch), 0x20..=0x7E | 0xA0..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
                && !matches!(ch, '\u{2028}' | '\u{2029}')
        };
        assert!(quoted.chars().all(printable), "{quoted:?}");
        assert!(quoted.contains("\\u007F") && quoted.contains("\\uFFFF"));
        assert_eq!(serde_json::from_str::<String>(&quoted).unwrap(), text);
    }

    #[test]
    fn the_list_survives_the_disk() {
        let dir = std::env::temp_dir().join(format!("lookout-list-{}", std::process::id()));
        let path = dir.join("sub/open.json");
        let list = [open(A, 7), open(B, 8)];
        save(&path, &list).unwrap();
        assert_eq!(load(&path), [open(A, NO_TERMINAL), open(B, NO_TERMINAL)]);
        assert!(!path.with_extension("tmp").exists(), "nothing left behind");

        // After a restart a new Warp can get the old process number: the list must not take it for the old terminal.
        let again = [window("claude:cccccccc-1111-2222-3333-444444444444", 7)];
        let next = remember(&load(&path), open_sessions(&again), &again);
        assert_eq!(next.len(), 3, "A and B stay offered");

        fs::write(&path, "kaputt").unwrap();
        assert!(load(&path).is_empty(), "a broken file is an empty list");
        assert!(load(&dir.join("fehlt.json")).is_empty());
        fs::write(&path, r#"[{"id":"claude:$(x)","cwd":"/a","name":"n"}]"#).unwrap();
        assert!(load(&path).is_empty(), "the file is not trusted either");
        fs::remove_dir_all(&dir).unwrap();
    }
}
