//! Training a model: what the page shows before starting (`TrainPreview`)
//! and the record each run keeps in `training/trains/<name>/train.toml`.

use serde::{Deserialize, Serialize};

/// Checked before starting: where it goes, what runs, and what's in the way.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainPreview {
    /// `training/trains/<name>`, absolute.
    pub run_dir: String,
    /// Ultralytics model argument, e.g. "yolo11n.pt".
    pub model: String,
    /// What training runs on, e.g. "GPU (CUDA)".
    pub device: String,
    /// Anything here blocks training.
    pub problems: Vec<String>,
    /// Good to know, e.g. "COCO labels are converted to YOLO in the run folder".
    pub notes: Vec<String>,
}

impl TrainPreview {
    pub fn can_run(&self) -> bool {
        self.problems.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrainStatus {
    #[default]
    Running,
    Done,
    Failed,
    Stopped,
}

/// `train.toml` in a run's folder.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrainRecord {
    pub name: String,
    pub dataset_id: u32,
    pub dataset_name: String,
    /// The parameter set's name; a copy of it is saved as `parameters.toml`.
    pub parameters: String,
    pub model: String,
    pub task: String,
    pub device: String,
    pub epochs: u32,
    #[serde(default)]
    pub epochs_done: u32,
    pub started: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished: Option<String>,
    #[serde(default)]
    pub status: TrainStatus,
    /// Best weights, e.g. `.../runs/weights/best.pt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A piece of a run's console output, read from `offset` on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainLog {
    pub text: String,
    /// Where the next read starts.
    pub offset: u64,
}

/// Strips terminal colour codes (`ESC [ ... letter`) from console output.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colours() {
        assert_eq!(strip_ansi("\u{1b}[34m\u{1b}[1mtrain: \u{1b}[0mScanning"), "train: Scanning");
        assert_eq!(strip_ansi("plain"), "plain");
    }
}
