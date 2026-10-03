//! A workspace's history: what the user did and what each process ended
//! with, newest first. Kept in `.cache/history/` (clearing the cache clears it).

use serde::{Deserialize, Serialize};

/// How a process, or one of its items, ended. Ordered from best to worst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Done,
    /// Nothing to do, e.g. the dataset was already there.
    Skipped,
    /// Stopped by the user.
    Cancelled,
    /// Didn't finish, but nothing broke: the user has a choice to make.
    Warning,
    Failed,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Outcome::Done => "Done",
            Outcome::Skipped => "Skipped",
            Outcome::Cancelled => "Cancelled",
            Outcome::Warning => "Warning",
            Outcome::Failed => "Failed",
        }
    }
}

/// One thing a process worked on, e.g. a link or a dataset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryItem {
    pub label: String,
    pub outcome: Outcome,
    /// What it made ("Saved to …") or why it didn't finish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One process: where it was started (category › section) and what it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Unix seconds.
    pub started: i64,
    pub finished: i64,
    /// Sidebar group, e.g. "Dataset Manager" (or "Settings").
    pub category: String,
    /// Page, e.g. "Datasets".
    pub section: String,
    /// What was done, e.g. "Download".
    pub process: String,
    pub items: Vec<HistoryItem>,
}

impl HistoryEntry {
    /// The worst way an item ended (`Done` with no items).
    pub fn outcome(&self) -> Outcome {
        self.items.iter().map(|i| i.outcome).max().unwrap_or(Outcome::Done)
    }

    /// "Dataset Manager > Datasets > Download".
    pub fn path(&self) -> String {
        format!("{} > {} > {}", self.category, self.section, self.process)
    }

    /// Whole seconds it took.
    pub fn seconds(&self) -> i64 {
        (self.finished - self.started).max(0)
    }
}

/// An entry with its time written out for the reader ("Today 14:32").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRow {
    pub entry: HistoryEntry,
    pub when: String,
}

/// "45 s", "3 min", "1 h 5 min".
pub fn format_duration(seconds: i64) -> String {
    match seconds {
        s if s < 60 => format!("{s} s"),
        s if s < 3600 => format!("{} min", s / 60),
        s => match (s % 3600) / 60 {
            0 => format!("{} h", s / 3600),
            m => format!("{} h {m} min", s / 3600),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(outcome: Outcome) -> HistoryItem {
        HistoryItem { label: "x".into(), outcome, detail: None }
    }

    #[test]
    fn worst_item_wins() {
        let mut entry = HistoryEntry {
            started: 10,
            finished: 75,
            category: "Dataset Manager".into(),
            section: "Datasets".into(),
            process: "Download".into(),
            items: vec![],
        };
        assert_eq!(entry.outcome(), Outcome::Done);
        entry.items = vec![item(Outcome::Done), item(Outcome::Failed), item(Outcome::Skipped)];
        assert_eq!(entry.outcome(), Outcome::Failed);
        assert_eq!(entry.path(), "Dataset Manager > Datasets > Download");
        assert_eq!(format_duration(entry.seconds()), "1 min");
        assert_eq!(format_duration(3900), "1 h 5 min");
        assert_eq!(format_duration(7200), "2 h");
    }
}
