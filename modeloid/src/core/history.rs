//! The workspace's history (`.cache/history/history.jsonl`, one
//! `HistoryEntry` per line, oldest first). Clearing the cache removes it.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use chrono::{DateTime, Local, TimeZone};

use crate::models::workspace::Workspace;

pub use modeloid_types::history::*;

const HISTORY_FILE: &str = "history.jsonl";
/// Entries kept when the file grows past `TRIM_BYTES`; older ones are dropped.
pub const MAX_ENTRIES: usize = 1000;
const TRIM_BYTES: u64 = 2 * 1024 * 1024;

impl Workspace {
    pub fn history_dir(&self) -> PathBuf {
        self.path().join(".cache").join("history")
    }

    fn history_file(&self) -> PathBuf {
        self.history_dir().join(HISTORY_FILE)
    }

    /// Adds `entry` at the end of the history.
    pub fn append_history(&self, entry: &HistoryEntry) -> Result<(), crate::Error> {
        fs::create_dir_all(self.history_dir())?;
        let mut line = serde_json::to_string(entry)?;
        line.push('\n');
        fs::OpenOptions::new().create(true).append(true).open(self.history_file())?.write_all(line.as_bytes())?;
        self.trim_history(TRIM_BYTES);
        Ok(())
    }

    /// Every entry, newest first. Unreadable lines are skipped.
    pub fn history(&self) -> Vec<HistoryEntry> {
        let Ok(text) = fs::read_to_string(self.history_file()) else { return Vec::new() };
        let mut entries: Vec<HistoryEntry> = text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
        entries.reverse();
        entries
    }

    /// Keeps the newest `MAX_ENTRIES` once the file is bigger than `max_bytes`.
    fn trim_history(&self, max_bytes: u64) {
        let path = self.history_file();
        if fs::metadata(&path).map_or(true, |m| m.len() <= max_bytes) {
            return;
        }
        let Ok(text) = fs::read_to_string(&path) else { return };
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() <= MAX_ENTRIES {
            return;
        }
        let mut kept = lines[lines.len() - MAX_ENTRIES..].join("\n");
        kept.push('\n');
        let _ = fs::write(&path, kept);
    }
}

/// The current time in Unix seconds.
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// When `timestamp` was, in this computer's time zone: "Today 14:32",
/// "Yesterday 09:10", "Oct 1, 14:32", or with the year when it isn't this one.
pub fn format_when(timestamp: i64) -> String {
    format_when_at(timestamp, Local::now())
}

fn format_when_at<Tz: TimeZone>(timestamp: i64, now: DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let Some(when) = now.timezone().timestamp_opt(timestamp, 0).single() else { return String::new() };
    let days = (now.date_naive() - when.date_naive()).num_days();
    let time = when.format("%H:%M");
    match days {
        0 => format!("Today {time}"),
        1 => format!("Yesterday {time}"),
        _ if when.format("%Y").to_string() == now.format("%Y").to_string() => when.format("%b %-d, %H:%M").to_string(),
        _ => when.format("%b %-d %Y, %H:%M").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn entry(process: &str) -> HistoryEntry {
        HistoryEntry {
            started: 1,
            finished: 2,
            category: "Dataset Manager".into(),
            section: "Datasets".into(),
            process: process.into(),
            items: vec![HistoryItem { label: "a".into(), outcome: Outcome::Done, detail: Some("Saved to x".into()) }],
        }
    }

    #[test]
    fn appends_reads_newest_first_and_trims() {
        let dir = std::env::temp_dir().join(format!("modeloid-history-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let ws = Workspace::new("h".into(), dir.clone());
        ws.make_to_dir().unwrap();

        assert!(ws.history().is_empty());
        ws.append_history(&entry("first")).unwrap();
        ws.append_history(&entry("second")).unwrap();
        fs::OpenOptions::new().append(true).open(ws.history_file()).unwrap().write_all(b"not json\n").unwrap();
        let processes: Vec<String> = ws.history().into_iter().map(|e| e.process).collect();
        assert_eq!(processes, ["second", "first"], "newest first, broken lines skipped");

        let mut lines = String::new();
        for i in 0..MAX_ENTRIES + 5 {
            lines.push_str(&serde_json::to_string(&entry(&i.to_string())).unwrap());
            lines.push('\n');
        }
        fs::write(ws.history_file(), lines).unwrap();
        ws.trim_history(u64::MAX);
        assert_eq!(ws.history().len(), MAX_ENTRIES + 5, "small enough: left alone");
        ws.trim_history(0);
        let history = ws.history();
        assert_eq!(history.len(), MAX_ENTRIES);
        assert_eq!(history[0].process, (MAX_ENTRIES + 4).to_string(), "the newest stay");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_times_for_people() {
        let now = Utc.with_ymd_and_hms(2026, 10, 3, 15, 0, 0).unwrap();
        let at = |y, mo, d, h, mi| Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp();
        assert_eq!(format_when_at(at(2026, 10, 3, 14, 32), now), "Today 14:32");
        assert_eq!(format_when_at(at(2026, 10, 2, 9, 5), now), "Yesterday 09:05");
        assert_eq!(format_when_at(at(2026, 10, 1, 8, 0), now), "Oct 1, 08:00");
        assert_eq!(format_when_at(at(2025, 12, 31, 23, 0), now), "Dec 31 2025, 23:00");
    }
}
