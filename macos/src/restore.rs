//! Reopens sessions in Warp after a crash or shutdown (`lookout_core::restore` decides which): one launch configuration
//! per window, opened through `warp://launch/<name>`, the way `/handoff` opens its windows.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use lookout_core::launch::launch_exec;
use lookout_core::motion::GHOSTTY_BID;
use lookout_core::restore::{
    OpenSession, config_dir_of, resume_command, uuid_of, warp_launch_yaml, warp_new_window_url,
};

/// Between two windows: every Claude start runs its hooks and servers, twenty at once would stall the Mac.
const PAUSE: Duration = Duration::from_secs(3);
/// Warp reads the file when it handles the URL; a cold start takes a while. Left behind, it would stay in Warp's list.
const KEEP_FILE: Duration = Duration::from_secs(60);
/// The environment `open` passes on: Lookout may have been started from inside a session, whose markers must not reach
/// the new windows.
const KEEP_ENV: [&str; 8] = [
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "SSH_AUTH_SOCK",
    "LANG",
    "__CF_USER_TEXT_ENCODING",
];

/// Where the list of open sessions is kept.
pub fn list_path() -> PathBuf {
    PathBuf::from(format!(
        "{}/Library/Application Support/Lookout/open-sessions.json",
        crate::home()
    ))
}

/// Opens each session in its own Warp window, one after the other, off the main thread.
pub fn reopen(
    list: Vec<OpenSession>,
    config_dirs: Vec<String>,
    templates: BTreeMap<String, String>,
) {
    std::thread::spawn(move || {
        let dir = PathBuf::from(format!("{}/.warp/launch_configurations", crate::home()));
        let mut files = vec![];
        for (k, o) in list.iter().enumerate() {
            if k > 0 {
                sleep(PAUSE);
            }
            match open_one(o, &config_dirs, &templates, &dir) {
                Ok(file) => {
                    crate::log(&format!("Wiederhergestellt: {}", o.name));
                    files.push(file);
                }
                Err(why) => crate::log(&format!("Nicht wiederhergestellt: {} ({why})", o.name)),
            }
        }
        sleep(KEEP_FILE);
        for file in files {
            let _ = fs::remove_file(file);
        }
    });
}

/// A command with nothing of Lookout's environment but `KEEP_ENV`.
fn clean_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
    for key in KEEP_ENV {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }
    command
}

/// Opens a new empty window of the given terminal, off the main thread: Warp in the home folder through its URL, Ghostty
/// through its AppleScript (`open -n` would start a second Ghostty).
pub fn new_window(bundle_id: String) {
    std::thread::spawn(move || {
        let mut command = if bundle_id == GHOSTTY_BID {
            let mut c = clean_command("/usr/bin/osascript");
            c.args(["-e", "tell application \"Ghostty\" to new window"]);
            c
        } else {
            let mut c = clean_command("/usr/bin/open");
            c.arg(warp_new_window_url(&crate::home()));
            c
        };
        match command.status() {
            Ok(status) if status.success() => {}
            Ok(status) => crate::log(&format!("Neues Fenster: {status}")),
            Err(e) => crate::log(&format!("Neues Fenster: {e}")),
        }
    });
}

fn open_one(
    o: &OpenSession,
    config_dirs: &[String],
    templates: &BTreeMap<String, String>,
    dir: &Path,
) -> Result<PathBuf, String> {
    let uuid = uuid_of(&o.id).ok_or("keine gueltige Kennung")?;
    let config = config_dir_of(config_dirs, uuid).ok_or("kein Transkript gefunden")?;
    let command = resume_command(templates, config, uuid);
    // The folder can be gone (a removed worktree): Claude finds the session by its id from anywhere.
    let cwd = if Path::new(&o.cwd).is_dir() {
        o.cwd.clone()
    } else {
        crate::home()
    };
    let name = format!("lookout-restore-{uuid}");
    let file = dir.join(format!("{name}.yaml"));
    write_private(&file, &warp_launch_yaml(&name, &cwd, &command))?;
    let failed = match open_launch(&name) {
        Ok(()) => return Ok(file),
        Err(why) => why,
    };
    let _ = fs::remove_file(&file);
    Err(failed)
}

/// A new session in a new Warp window (the inbox's +, Kevin 2026-11-10), off the main thread. The start script
/// (`launch::start_script`) goes to the temporary folder; the launch configuration only runs `/bin/sh <script>`, so the
/// task never meets the user's shell. The script removes itself when it runs; both files go after `KEEP_FILE`, at once
/// if Warp cannot be asked.
pub fn start_new(script: String, cwd: String, label: String) {
    std::thread::spawn(move || {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let name = format!("lookout-new-{stamp}-{}", std::process::id());
        let script_file = std::env::temp_dir().join(format!("{name}.sh"));
        let launch_file = PathBuf::from(format!(
            "{}/.warp/launch_configurations/{name}.yaml",
            crate::home()
        ));
        let started = launch_exec(&script_file.to_string_lossy())
            .ok_or_else(|| "Temp-Ordner mit Sonderzeichen im Pfad".to_owned())
            .and_then(|exec| {
                write_private(&script_file, &script)?;
                write_private(&launch_file, &warp_launch_yaml(&name, &cwd, &exec))?;
                open_launch(&name)
            });
        match started {
            Ok(()) => {
                crate::log(&format!("Neue Session: {label}"));
                sleep(KEEP_FILE);
            }
            Err(why) => crate::log(&format!("Neue Session nicht gestartet: {label} ({why})")),
        }
        let _ = fs::remove_file(&script_file);
        let _ = fs::remove_file(&launch_file);
    });
}

/// Writes `text` to `file`, readable only by the user: a start line can hold a task.
fn write_private(file: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(file)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .map_err(|e| e.to_string())
}

/// Asks Warp to open the launch configuration `name`.
fn open_launch(name: &str) -> Result<(), String> {
    let mut open = clean_command("/usr/bin/open");
    open.arg(format!("warp://launch/{name}"));
    match open.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("open: {status}")),
        Err(e) => Err(e.to_string()),
    }
}
