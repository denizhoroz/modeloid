//! Training a model with Ultralytics in the Python environment.
//!
//! A run lives in `training/trains/<name>/`:
//!
//! ```text
//! train.toml        TrainRecord: dataset, parameters, device, status, best weights, end summary
//! parameters.toml   the parameter set as it was when the run started
//! data.yaml         the dataset for Ultralytics (or dataset/data.yaml when converted)
//! dataset/          YOLO copy of a COCO / VOC dataset (YOLO datasets are used in place)
//! config.json       what main.py gets: model, task, Ultralytics arguments
//! resume.json       what main.py gets to continue a stopped run from runs/weights/last.pt
//! console.log       Ultralytics' output, finished lines only
//! runs/             Ultralytics' results: weights/best.pt, plots, results.csv
//! ```

use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::core::labels::{self, LabelFormat};
use crate::core::progress::{Reporter, Unit};
use crate::core::pyenv::{self, Output, PyEnv, TrainDevice};
use crate::models::parameters::{AutoAugment, Batch, Cache, Family, FrameworkParams, ParameterSet, Task, UltralyticsParams};
use crate::models::workspace::Workspace;

pub use modeloid_types::train::*;

const RECORD_FILE: &str = "train.toml";
const PARAMETERS_FILE: &str = "parameters.toml";
pub(crate) const CONSOLE_FILE: &str = "console.log";
/// Ultralytics' run folder name inside a run (`project` = the run folder).
pub(crate) const RUNS_NAME: &str = "runs";

/// Where a dataset's images are for Ultralytics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DataSource {
    /// A YOLO dataset used where it is: `data.yaml` points at its folders.
    YoloInPlace { yaml: String, val_is_train: bool },
    /// COCO / VOC (or YOLO in an odd layout): written as YOLO into `dataset/`.
    Convert(LabelFormat),
    /// A classification dataset (class folders), passed as it is.
    Folders(PathBuf),
}

/// Everything checked before a run.
pub(crate) struct Plan {
    pub(crate) preview: TrainPreview,
    pub(crate) dataset_dir: PathBuf,
    pub(crate) dataset_name: String,
    pub(crate) data: Option<DataSource>,
}

/// A run ready to start (folder written).
#[derive(Debug, Clone)]
pub struct PreparedTrain {
    pub dir: PathBuf,
    pub config: PathBuf,
    pub record: TrainRecord,
}

pub(crate) fn ultra(set: &ParameterSet) -> &UltralyticsParams {
    let FrameworkParams::Ultralytics(p) = &set.params;
    p
}

/// The device Ultralytics gets: the set's own, or for "auto" the one chosen in Settings.
pub fn resolve_device(set: &ParameterSet, choice: TrainDevice) -> String {
    match ultra(set).run.device.as_str() {
        "auto" | "" => choice.ultralytics_device().to_string(),
        device => device.to_string(),
    }
}

pub(crate) fn device_label(device: &str) -> String {
    match device {
        "cpu" => "CPU".into(),
        "mps" => "Apple GPU (MPS)".into(),
        ids => format!("GPU {ids} (CUDA)"),
    }
}

/// `train` / `val` / `test` image folders of a YOLO dataset, relative to it:
/// `<split>/images` (Roboflow) or `images/<split>` (Ultralytics).
fn yolo_layout(dir: &Path) -> Option<(String, Option<String>, Option<String>)> {
    let find = |names: &[&str]| {
        names.iter().flat_map(|n| [format!("{n}/images"), format!("images/{n}")]).find(|rel| dir.join(rel).is_dir())
    };
    let train = find(&["train"])?;
    Some((train, find(&["valid", "val", "validation"]), find(&["test"])))
}

/// `path` made absolute against the current folder (paths handed to Python
/// must not depend on its working folder).
pub(crate) fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn quote(name: &str) -> String {
    format!("'{}'", name.replace('\'', "''"))
}

impl Workspace {
    /// `training/trains/<name>`, absolute (Python runs from another folder).
    pub fn train_dir(&self, name: &str) -> PathBuf {
        absolute(&self.trains_dir().join(name))
    }

    fn train_plan(&self, id: u32, set: &ParameterSet, name: &str, choice: TrainDevice) -> Result<Plan, crate::Error> {
        self.run_plan(id, set, name, &self.train_dir(name), choice)
    }

    /// Checks running `set` on dataset `id` in folder `run_dir` (a training or
    /// a tuning run named `name`): the name, the parameters, the task against
    /// the dataset, and how the data reaches Ultralytics.
    pub(crate) fn run_plan(&self, id: u32, set: &ParameterSet, name: &str, run_dir: &Path, choice: TrainDevice) -> Result<Plan, crate::Error> {
        let (dataset_dir, dataset) = self.find_dataset(id)?;
        let dataset_dir = absolute(&dataset_dir);
        let p = ultra(set);
        let device = resolve_device(set, choice);
        let mut preview = TrainPreview {
            run_dir: run_dir.display().to_string(),
            model: p.model_name(),
            device: device_label(&device),
            ..Default::default()
        };
        let problems = &mut preview.problems;
        if let Err(e) = crate::validate_dataset_name(name) {
            problems.push(format!("Model name: {e}"));
        } else if run_dir.exists() {
            preview.name_taken = true;
        }
        problems.extend(set.problems().into_iter().map(|p| format!("Parameters: {p}")));

        let kind = dataset.dataset_type();
        let data = match p.model.task {
            Task::Pose | Task::Obb => {
                problems.push(format!(
                    "Training {} models isn't supported yet: Modeloid's labels don't hold keypoints or rotated boxes.",
                    p.model.task.label().to_lowercase()
                ));
                None
            }
            Task::Classify if kind == "classification" => Some(DataSource::Folders(dataset_dir.clone())),
            Task::Classify => {
                problems.push(format!("A classify model needs a classification dataset (class folders); {} is {kind}.", dataset.name()));
                None
            }
            _ if kind == "classification" => {
                problems.push(format!("{} is a classification dataset; choose the Classify task, or another dataset.", dataset.name()));
                None
            }
            Task::Segment if kind != "instance-segmentation" => {
                problems.push(format!("A segment model needs outlines (polygons); {} has boxes only ({kind}).", dataset.name()));
                None
            }
            _ => match LabelFormat::parse(dataset.annotation_format()) {
                None => {
                    problems.push(format!("'{}' labels can't be trained on yet; YOLO, COCO and Pascal VOC can.", dataset.annotation_format()));
                    None
                }
                Some(LabelFormat::Yolo) if !dataset.class_names().is_empty() => match yolo_layout(&dataset_dir) {
                    Some((train, val, test)) => {
                        let val_is_train = val.is_none();
                        let mut yaml = format!("path: {}\ntrain: {train}\nval: {}\n", quote(&dataset_dir.display().to_string()), val.unwrap_or_else(|| train.clone()));
                        if let Some(test) = test {
                            yaml.push_str(&format!("test: {test}\n"));
                        }
                        let names: Vec<String> = dataset.class_names().iter().map(|c| quote(c)).collect();
                        yaml.push_str(&format!("nc: {}\nnames: [{}]\n", names.len(), names.join(", ")));
                        Some(DataSource::YoloInPlace { yaml, val_is_train })
                    }
                    None => Some(DataSource::Convert(LabelFormat::Yolo)),
                },
                Some(format) => Some(DataSource::Convert(format)),
            },
        };
        match &data {
            Some(DataSource::YoloInPlace { val_is_train: true, .. }) => {
                preview.notes.push("The dataset has no validation split, so training validates on the train images. Split & Balance can make one.".into())
            }
            Some(DataSource::Convert(LabelFormat::Yolo)) => {
                preview.notes.push("The dataset's folder layout isn't one Ultralytics reads, so a YOLO copy is written into the run folder.".into())
            }
            Some(DataSource::Convert(format)) => {
                preview.notes.push(format!("{} labels are converted to YOLO in the run folder (the dataset stays as it is).", format.name().to_uppercase()))
            }
            _ => {}
        }
        if matches!(p.model.weights, crate::models::parameters::Weights::Pretrained) {
            preview.notes.push(format!("The pretrained {} weights download on the first run that uses them.", p.model_name()));
        }
        Ok(Plan { preview, dataset_dir, dataset_name: dataset.name().to_string(), data })
    }

    /// What training dataset `id` with `set` as `name` would do, and what's in the way.
    pub fn train_preview(&self, id: u32, set: &ParameterSet, name: &str, choice: TrainDevice) -> Result<TrainPreview, crate::Error> {
        Ok(self.train_plan(id, set, name, choice)?.preview)
    }

    /// Creates the run folder: record, parameter copy, data (converting labels
    /// if needed, counted on `reporter`) and `config.json`.
    pub fn prepare_train(&self, id: u32, set: &ParameterSet, name: &str, choice: TrainDevice, reporter: &dyn Reporter) -> Result<PreparedTrain, crate::Error> {
        let plan = self.train_plan(id, set, name, choice)?;
        if let Some(problem) = plan.preview.blocker(name) {
            return Err(problem.into());
        }
        let p = ultra(set);
        let dir = self.train_dir(name);
        fs::create_dir_all(&dir)?;

        let data_path = self.write_data(id, plan.data.expect("no problems means data"), &dir, reporter)?;

        let device = resolve_device(set, choice);
        let args = ultralytics_args(p, &device, &data_path, &dir);
        let config = dir.join("config.json");
        let rtdetr = p.model.family == Family::RtDetr;
        fs::write(
            &config,
            serde_json::to_vec_pretty(&json!({ "model": p.model_name(), "task": p.model.task.id(), "rtdetr": rtdetr, "args": args }))?,
        )?;
        fs::write(dir.join(PARAMETERS_FILE), toml::to_string(set)?)?;

        let record = TrainRecord {
            name: name.to_string(),
            dataset_id: id,
            dataset_name: plan.dataset_name,
            parameters: set.name.clone(),
            model: p.model_name(),
            task: p.model.task.id().to_string(),
            device: device_label(&device),
            epochs: p.training.epochs,
            started: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            ..Default::default()
        };
        save_record(&dir, &record)?;
        let _ = plan.dataset_dir;
        Ok(PreparedTrain { dir, config, record })
    }

    /// Checks continuing stopped run `name` from its last epoch: the run, its
    /// saved parameters (Resume on) and `last.pt`.
    fn resume_plan(&self, name: &str, choice: TrainDevice) -> Result<(TrainPreview, TrainRecord, ParameterSet), crate::Error> {
        let record = self.train_record(name)?;
        let dir = self.train_dir(name);
        let set: ParameterSet = toml::from_str(&fs::read_to_string(dir.join(PARAMETERS_FILE)).map_err(|e| format!("{name} has no saved parameters: {e}"))?)?;
        let device = resolve_device(&set, choice);
        let mut preview = TrainPreview { run_dir: dir.display().to_string(), model: record.model.clone(), device: device_label(&device), ..Default::default() };
        if record.status != TrainStatus::Stopped {
            preview.problems.push(format!("Only a stopped run can be continued; {name} is {}.", status_word(record.status)));
        } else if !ultra(&set).run.resume {
            preview.problems.push(format!("Resume is off in its parameters ({}).", set.name));
        } else if !last_weights(&dir).is_file() {
            preview.problems.push("It stopped before its first epoch ended, so there's nothing to continue from. Train it again instead.".into());
        } else {
            preview.notes.push(format!("Continues from epoch {} of {}, with the learning rate and optimizer where they were.", record.epochs_done, record.epochs));
        }
        Ok((preview, record, set))
    }

    /// What continuing stopped run `name` would do, and what's in the way.
    pub fn resume_preview(&self, name: &str, choice: TrainDevice) -> Result<TrainPreview, crate::Error> {
        Ok(self.resume_plan(name, choice)?.0)
    }

    /// Readies stopped run `name` to continue: `resume.json` for main.py, the
    /// record back to running. Its dataset copy and settings are the ones
    /// saved in `last.pt`; only the device may change.
    pub fn prepare_resume(&self, name: &str, choice: TrainDevice) -> Result<PreparedTrain, crate::Error> {
        let (preview, mut record, set) = self.resume_plan(name, choice)?;
        if let Some(problem) = preview.problems.first() {
            return Err(problem.clone().into());
        }
        let dir = self.train_dir(name);
        let p = ultra(&set);
        let device = resolve_device(&set, choice);
        let config = dir.join("resume.json");
        fs::write(
            &config,
            serde_json::to_vec_pretty(&json!({
                "model": last_weights(&dir).display().to_string(),
                "task": p.model.task.id(),
                "rtdetr": p.model.family == Family::RtDetr,
                "resume": true,
                "args": { "device": device },
            }))?,
        )?;
        let mut console = File::options().create(true).append(true).open(dir.join(CONSOLE_FILE))?;
        writeln!(console, "\n[modeloid] Continuing from epoch {} of {}\n", record.epochs_done, record.epochs)?;

        record.status = TrainStatus::Running;
        record.device = device_label(&device);
        record.finished = None;
        record.error = None;
        save_record(&dir, &record)?;
        Ok(PreparedTrain { dir, config, record })
    }

    /// Puts dataset `id` where Ultralytics reads it for a run in `dir`; returns
    /// the `data` argument (a data.yaml, or the class-folder dataset).
    pub(crate) fn write_data(&self, id: u32, data: DataSource, dir: &Path, reporter: &dyn Reporter) -> Result<PathBuf, crate::Error> {
        Ok(match data {
            DataSource::YoloInPlace { yaml, .. } => {
                let path = dir.join("data.yaml");
                fs::write(&path, yaml)?;
                path
            }
            DataSource::Convert(_) => {
                let progress = reporter.start("Converting labels to YOLO", Unit::Files);
                let (_, _, set) = self.read_labels(id, progress.as_ref())?;
                let out = dir.join("dataset");
                labels::write(&set, &out, LabelFormat::Yolo, progress.as_ref())?;
                progress.finish();
                out.join("data.yaml")
            }
            DataSource::Folders(folder) => folder,
        })
    }

    /// Every training run's record, by start time, and a warning for each
    /// run folder whose `train.toml` couldn't be read (those are left out).
    pub fn train_records(&self) -> Result<(Vec<TrainRecord>, Vec<String>), crate::Error> {
        let Ok(entries) = fs::read_dir(self.trains_dir()) else { return Ok((Vec::new(), Vec::new())) };
        let (mut records, mut warnings) = (Vec::new(), Vec::new());
        for entry in entries.filter_map(Result::ok).filter(|e| e.path().is_dir()) {
            let path = entry.path().join(RECORD_FILE);
            let read = fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| toml::from_str::<TrainRecord>(&t).map_err(|e| e.to_string()));
            match read {
                // The folder name is the name; a hand-renamed folder keeps working.
                Ok(mut record) => {
                    record.name = entry.file_name().to_string_lossy().into_owned();
                    records.push(record);
                }
                Err(e) => warnings.push(format!("{} couldn't be read: {e}", path.display())),
            }
        }
        records.sort_by(|a, b| a.started.cmp(&b.started).then_with(|| a.name.cmp(&b.name)));
        Ok((records, warnings))
    }

    /// Deletes trained model `name`: its whole run folder (weights, results,
    /// console log). Permanent. Only a folder directly inside `training/trains`.
    pub fn delete_train(&self, name: &str) -> Result<(), crate::Error> {
        crate::validate_dataset_name(name)?;
        let dir = self.train_dir(name);
        if dir.parent() != Some(absolute(&self.trains_dir()).as_path()) || !dir.is_dir() {
            return Err(format!("there's no trained model named '{name}'").into());
        }
        crate::core::io::retry_busy(&dir, || fs::remove_dir_all(&dir))?;
        Ok(())
    }

    /// The record of run `name`.
    pub fn train_record(&self, name: &str) -> Result<TrainRecord, crate::Error> {
        // Only a folder directly inside `training/trains` ("../x" would load any last.pt).
        crate::validate_dataset_name(name)?;
        let text = fs::read_to_string(self.train_dir(name).join(RECORD_FILE)).map_err(|e| format!("no training named '{name}': {e}"))?;
        Ok(toml::from_str(&text)?)
    }

    /// Console output of run `name` from byte `offset` (at most `max` bytes, cut at a line end).
    pub fn train_log(&self, name: &str, offset: u64, max: usize) -> Result<TrainLog, crate::Error> {
        crate::validate_dataset_name(name)?;
        read_log(&self.train_dir(name).join(CONSOLE_FILE), offset, max)
    }
}

/// A console log from byte `offset` (at most `max` bytes, cut at a line end).
pub(crate) fn read_log(path: &Path, offset: u64, max: usize) -> Result<TrainLog, crate::Error> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = File::open(path) else { return Ok(TrainLog { text: String::new(), offset: 0 }) };
    let len = file.metadata()?.len();
    let offset = offset.min(len);
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(max as u64).read_to_end(&mut bytes)?;
    if offset + (bytes.len() as u64) < len {
        // Stop at the last full line; the rest comes next time.
        if let Some(end) = bytes.iter().rposition(|&b| b == b'\n') {
            bytes.truncate(end + 1);
        }
    }
    Ok(TrainLog { text: String::from_utf8_lossy(&bytes).into_owned(), offset: offset + bytes.len() as u64 })
}

/// Ultralytics' checkpoint after the last finished epoch, with the optimizer state.
fn last_weights(run_dir: &Path) -> PathBuf {
    run_dir.join(RUNS_NAME).join("weights").join("last.pt")
}

fn status_word(status: TrainStatus) -> &'static str {
    match status {
        TrainStatus::Running => "still running",
        TrainStatus::Done => "done",
        TrainStatus::Failed => "failed",
        TrainStatus::Stopped => "stopped",
    }
}

fn save_record(dir: &Path, record: &TrainRecord) -> Result<(), crate::Error> {
    fs::write(dir.join(RECORD_FILE), toml::to_string(record)?)?;
    Ok(())
}

/// Ultralytics `train()` keyword arguments for these settings.
pub fn ultralytics_args(p: &UltralyticsParams, device: &str, data: &Path, run_dir: &Path) -> Value {
    let (t, s, a, l, r) = (&p.training, &p.schedule, &p.augmentation, &p.loss, &p.run);
    let mut m = Map::new();
    let mut set = |k: &str, v: Value| {
        m.insert(k.to_string(), v);
    };
    set("data", json!(data.display().to_string()));
    set("project", json!(run_dir.display().to_string()));
    set("name", json!(RUNS_NAME));
    set("exist_ok", json!(true));
    set("verbose", json!(true));
    // Training
    set("epochs", json!(t.epochs));
    set("batch", match t.batch {
        Batch::Auto => json!(-1),
        Batch::Size(n) => json!(n),
    });
    set("imgsz", json!(t.imgsz));
    set("patience", json!(t.patience));
    set("optimizer", json!(t.optimizer.id()));
    set("lr0", json!(t.lr0));
    set("rect", json!(t.rect));
    set("multi_scale", json!(t.multi_scale));
    set("single_cls", json!(t.single_cls));
    set("fraction", json!(t.fraction));
    if let Some(n) = t.freeze {
        set("freeze", json!(n));
    }
    set("dropout", json!(t.dropout));
    set("val", json!(t.val));
    // Schedule
    set("lrf", json!(s.lrf));
    set("cos_lr", json!(s.cos_lr));
    set("momentum", json!(s.momentum));
    set("weight_decay", json!(s.weight_decay));
    set("warmup_epochs", json!(s.warmup_epochs));
    set("warmup_momentum", json!(s.warmup_momentum));
    set("warmup_bias_lr", json!(s.warmup_bias_lr));
    set("nbs", json!(s.nbs));
    // Augmentation
    for (k, v) in [
        ("hsv_h", a.hsv_h),
        ("hsv_s", a.hsv_s),
        ("hsv_v", a.hsv_v),
        ("degrees", a.degrees),
        ("translate", a.translate),
        ("scale", a.scale),
        ("shear", a.shear),
        ("perspective", a.perspective),
        ("flipud", a.flipud),
        ("fliplr", a.fliplr),
        ("bgr", a.bgr),
        ("mosaic", a.mosaic),
        ("mixup", a.mixup),
        ("cutmix", a.cutmix),
        ("copy_paste", a.copy_paste),
        ("erasing", a.erasing),
    ] {
        set(k, json!(v));
    }
    set("close_mosaic", json!(a.close_mosaic));
    set("auto_augment", match a.auto_augment {
        AutoAugment::RandAugment => json!("randaugment"),
        AutoAugment::AutoAugment => json!("autoaugment"),
        AutoAugment::AugMix => json!("augmix"),
        AutoAugment::Off => Value::Null,
    });
    // Loss
    set("box", json!(l.box_gain));
    set("cls", json!(l.cls));
    set("dfl", json!(l.dfl));
    set("pose", json!(l.pose));
    set("kobj", json!(l.kobj));
    set("overlap_mask", json!(l.overlap_mask));
    set("mask_ratio", json!(l.mask_ratio));
    // Run
    set("device", json!(device));
    set("workers", json!(r.workers));
    set("amp", json!(r.amp));
    set("cache", match r.cache {
        Cache::Off => json!(false),
        Cache::Ram => json!("ram"),
        Cache::Disk => json!("disk"),
    });
    set("seed", json!(r.seed));
    set("deterministic", json!(r.deterministic));
    set("save_period", json!(r.save_period.map_or(-1, i64::from)));
    if let Some(hours) = r.time {
        set("time", json!(hours));
    }
    set("plots", json!(r.plots));
    Value::Object(m)
}

/// Runs a prepared training in `env`: Ultralytics' finished lines go to
/// `console.log` and to `reporter` as messages, progress bars only to
/// `reporter`; epochs count on an "Epochs" stage. Keeps `train.toml` up to
/// date and returns it. Stops (killing Python) when cancelled.
pub fn run(env: &PyEnv, prepared: &PreparedTrain, reporter: &dyn Reporter) -> Result<TrainRecord, crate::Error> {
    let mut record = prepared.record.clone();
    let mut console = File::options().create(true).append(true).open(prepared.dir.join(CONSOLE_FILE))?;
    let epochs = reporter.start("Epochs", Unit::Steps);
    epochs.set_total(u64::from(record.epochs));
    // A continued run counts on from the epochs it had.
    let mut done_epochs = record.epochs_done;
    epochs.add(u64::from(done_epochs));
    let mut total_epochs = record.epochs;
    let mut best: Option<String> = None;
    let mut summary: Option<RunSummary> = None;
    let mut last_error: Option<String> = None;

    let cmd = env.service_command([std::ffi::OsStr::new("train"), prepared.config.as_os_str()])?;
    let result = pyenv::stream(cmd, epochs.as_ref(), |out| match out {
        Output::Line(line) => {
            let line = strip_ansi(&line);
            if let Some(event) = line.strip_prefix("@modeloid ").and_then(|j| serde_json::from_str::<Value>(j).ok()) {
                match event["event"].as_str() {
                    Some("start") => {
                        if let Some(total) = event["epochs"].as_u64() {
                            epochs.set_total(total);
                            total_epochs = total as u32;
                        }
                        // Saved now, so pages show it while the run trains.
                        if let Some(name) = event["device_name"].as_str().filter(|n| !n.is_empty()) {
                            record.device_name = Some(name.to_string());
                            let _ = save_record(&prepared.dir, &record);
                        }
                    }
                    Some("epoch") => {
                        // Ultralytics also reports its final validation as an epoch end.
                        let epoch = (event["epoch"].as_u64().unwrap_or(0) as u32).min(total_epochs);
                        if epoch > done_epochs {
                            epochs.add(u64::from(epoch - done_epochs));
                            done_epochs = epoch;
                        }
                    }
                    Some("done") => {
                        best = event["best"].as_str().filter(|b| !b.is_empty()).map(str::to_string);
                        summary = serde_json::from_value::<RunSummary>(event["summary"].clone()).ok().filter(|s| !s.is_empty());
                    }
                    Some("error") => last_error = event["error"].as_str().map(str::to_string),
                    _ => {}
                }
                return;
            }
            let _ = writeln!(console, "{line}");
            if !line.trim().is_empty() {
                reporter.message(line.trim_end());
            }
        }
        Output::Progress(line) => {
            let line = strip_ansi(&line);
            if !line.trim().is_empty() {
                reporter.message(line.trim_end());
            }
        }
    });
    epochs.finish();

    record.epochs_done = done_epochs;
    record.finished = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    record.best = best.or_else(|| {
        let path = prepared.dir.join(RUNS_NAME).join("weights").join("best.pt");
        path.is_file().then(|| path.display().to_string())
    });
    record.summary = summary;
    record.status = match &result {
        Ok(()) => TrainStatus::Done,
        Err(e) if e.downcast_ref::<crate::Cancelled>().is_some() => TrainStatus::Stopped,
        Err(_) => TrainStatus::Failed,
    };
    if let Err(e) = &result {
        record.error = Some(last_error.clone().unwrap_or_else(|| e.to_string()));
        let _ = writeln!(console, "\n[modeloid] {}", record.error.as_deref().unwrap_or_default());
    }
    save_record(&prepared.dir, &record)?;
    match result {
        Ok(()) => Ok(record),
        Err(e) if record.status == TrainStatus::Stopped => Err(e),
        Err(_) => Err(record.error.clone().unwrap_or_default().into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_parameters_to_ultralytics() {
        let mut p = UltralyticsParams::default();
        p.training.batch = Batch::Auto;
        p.run.save_period = None;
        let args = ultralytics_args(&p, "0", Path::new("/d/data.yaml"), Path::new("/w/training/trains/m"));
        assert_eq!(args["batch"], json!(-1));
        assert_eq!(args["save_period"], json!(-1));
        assert_eq!(args["box"], json!(7.5));
        assert_eq!(args["cache"], json!(false));
        assert_eq!(args["name"], json!("runs"));
        assert_eq!(args["device"], json!("0"));
        assert!(args.get("freeze").is_none() && args.get("time").is_none());
    }

    #[test]
    fn device_follows_settings_for_auto() {
        let mut set = ParameterSet::new("s");
        assert_eq!(resolve_device(&set, TrainDevice::Cpu), "cpu");
        assert_eq!(resolve_device(&set, TrainDevice::Gpu), "0");
        let FrameworkParams::Ultralytics(p) = &mut set.params;
        p.run.device = "cpu".into();
        assert_eq!(resolve_device(&set, TrainDevice::Gpu), "cpu");
    }

    #[test]
    fn lists_runs_by_start() {
        let dir = std::env::temp_dir().join(format!("modeloid-train-records-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let ws = Workspace::new("ws".into(), dir.clone());
        ws.make_to_dir().unwrap();
        for (name, started) in [("late", "2026-10-02T00:00:00Z"), ("early", "2026-10-01T00:00:00Z")] {
            let run = ws.train_dir(name);
            fs::create_dir_all(&run).unwrap();
            let record = TrainRecord { name: name.into(), started: started.into(), status: TrainStatus::Done, ..Default::default() };
            save_record(&run, &record).unwrap();
        }
        fs::create_dir_all(ws.train_dir("broken")).unwrap();
        let (records, warnings) = ws.train_records().unwrap();
        assert_eq!(records.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["early", "late"]);
        assert_eq!(warnings.len(), 1, "a run folder without train.toml");

        ws.delete_train("late").unwrap();
        assert!(!ws.train_dir("late").exists());
        assert!(ws.delete_train("late").is_err());
        assert!(ws.delete_train("../datasets").is_err(), "only run folders");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn continues_stopped_runs() {
        let dir = std::env::temp_dir().join(format!("modeloid-train-resume-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let ws = Workspace::new("ws".into(), dir.clone());
        ws.make_to_dir().unwrap();
        let run = ws.train_dir("cars");
        fs::create_dir_all(&run).unwrap();
        let mut set = ParameterSet::new("base");
        fs::write(run.join(PARAMETERS_FILE), toml::to_string(&set).unwrap()).unwrap();
        let mut record = TrainRecord { name: "cars".into(), model: "yolo26n.pt".into(), epochs: 100, epochs_done: 12, status: TrainStatus::Done, ..Default::default() };
        save_record(&run, &record).unwrap();
        let problems = |ws: &Workspace| ws.resume_preview("cars", TrainDevice::Cpu).unwrap().problems;

        assert!(problems(&ws)[0].contains("is done"), "only stopped runs");
        record.status = TrainStatus::Stopped;
        save_record(&run, &record).unwrap();
        assert!(problems(&ws)[0].contains("nothing to continue"), "no last.pt yet");
        fs::create_dir_all(last_weights(&run).parent().unwrap()).unwrap();
        fs::write(last_weights(&run), b"ckpt").unwrap();
        let FrameworkParams::Ultralytics(p) = &mut set.params;
        p.run.resume = false;
        fs::write(run.join(PARAMETERS_FILE), toml::to_string(&set).unwrap()).unwrap();
        assert!(problems(&ws)[0].contains("Resume is off"));
        let FrameworkParams::Ultralytics(p) = &mut set.params;
        p.run.resume = true;
        fs::write(run.join(PARAMETERS_FILE), toml::to_string(&set).unwrap()).unwrap();
        let preview = ws.resume_preview("cars", TrainDevice::Cpu).unwrap();
        assert!(preview.can_run(), "{:?}", preview.problems);
        assert!(preview.notes[0].contains("epoch 12 of 100"));
        // The same resumable folder by a path: refused, so no other folder's last.pt is ever loaded.
        for name in ["../trains/cars", "..\\trains\\cars", ".."] {
            assert!(ws.resume_preview(name, TrainDevice::Cpu).is_err(), "{name}");
            assert!(ws.prepare_resume(name, TrainDevice::Cpu).is_err(), "{name}");
            assert!(ws.train_record(name).is_err(), "{name}");
            assert!(ws.train_log(name, 0, 100).is_err(), "{name}");
        }

        let prepared = ws.prepare_resume("cars", TrainDevice::Cpu).unwrap();
        let config: Value = serde_json::from_slice(&fs::read(&prepared.config).unwrap()).unwrap();
        assert_eq!(config["resume"], json!(true));
        assert!(config["model"].as_str().unwrap().ends_with("last.pt"));
        assert_eq!(config["args"]["device"], json!("cpu"));
        assert_eq!(ws.train_record("cars").unwrap().status, TrainStatus::Running);
        assert_eq!(prepared.record.epochs_done, 12, "counts on from where it stopped");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_yolo_layouts() {
        let dir = std::env::temp_dir().join(format!("modeloid-train-layout-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("train/images")).unwrap();
        fs::create_dir_all(dir.join("valid/images")).unwrap();
        assert_eq!(yolo_layout(&dir), Some(("train/images".into(), Some("valid/images".into()), None)));
        fs::remove_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("images/train")).unwrap();
        assert_eq!(yolo_layout(&dir), Some(("images/train".into(), None, None)));
        let _ = fs::remove_dir_all(&dir);
    }
}
