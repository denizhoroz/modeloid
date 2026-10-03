//! The open workspace's history (see the core's `core::history`): every job
//! when it ends (`api/jobs.rs` records it) and the one-step actions that
//! change the workspace (`record`). Shown by the sidebar's History overlay.

use dioxus::prelude::*;

pub use modeloid_types::history::*;

/// The open workspace's history, newest first (empty without a workspace).
#[post("/api/history")]
pub async fn workspace_history() -> Result<Vec<HistoryRow>, ServerFnError> {
    backend::rows().await.map_err(ServerFnError::new)
}

#[cfg(feature = "server")]
pub use backend::{record, record_in};

#[cfg(feature = "server")]
mod backend {
    use std::path::Path;

    use modeloid::core::history::{format_when, now};
    use modeloid::Workspace;

    use super::{HistoryEntry, HistoryItem, HistoryRow, Outcome};
    use crate::api::workspace;

    pub async fn rows() -> Result<Vec<HistoryRow>, String> {
        tokio::task::spawn_blocking(|| {
            let Some(dir) = workspace::current_dir() else { return Ok(Vec::new()) };
            let ws = Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))?;
            Ok(ws.history().into_iter().map(|entry| HistoryRow { when: format_when(entry.finished), entry }).collect())
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// Adds `entry` to the history of the workspace in `dir`. The history is
    /// a convenience: when it can't be written, the action still counts.
    pub fn record_in(dir: &Path, entry: &HistoryEntry) {
        if let Ok(ws) = Workspace::load_from_dir(dir) {
            if let Err(e) = ws.append_history(entry) {
                eprintln!("Couldn't write the history: {e}");
            }
        }
    }

    /// Records a one-step action that just succeeded in the open workspace,
    /// e.g. `record("Training", "Parameters", "Save parameter set", name, None)`.
    pub fn record(category: &str, section: &str, process: &str, label: impl Into<String>, detail: Option<String>) {
        let Some(dir) = workspace::current_dir() else { return };
        let time = now();
        let entry = HistoryEntry {
            started: time,
            finished: time,
            category: category.into(),
            section: section.into(),
            process: process.into(),
            items: vec![HistoryItem { label: label.into(), outcome: Outcome::Done, detail }],
        };
        record_in(&dir, &entry);
    }
}
