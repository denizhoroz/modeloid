//! A trained model's results, read from its run's `runs/` folder (what
//! Ultralytics saved): `results.csv` per epoch, plots, batches and weights.
//! Runs from before `train.toml` kept a summary get theirs from `console.log`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::train::{ImageGroup, RunFile, RunImage, RunResults, RunSummary, CONSOLE_FILE, RUNS_NAME};
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
        if record.summary.is_none() || record.device_name.is_none() {
            if let Ok(text) = fs::read_to_string(self.train_dir(name).join(CONSOLE_FILE)) {
                if record.summary.is_none() {
                    record.summary = Some(summary_from_console(&text)).filter(|s| !s.is_empty());
                }
                if record.device_name.is_none() {
                    record.device_name = device_from_console(&text);
                }
            }
        }
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

/// What a run's console says about its end (for runs whose `train.toml` has
/// no summary): the final validation's "all" row, the speed line, the model
/// summary line and the most GPU memory an epoch line shows. Peak RAM isn't
/// in the console.
fn summary_from_console(text: &str) -> RunSummary {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut summary = RunSummary::default();

    // After the last epoch Ultralytics prints "Validating .../best.pt...", a
    // header naming the score groups, then one row per class, "all" first.
    if let Some(start) = lines.iter().rposition(|l| l.starts_with("Validating ")) {
        let rest = &lines[start..];
        let header = rest.iter().find(|l| l.contains("Instances") || l.contains("top1_acc"));
        let all = rest.iter().map(|l| l.split_whitespace().collect::<Vec<_>>()).find(|t| t.first() == Some(&"all"));
        if let (Some(header), Some(all)) = (header, all) {
            let numbers: Vec<f64> = all[1..].iter().filter_map(|v| v.parse().ok()).collect();
            if header.contains("top1_acc") {
                for (key, value) in ["metrics/accuracy_top1", "metrics/accuracy_top5"].into_iter().zip(numbers) {
                    summary.final_metrics.insert(key.into(), value);
                }
            } else {
                // Groups in header order, e.g. "Box(P R mAP50 mAP50-95) Mask(P ...)"; images and instances come first.
                let mut groups: Vec<(usize, &str)> =
                    [("Box(", "B"), ("Mask(", "M"), ("Pose(", "P")].into_iter().filter_map(|(g, k)| header.find(g).map(|at| (at, k))).collect();
                groups.sort_unstable();
                let scores = numbers.get(2..).unwrap_or_default();
                for ((_, kind), values) in groups.iter().zip(scores.chunks(4)) {
                    for (name, value) in ["precision", "recall", "mAP50", "mAP50-95"].into_iter().zip(values) {
                        summary.final_metrics.insert(format!("metrics/{name}({kind})"), *value);
                    }
                }
            }
        }
    }

    // "Speed: 0.1ms preprocess, 1.6ms inference, 0.0ms loss, 0.7ms postprocess per image"
    if let Some(line) = lines.iter().rev().find_map(|l| l.strip_prefix("Speed:")) {
        for part in line.split(',') {
            let mut words = part.split_whitespace();
            if let (Some(ms), Some(name)) = (words.next().and_then(|w| w.strip_suffix("ms")).and_then(|v| v.parse().ok()), words.next()) {
                summary.speed.insert(name.to_string(), ms);
            }
        }
    }

    // "YOLO26n summary: 260 layers, 2,505,750 parameters, 2,505,750 gradients, 5.9 GFLOPs"
    // (the model as trained; the "(fused)" one after training is smaller).
    if let Some(line) = lines.iter().find_map(|l| l.split_once(" summary: ").map(|(_, rest)| rest).filter(|r| r.contains("parameters"))) {
        for part in line.split(", ") {
            if let Some((value, unit)) = part.trim().split_once(' ') {
                let number = value.replace(',', "");
                match unit {
                    "layers" => summary.layers = number.parse().ok(),
                    "parameters" => summary.parameters = number.parse().ok(),
                    "GFLOPs" => summary.gflops = number.parse().ok(),
                    _ => {}
                }
            }
        }
    }

    // Epoch lines: "96/100  2.98G  0.4326 ..." (GPU memory reserved, in GiB).
    let peak_gib = lines
        .iter()
        .filter_map(|l| {
            let mut t = l.split_whitespace();
            let (epoch, of) = t.next()?.split_once('/')?;
            if epoch.parse::<u32>().is_err() || of.parse::<u32>().is_err() {
                return None;
            }
            t.next()?.strip_suffix('G')?.parse::<f64>().ok()
        })
        .fold(0.0, f64::max);
    if peak_gib > 0.0 {
        summary.peak_gpu_bytes = Some((peak_gib * (1u64 << 30) as f64) as u64);
    }
    summary
}

/// The hardware named in Ultralytics' first line, e.g. "Ultralytics 8.4.171
/// Python-3.12.13 torch-2.14.1 CUDA:0 (NVIDIA GeForce RTX 3060, 12288MiB)"
/// → "NVIDIA GeForce RTX 3060"; "CPU (Intel Core i7-12700)" → the processor.
fn device_from_console(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| l.starts_with("Ultralytics "))?;
    let mut names: Vec<&str> = Vec::new();
    for (at, _) in line.match_indices(" (") {
        let device = line[..at].rsplit(' ').next().unwrap_or_default();
        if !(device == "CPU" || device == "MPS" || device.starts_with("CUDA:")) {
            continue;
        }
        // To the matching ")": CPU names can hold their own, e.g. "Core(TM)".
        let rest = &line[at + 2..];
        let mut depth = 0;
        let Some(end) = rest.char_indices().find_map(|(i, c)| match c {
            '(' => {
                depth += 1;
                None
            }
            ')' if depth == 0 => Some(i),
            ')' => {
                depth -= 1;
                None
            }
            _ => None,
        }) else {
            continue;
        };
        let inside = &rest[..end];
        // GPUs carry their memory after the name.
        let name = inside.rsplit_once(", ").filter(|(_, memory)| memory.ends_with("MiB")).map_or(inside, |(name, _)| name).trim();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    (!names.is_empty()).then(|| names.join(", "))
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

    #[test]
    fn reads_the_end_of_a_console() {
        let console = "\
YOLO26n summary: 260 layers, 2,505,750 parameters, 2,505,750 gradients, 5.9 GFLOPs
      Epoch    GPU_mem   box_loss   cls_loss    l1_loss  Instances       Size
      1/2      2.5G     0.4326     0.1807   0.009338         15        640: 100% 187/187 9.1it/s 20.6s
      2/2      2.98G     0.4197     0.1753   0.009031         17        640: 100% 187/187 9.0it/s 20.7s
                 Class     Images  Instances      Box(P          R      mAP50  mAP50-95): 100% 14/14 8.8it/s 1.6s
                   all        427        598      0.973      0.901      0.954      0.783

Validating runs\\weights\\best.pt...
YOLO26n summary (fused): 120 layers, 2,375,811 parameters, 0 gradients, 5.3 GFLOPs
                 Class     Images  Instances      Box(P          R      mAP50  mAP50-95): 100% 14/14 6.6it/s 2.1s
                   all        427        598      0.974      0.902      0.954      0.785
                   bus        115        158      0.974      0.941      0.978      0.875
Speed: 0.1ms preprocess, 1.6ms inference, 0.0ms loss, 0.7ms postprocess per image
";
        let s = summary_from_console(console);
        assert_eq!(s.final_metrics.get("metrics/precision(B)"), Some(&0.974));
        assert_eq!(s.final_metrics.get("metrics/mAP50-95(B)"), Some(&0.785));
        assert_eq!(s.final_metrics.len(), 4);
        assert_eq!(s.speed.get("inference"), Some(&1.6));
        assert!((s.inference_ms().unwrap() - 2.4).abs() < 1e-9);
        assert_eq!((s.layers, s.parameters, s.gflops), (Some(260), Some(2_505_750), Some(5.9)));
        assert_eq!(s.peak_gpu_bytes, Some((2.98 * (1u64 << 30) as f64) as u64));
        assert_eq!(s.peak_ram_bytes, None);

        let segment = "Validating best.pt...\n Class Images Instances Box(P R mAP50 mAP50-95) Mask(P R mAP50 mAP50-95): 100%\n all 10 20 0.9 0.8 0.7 0.6 0.5 0.4 0.3 0.2\n";
        let s = summary_from_console(segment);
        assert_eq!(s.final_metrics.get("metrics/mAP50(M)"), Some(&0.3));
        assert!(summary_from_console("nothing useful").is_empty());

        let gpu = "New version available\nUltralytics 8.4.171  Python-3.12.13 torch-2.14.1+cu130 CUDA:0 (NVIDIA GeForce RTX 3060, 12288MiB)\n";
        assert_eq!(device_from_console(gpu).as_deref(), Some("NVIDIA GeForce RTX 3060"));
        let cpu = "Ultralytics 8.4.171  Python-3.12.13 torch-2.14.1+cpu CPU (13th Gen Intel Core(TM) i7-13700H)\n";
        assert_eq!(device_from_console(cpu).as_deref(), Some("13th Gen Intel Core(TM) i7-13700H"));
        let gpus = "Ultralytics 8.4.171 torch-2.14.1 CUDA:0 (RTX 3060, 12288MiB), CUDA:1 (RTX 3060, 12288MiB)\n";
        assert_eq!(device_from_console(gpus).as_deref(), Some("RTX 3060"));
        assert_eq!(device_from_console("no header"), None);
    }
}
