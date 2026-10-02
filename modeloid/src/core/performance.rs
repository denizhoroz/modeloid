//! A trained model's results, read from its run's `runs/` folder (what
//! Ultralytics saved): `results.csv` per epoch, plots, batches and weights.

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::train::{ImageGroup, RunFile, RunImage, RunResults, RUNS_NAME};
use crate::models::workspace::Workspace;

/// Picture types shown on the Performance page.
const IMAGE_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

impl Workspace {
    /// What run `name` left: its record, `results.csv`, pictures and weights.
    /// A run that hasn't written anything yet has empty lists.
    pub fn train_results(&self, name: &str) -> Result<RunResults, crate::Error> {
        crate::validate_dataset_name(name)?;
        let mut record = self.train_record(name)?;
        record.name = name.to_string();
        let runs = self.train_dir(name).join(RUNS_NAME);
        let mut results = RunResults { record, ..Default::default() };

        let csv = runs.join("results.csv");
        if csv.is_file() {
            match fs::read_to_string(&csv) {
                Ok(text) => {
                    let (columns, rows) = parse_results(&text);
                    results.columns = columns;
                    results.rows = rows;
                }
                Err(e) => results.warnings.push(format!("results.csv couldn't be read: {e}")),
            }
        }

        let mut files = Vec::new();
        collect(&runs, &runs, &mut files);
        for (rel, size) in files {
            let ext = rel.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
            if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
                let (group, title) = describe_image(&rel);
                results.images.push(RunImage { path: rel, title, group });
            } else if ext == "pt" {
                results.weights.push(RunFile { path: rel, size });
            }
        }
        results.images.sort_by_key(|i| (i.group, image_rank(&i.path), i.path.clone()));
        results.weights.sort_by_key(|w| (!w.path.ends_with("best.pt"), !w.path.ends_with("last.pt"), w.path.clone()));
        Ok(results)
    }

    /// The file at `rel` inside run `name`'s `runs/` folder, if it is one of
    /// its pictures (nothing outside that folder, no other file types).
    pub fn run_image(&self, name: &str, rel: &str) -> Result<PathBuf, crate::Error> {
        crate::validate_dataset_name(name)?;
        let ext = rel.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
        if !IMAGE_EXTENSIONS.contains(&ext.as_str()) || rel.split(['/', '\\']).any(|part| part == ".." || part.is_empty()) || Path::new(rel).is_absolute() {
            return Err(format!("not a picture of run '{name}': {rel}").into());
        }
        let runs = self.train_dir(name).join(RUNS_NAME);
        let path = runs.join(rel);
        let (root, file) = (fs::canonicalize(&runs)?, fs::canonicalize(&path)?);
        if !file.starts_with(&root) || !file.is_file() {
            return Err(format!("not a picture of run '{name}': {rel}").into());
        }
        Ok(file)
    }
}

/// Files under `dir`, as (path relative to `root` with forward slashes, size).
fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out);
        } else if let (Ok(rel), Ok(meta)) = (path.strip_prefix(root), entry.metadata()) {
            let rel = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            out.push((rel, meta.len()));
        }
    }
}

/// `results.csv`: trimmed headers, and rows of numbers (NaN where a cell isn't one).
fn parse_results(text: &str) -> (Vec<String>, Vec<Vec<f64>>) {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let Some(header) = lines.next() else { return (Vec::new(), Vec::new()) };
    let columns: Vec<String> = header.split(',').map(|c| c.trim().to_string()).collect();
    let rows = lines
        .map(|line| {
            let mut row: Vec<f64> = line.split(',').map(|v| v.trim().parse().unwrap_or(f64::NAN)).collect();
            row.resize(columns.len(), f64::NAN);
            row
        })
        .collect();
    (columns, rows)
}

/// Group and readable title of a picture Ultralytics saves.
fn describe_image(rel: &str) -> (ImageGroup, String) {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let kind = |s: &str| {
        if s.starts_with("Box") {
            " (boxes)"
        } else if s.starts_with("Mask") {
            " (masks)"
        } else if s.starts_with("Pose") {
            " (keypoints)"
        } else {
            ""
        }
    };
    let curve = |s: &str| s.trim_start_matches("Box").trim_start_matches("Mask").trim_start_matches("Pose").to_string();
    match stem {
        "results" => (ImageGroup::Metrics, "Results overview".into()),
        "confusion_matrix" => (ImageGroup::Metrics, "Confusion matrix".into()),
        "confusion_matrix_normalized" => (ImageGroup::Metrics, "Confusion matrix (normalized)".into()),
        s if s.ends_with("_curve") => {
            let name = match curve(s).as_str() {
                "PR_curve" => "Precision–recall curve",
                "P_curve" => "Precision–confidence curve",
                "R_curve" => "Recall–confidence curve",
                "F1_curve" => "F1–confidence curve",
                other => return (ImageGroup::Metrics, other.replace('_', " ")),
            };
            (ImageGroup::Metrics, format!("{name}{}", kind(s)))
        }
        "labels" => (ImageGroup::Labels, "Label distribution".into()),
        "labels_correlogram" => (ImageGroup::Labels, "Label correlogram".into()),
        s if s.starts_with("train_batch") => (ImageGroup::Training, format!("Training batch {}", &s["train_batch".len()..])),
        s if s.starts_with("val_batch") => {
            let rest = &s["val_batch".len()..];
            match rest.split_once('_') {
                Some((n, "labels")) => (ImageGroup::Validation, format!("Validation batch {n}: labels")),
                Some((n, "pred")) => (ImageGroup::Validation, format!("Validation batch {n}: predictions")),
                _ => (ImageGroup::Validation, format!("Validation batch {rest}")),
            }
        }
        s => (ImageGroup::Other, s.replace('_', " ")),
    }
}

/// Order inside a group: the most telling first.
fn image_rank(rel: &str) -> usize {
    const ORDER: [&str; 8] = ["results.", "confusion_matrix_normalized.", "confusion_matrix.", "PR_curve", "F1_curve", "P_curve", "R_curve", "labels."];
    let file = rel.rsplit('/').next().unwrap_or(rel);
    ORDER.iter().position(|o| file.contains(o)).unwrap_or(ORDER.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::train::{TrainRecord, TrainStatus};

    #[test]
    fn reads_a_run_folder() {
        let dir = std::env::temp_dir().join(format!("modeloid-performance-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let ws = Workspace::new("ws".into(), dir.clone());
        ws.make_to_dir().unwrap();
        let run = ws.train_dir("cars");
        fs::create_dir_all(run.join("runs/weights")).unwrap();
        let record = TrainRecord { name: "cars".into(), status: TrainStatus::Done, ..Default::default() };
        fs::write(run.join("train.toml"), toml::to_string(&record).unwrap()).unwrap();
        fs::write(run.join("runs/results.csv"), "  epoch,  train/box_loss,  metrics/mAP50-95(B)\n1, 1.5, 0.2\n2, 1.2, nan\n").unwrap();
        for f in ["val_batch0_pred.jpg", "BoxPR_curve.png", "results.png", "labels.jpg", "train_batch1.jpg", "args.yaml"] {
            fs::write(run.join("runs").join(f), b"x").unwrap();
        }
        fs::write(run.join("runs/weights/last.pt"), b"xx").unwrap();
        fs::write(run.join("runs/weights/best.pt"), b"xxx").unwrap();
        fs::write(dir.join("secret.png"), b"x").unwrap();

        let r = ws.train_results("cars").unwrap();
        assert_eq!(r.columns, ["epoch", "train/box_loss", "metrics/mAP50-95(B)"]);
        assert_eq!(r.rows[0], [1.0, 1.5, 0.2]);
        assert!(r.rows[1][2].is_nan());
        let titles: Vec<&str> = r.images.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Results overview", "Precision–recall curve (boxes)", "Label distribution", "Training batch 1", "Validation batch 0: predictions"]);
        assert_eq!(r.weights.iter().map(|w| (w.path.as_str(), w.size)).collect::<Vec<_>>(), [("weights/best.pt", 3), ("weights/last.pt", 2)]);

        assert!(ws.run_image("cars", "BoxPR_curve.png").is_ok());
        assert!(ws.run_image("cars", "../../../secret.png").is_err());
        assert!(ws.run_image("cars", "weights/best.pt").is_err());
        assert!(ws.run_image("../ws", "runs/results.png").is_err());
        assert!(ws.train_results("missing").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
