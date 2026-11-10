//! The programs' model lists, asked once a day (Kevin 2026-11-10: "nur einmal am tag"): each program with a
//! `modelsCommand` names its current models, one per line (`launch::model_lines`). The lists are kept on disk with the
//! day they were asked, so a restart on the same day asks nothing. A command that fails or says nothing keeps the list
//! there was.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread::sleep;
use std::time::{Duration, Instant};

use lookout_core::launch::{model_lines, models_cache_from_json, models_cache_to_json};

/// How long one command may take.
const LIMIT: Duration = Duration::from_secs(30);
/// How much of its output counts.
const OUTPUT_MAX: u64 = 64 * 1024;

fn cache_path() -> PathBuf {
    PathBuf::from(format!(
        "{}/Library/Application Support/Lookout/models.json",
        crate::home()
    ))
}

/// The lists kept on disk, with their day.
pub fn load() -> Option<(String, BTreeMap<String, Vec<String>>)> {
    models_cache_from_json(&std::fs::read_to_string(cache_path()).ok()?)
}

/// Asks every program off the main thread, one after the other, keeps what came back with `day`, and hands it to `done`
/// on the main thread. A program without an answer keeps its last list from the disk (review quick 2026-11-10: a
/// passing network error must not lose it over a restart).
pub fn refresh(
    commands: Vec<(String, Vec<String>)>,
    day: String,
    done: impl FnOnce(BTreeMap<String, Vec<String>>) + Send + 'static,
) {
    std::thread::spawn(move || {
        let mut last = load().map(|(_, lists)| lists).unwrap_or_default();
        let mut lists = BTreeMap::new();
        for (program, words) in commands {
            let answer = match run(&words, LIMIT).map(|out| model_lines(&out)) {
                Some(models) if !models.is_empty() => {
                    crate::log(&format!("Modelle: {program} {}", models.len()));
                    Some(models)
                }
                Some(_) => {
                    crate::log(&format!("Modelle: {program} ohne Antwort"));
                    None
                }
                None => {
                    crate::log(&format!("Modelle nicht abgefragt: {program}"));
                    None
                }
            };
            if let Some(models) = answer.or_else(|| last.remove(&program)) {
                lists.insert(program, models);
            }
        }
        let text = models_cache_to_json(&day, &lists);
        let path = cache_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, text) {
            crate::log(&format!("Modelle nicht gespeichert: {e}"));
        }
        crate::on_main(move || done(lists));
    });
}

/// The command's output, `None` if it cannot start, fails, or takes longer than `limit` to end and to close its output.
/// The output is read beside it: a background process the command leaves behind can hold the output open (review quick
/// 2026-11-10). ponytail: the reading thread of such a command stays until that process ends.
fn run(words: &[String], limit: Duration) -> Option<String> {
    let (program, args) = words.split_first()?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // At most `OUTPUT_MAX` bytes: a process left behind that keeps writing must not fill the memory (review quick
    // 2026-11-10); a model list is a few hundred bytes.
    let mut stdout = child.stdout.take()?.take(OUTPUT_MAX);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = String::new();
        let _ = tx.send(stdout.read_to_string(&mut out).ok().map(|_| out));
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if start.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => sleep(Duration::from_millis(100)),
        }
    }
    rx.recv_timeout(limit.saturating_sub(start.elapsed()))
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    /// The answer of a command that ends well; nothing from one that fails, cannot start or takes too long.
    #[test]
    fn a_command_answers_or_counts_as_failed() {
        let second = Duration::from_secs(1);
        assert_eq!(
            run(&words(&["/bin/echo", "a"]), second).as_deref(),
            Some("a\n")
        );
        assert_eq!(run(&words(&["/usr/bin/false"]), second), None);
        assert_eq!(run(&words(&["/no/such/program"]), second), None);
        assert_eq!(run(&[], second), None);
        let start = Instant::now();
        assert_eq!(
            run(&words(&["/bin/sleep", "5"]), Duration::from_millis(300)),
            None
        );
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "stopped at the limit"
        );
        // Ends at once but leaves a process behind that holds the output open (review quick 2026-11-10).
        let start = Instant::now();
        assert_eq!(
            run(
                &words(&["/bin/sh", "-c", "echo m; sleep 5 &"]),
                Duration::from_millis(500)
            ),
            None
        );
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "the reading has a limit too"
        );
        // Endless output is not collected (review quick 2026-11-10): reading stops at `OUTPUT_MAX`, the writer loses its
        // pipe, and that counts as failed.
        let start = Instant::now();
        assert_eq!(run(&words(&["/bin/sh", "-c", "yes model"]), second), None);
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}
