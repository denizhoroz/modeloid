//! Training a model: what the page shows before starting (`TrainPreview`)
//! and the record each run keeps in `training/trains/<name>/train.toml`.

use std::collections::BTreeMap;

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
    /// `training/trains/<name>` exists already; blocks training too. Kept
    /// apart from `problems` so the page can tell it from the run it just
    /// started under that name.
    #[serde(default)]
    pub name_taken: bool,
    /// Good to know, e.g. "COCO labels are converted to YOLO in the run folder".
    pub notes: Vec<String>,
}

impl TrainPreview {
    pub fn can_run(&self) -> bool {
        self.problems.is_empty() && !self.name_taken
    }

    /// What stops training under `name`, if anything.
    pub fn blocker(&self, name: &str) -> Option<String> {
        if self.name_taken {
            Some(name_taken_message(name))
        } else {
            self.problems.first().cloned()
        }
    }
}

pub fn name_taken_message(name: &str) -> String {
    format!("A model named '{name}' already exists in training/trains. Choose another name.")
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
    /// The hardware's own name, e.g. "NVIDIA GeForce RTX 3060" (runs from
    /// before it was saved have none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
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
    /// What the run measured at its end; runs from before it was saved have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<RunSummary>,
}

/// The end of a run: the final validation of the best weights, the model's
/// size, its speed and the memory training took. Every part is optional:
/// Ultralytics versions and devices differ in what they give.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    /// Official final scores (best weights validated once more), by
    /// `results.csv` column, e.g. "metrics/mAP50-95(B)" → 0.785.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub final_metrics: BTreeMap<String, f64>,
    /// Milliseconds per image in that validation: "preprocess", "inference",
    /// "loss", "postprocess".
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub speed: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layers: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gflops: Option<f64>,
    /// Most GPU memory reserved while training.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_gpu_bytes: Option<u64>,
    /// Most RAM the training process used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_ram_bytes: Option<u64>,
}

impl RunSummary {
    pub fn is_empty(&self) -> bool {
        *self == RunSummary::default()
    }

    /// Milliseconds per image from input to result (everything but the loss).
    pub fn inference_ms(&self) -> Option<f64> {
        let parts: Vec<f64> = ["preprocess", "inference", "postprocess"].iter().filter_map(|k| self.speed.get(*k).copied()).collect();
        (!parts.is_empty()).then(|| parts.iter().sum())
    }
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

/// What a run left in its `runs/` folder, for the Performance page.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunResults {
    pub record: TrainRecord,
    /// `results.csv` headers, trimmed (e.g. "epoch", "train/box_loss", "metrics/mAP50(B)").
    pub columns: Vec<String>,
    /// One row per epoch; cells that aren't numbers are NaN.
    pub rows: Vec<Vec<f64>>,
    /// Plots and example batches, in showing order.
    pub images: Vec<RunImage>,
    /// `weights/*.pt`.
    pub weights: Vec<RunFile>,
    /// Files that couldn't be read, as messages.
    pub warnings: Vec<String>,
}

impl RunResults {
    /// Values of column `name`, one per epoch.
    pub fn column(&self, name: &str) -> Option<Vec<f64>> {
        let i = self.columns.iter().position(|c| c == name)?;
        Some(self.rows.iter().map(|r| r.get(i).copied().unwrap_or(f64::NAN)).collect())
    }

    /// The metric that picks the best epoch: mAP50-95 (boxes, then masks),
    /// else top-1 accuracy, else the first metric.
    pub fn key_metric(&self) -> Option<&str> {
        ["metrics/mAP50-95(B)", "metrics/mAP50-95(M)", "metrics/accuracy_top1"]
            .into_iter()
            .find(|k| self.columns.iter().any(|c| c == k))
            .or_else(|| self.columns.iter().map(String::as_str).find(|c| c.starts_with("metrics/")))
    }

    /// Row of the best epoch by `key_metric` (highest); `None` without metrics.
    pub fn best_row(&self) -> Option<usize> {
        let values = self.column(self.key_metric()?)?;
        values.iter().enumerate().filter(|(_, v)| v.is_finite()).max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i)
    }
}

/// A picture Ultralytics saved, relative to the run's `runs/` folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunImage {
    /// e.g. "BoxPR_curve.png", forward slashes.
    pub path: String,
    /// e.g. "Precision–recall curve (boxes)".
    pub title: String,
    pub group: ImageGroup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ImageGroup {
    /// Curves, confusion matrices, the results overview.
    Metrics,
    /// What the labels look like (label distribution, correlogram).
    Labels,
    /// Augmented training batches.
    Training,
    /// Validation batches: labels and predictions.
    Validation,
    Other,
}

impl ImageGroup {
    pub fn label(self) -> &'static str {
        match self {
            ImageGroup::Metrics => "Curves and confusion matrices",
            ImageGroup::Labels => "Labels",
            ImageGroup::Training => "Training batches",
            ImageGroup::Validation => "Validation: labels and predictions",
            ImageGroup::Other => "Other images",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunFile {
    /// Relative to `runs/`, e.g. "weights/best.pt".
    pub path: String,
    pub size: u64,
}

/// Readable name of a column, e.g. "train/box_loss" → "Box loss",
/// "metrics/mAP50-95(B)" → "mAP50-95 (boxes)".
pub fn column_title(column: &str) -> String {
    let name = column.rsplit('/').next().unwrap_or(column);
    let (name, kind) = match name.strip_suffix("(B)") {
        Some(n) => (n, " (boxes)"),
        None => match name.strip_suffix("(M)") {
            Some(n) => (n, " (masks)"),
            None => (name, ""),
        },
    };
    let base = match name {
        "box_loss" => "Box loss".to_string(),
        "cls_loss" => "Class loss".to_string(),
        "dfl_loss" => "DFL loss".to_string(),
        "seg_loss" => "Mask loss".to_string(),
        "pose_loss" => "Pose loss".to_string(),
        "kobj_loss" => "Keypoint loss".to_string(),
        "loss" => "Loss".to_string(),
        "precision" => "Precision".to_string(),
        "recall" => "Recall".to_string(),
        "accuracy_top1" => "Top-1 accuracy".to_string(),
        "accuracy_top5" => "Top-5 accuracy".to_string(),
        "pg0" | "pg1" | "pg2" => "Learning rate".to_string(),
        n if n.starts_with("mAP") => n.to_string(),
        n => {
            let n = n.replace('_', " ");
            let mut c = n.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
    };
    format!("{base}{kind}")
}

#[cfg(test)]
mod results_tests {
    use super::*;

    #[test]
    fn names_columns_and_picks_the_best_epoch() {
        assert_eq!(column_title("train/box_loss"), "Box loss");
        assert_eq!(column_title("metrics/mAP50-95(B)"), "mAP50-95 (boxes)");
        assert_eq!(column_title("metrics/precision(M)"), "Precision (masks)");
        assert_eq!(column_title("metrics/accuracy_top1"), "Top-1 accuracy");

        let r = RunResults {
            columns: vec!["epoch".into(), "metrics/mAP50(B)".into(), "metrics/mAP50-95(B)".into()],
            rows: vec![vec![1.0, 0.5, 0.2], vec![2.0, 0.6, 0.4], vec![3.0, 0.7, f64::NAN], vec![4.0, 0.6, 0.3]],
            ..Default::default()
        };
        assert_eq!(r.key_metric(), Some("metrics/mAP50-95(B)"));
        assert_eq!(r.best_row(), Some(1));
        assert_eq!(RunResults::default().best_row(), None);
    }
}
