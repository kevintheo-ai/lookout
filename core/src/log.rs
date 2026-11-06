//! The log file (Common.swift `appendLog`): one line per event with date, rotated at a size. Same line format as the
//! Swift app, so both logs can be compared with diff. Which file and when to write is the platform layer's business.

use std::fs::{self, OpenOptions};
use std::io::Write;

/// From this size the log moves to `<path>.1` (one generation to look back is enough; about 4 weeks).
pub const LOG_ROTATE_BYTES: u64 = 5 << 20;

/// Appends `message` as one line "yyyy-MM-dd HH:mm:ss  message" (local time; with the date since review 2026-09-25:
/// without it an endless loop from before v3 looked current). If the file has reached `rotate_at` bytes, a new one
/// starts first and the old one becomes `.1`. A failed rotation keeps appending to the old file, never overwrites it
/// (review quick 2026-09-25). Errors are swallowed: logging never stops the app.
pub fn append_log(message: &str, path: &str, rotate_at: u64) {
    let line = format!(
        "{}  {message}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    if fs::metadata(path).is_ok_and(|m| m.len() >= rotate_at) {
        let old = format!("{path}.1");
        let _ = fs::remove_file(&old);
        let _ = fs::rename(path, &old);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Swift `testLogWithDateAndRotation`.
    #[test]
    fn log_with_date_and_rotation() {
        let dir = std::env::temp_dir().join(format!("lookout-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Lookout.log").to_string_lossy().into_owned();
        append_log("erste Zeile", &path, 50);
        let first = fs::read_to_string(&path).unwrap();
        let (stamp, rest) = first.split_at(19);
        assert!(
            stamp.len() == 19
                && stamp.bytes().enumerate().all(|(i, b)| match i {
                    4 | 7 => b == b'-',
                    10 => b == b' ',
                    13 | 16 => b == b':',
                    _ => b.is_ascii_digit(),
                })
                && rest == "  erste Zeile\n",
            "Zeile mit Datum, bekam {first}"
        );
        append_log("zweite", &path, 50);
        append_log("dritte", &path, 50);
        let old = fs::read_to_string(format!("{path}.1")).unwrap_or_default();
        let new = fs::read_to_string(&path).unwrap();
        assert!(
            old.contains("erste Zeile") && old.contains("zweite") && !old.contains("dritte"),
            "ab 50 Bytes rotiert: {old}"
        );
        assert!(
            new.contains("dritte") && !new.contains("zweite"),
            "neues Log beginnt frisch: {new}"
        );

        // The rotation fails (`.1` is a directory that cannot be removed): keep appending, never overwrite (review
        // quick 2026-09-25, GLM + Kimi: else the new line replaced the whole history).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let stuck = dir.join("Stuck.log").to_string_lossy().into_owned();
            let blocked = format!("{stuck}.1");
            fs::create_dir_all(&blocked).unwrap();
            fs::write(format!("{blocked}/inhalt"), "x").unwrap();
            fs::set_permissions(&blocked, fs::Permissions::from_mode(0o555)).unwrap();
            append_log("alt eins", &stuck, 50);
            append_log("alt zwei", &stuck, 50);
            append_log("neu", &stuck, 50);
            let kept = fs::read_to_string(&stuck).unwrap();
            fs::set_permissions(&blocked, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(
                kept.contains("alt eins") && kept.contains("neu"),
                "Rotation gescheitert: Historie bleibt, bekam {kept}"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
