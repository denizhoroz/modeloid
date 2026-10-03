//! Tuning hyperparameters with Optuna in the Python environment.
//!
//! A tuning run lives in `training/tuning/<name>/`:
//!
//! ```text
//! tune.toml         TuneRecord: dataset, parameter set, trials and their scores, the best
//! parameters.toml   the parameter set it started from
//! data.yaml         the dataset for Ultralytics (or dataset/data.yaml when converted)
//! config.json       what main.py tune gets: model, shared arguments, trials, search space
//! console.log       Ultralytics' and Optuna's output, finished lines only
//! trials/trial-N/   each trial's Ultralytics run (no weights are kept)
//! ```
//!
//! When it ends, the best trial's values are written into the parameter set:
//! the one it started from, or a new one.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::core::progress::{Reporter, Unit};
use crate::core::pyenv::{self, Output, PyEnv, TrainDevice};
use crate::core::train::{absolute, device_label, read_log, resolve_device, strip_ansi, ultra, ultralytics_args, TrainLog, TrainPreview, TrainStatus};
use crate::models::parameters::{Family, FrameworkParams, ParameterSet};
use crate::models::workspace::Workspace;

pub use modeloid_types::tune::*;

fn plural(n: u32, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

const RECORD_FILE: &str = "tune.toml";
const CONSOLE_FILE: &str = "console.log";
const TRIALS_DIR: &str = "trials";

/// A tuning run ready to start (folder written).
#[derive(Debug, Clone)]
pub struct PreparedTune {
    pub dir: PathBuf,
    pub config: PathBuf,
    pub record: TuneRecord,
    /// The settings it started from; the best values are written into a copy.
    pub base: ParameterSet,
}

impl Workspace {
    /// `training/tuning/<name>`, absolute.
    pub fn tune_dir(&self, name: &str) -> PathBuf {
        absolute(&self.tunings_dir().join(name))
    }

    /// The parameter set a plan starts from, and whether it's a new one.
    fn tune_base(&self, plan: &TunePlan) -> Result<(ParameterSet, bool), crate::Error> {
        match (&plan.parameters, &plan.new_set) {
            (Some(name), _) => {
                let (sets, _) = self.parameter_sets()?;
                let set = sets.into_iter().find(|s| &s.name == name).ok_or_else(|| format!("There's no parameter set named '{name}' any more."))?;
                Ok((set, false))
            }
            (None, Some(set)) => Ok((set.clone(), true)),
            (None, None) => Err("Choose a parameter set, or create a new one.".into()),
        }
    }

    /// What tuning `plan` would do and what's in the way: the training checks
    /// (dataset, task, model) for its parameter set, plus the plan's own.
    /// `name_taken` is about the tuning run's name.
    pub fn tune_preview(&self, plan: &TunePlan, choice: TrainDevice) -> Result<TrainPreview, crate::Error> {
        let (base, new) = self.tune_base(plan)?;
        let mut preview = self.run_plan(plan.dataset, &base, &plan.name, &self.tune_dir(&plan.name), choice)?.preview;
        // Training's own epochs don't matter here; trials use the plan's.
        preview.problems.retain(|p| !p.starts_with("Parameters: Epochs"));
        preview.problems.extend(plan.problems());
        if new {
            let (sets, _) = self.parameter_sets()?;
            if sets.iter().any(|s| s.name == base.name) {
                preview.problems.push(format!("A parameter set named '{}' exists already. Choose another name for the new one.", base.name));
            }
        }
        let task = ultra(&base).model.task;
        if plan.space.iter().any(|p| p.enabled) && plan.searched(task).is_empty() {
            preview.problems.push(format!("None of the chosen hyperparameters applies to {} models.", task.label().to_lowercase()));
        }
        let images = if plan.fraction < 1.0 { format!("{:.0}% of the training images", plan.fraction * 100.0) } else { "all training images".to_string() };
        preview.notes.push(format!(
            "{} × {} on {images}: about {} epochs of training in all. Trials keep no weights.",
            plural(plan.trials, "trial"),
            plural(plan.epochs, "epoch"),
            u64::from(plan.trials) * u64::from(plan.epochs)
        ));
        let target = if new { format!("the new set '{}'", base.name) } else { format!("'{}'", base.name) };
        preview.notes.push(format!("The best trial's values are saved into {target}; its other settings stay as they are."));
        Ok(preview)
    }

    /// Creates the tuning folder: record, base parameters, data and `config.json`.
    pub fn prepare_tune(&self, plan: &TunePlan, choice: TrainDevice, reporter: &dyn Reporter) -> Result<PreparedTune, crate::Error> {
        let preview = self.tune_preview(plan, choice)?;
        if preview.name_taken {
            return Err(tune_name_taken_message(&plan.name).into());
        }
        if let Some(problem) = preview.problems.first() {
            return Err(problem.clone().into());
        }
        let (base, new) = self.tune_base(plan)?;
        let run = self.run_plan(plan.dataset, &base, &plan.name, &self.tune_dir(&plan.name), choice)?;
        let p = ultra(&base);
        let dir = self.tune_dir(&plan.name);
        fs::create_dir_all(&dir)?;
        let data = self.write_data(plan.dataset, run.data.expect("no problems means data"), &dir, reporter)?;

        let device = resolve_device(&base, choice);
        let mut args = ultralytics_args(p, &device, &data, &dir);
        let map = args.as_object_mut().expect("arguments are an object");
        // Every trial: the plan's epochs and data share, scored on validation, nothing kept.
        map.insert("project".into(), json!(dir.join(TRIALS_DIR).display().to_string()));
        map.remove("name");
        map.remove("time");
        map.insert("epochs".into(), json!(plan.epochs));
        map.insert("fraction".into(), json!(plan.fraction));
        map.insert("val".into(), json!(true));
        map.insert("save".into(), json!(false));
        map.insert("save_period".into(), json!(-1));
        map.insert("plots".into(), json!(false));
        map.insert("patience".into(), json!(plan.epochs));

        let task = p.model.task;
        let searched = plan.searched(task);
        let space: Vec<Value> = searched.iter().map(|t| space_json(t)).collect();
        let config = dir.join("config.json");
        fs::write(
            &config,
            serde_json::to_vec_pretty(&json!({
                "model": p.model_name(),
                "task": task.id(),
                "rtdetr": p.model.family == Family::RtDetr,
                "args": args,
                "trials": plan.trials,
                "seed": p.run.seed,
                "space": space,
            }))?,
        )?;
        fs::write(dir.join("parameters.toml"), toml::to_string(&base)?)?;

        let record = TuneRecord {
            name: plan.name.clone(),
            dataset_id: plan.dataset,
            dataset_name: run.dataset_name,
            parameters: base.name.clone(),
            new_set: new,
            model: p.model_name(),
            task: task.id().to_string(),
            device: device_label(&device),
            trials: plan.trials,
            epochs: plan.epochs,
            fraction: plan.fraction,
            searched: searched.iter().map(|t| t.key.clone()).collect(),
            started: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            ..Default::default()
        };
        save_record(&dir, &record)?;
        Ok(PreparedTune { dir, config, record, base })
    }

    /// Names of the tuning runs (folders in `training/tuning`).
    pub fn tune_names(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.tunings_dir()) else { return Vec::new() };
        let mut names: Vec<String> = entries.filter_map(Result::ok).filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    /// The record of tuning run `name`.
    pub fn tune_record(&self, name: &str) -> Result<TuneRecord, crate::Error> {
        crate::validate_dataset_name(name)?;
        let text = fs::read_to_string(self.tune_dir(name).join(RECORD_FILE)).map_err(|e| format!("no tuning run named '{name}': {e}"))?;
        Ok(toml::from_str(&text)?)
    }

    /// Console output of tuning run `name` from byte `offset`.
    pub fn tune_log(&self, name: &str, offset: u64, max: usize) -> Result<TrainLog, crate::Error> {
        crate::validate_dataset_name(name)?;
        read_log(&self.tune_dir(name).join(CONSOLE_FILE), offset, max)
    }

    /// Writes the best trial's values into a copy of `base` and saves it: over
    /// the set it came from, or as the new set. Returns its name.
    pub fn save_tuned(&self, base: &ParameterSet, new: bool, best: &TrialResult) -> Result<String, crate::Error> {
        let mut set = base.clone();
        let FrameworkParams::Ultralytics(p) = &mut set.params;
        for (key, value) in &best.params {
            set_hyper_value(p, key, value)?;
        }
        let replacing = (!new).then_some(base.name.as_str());
        let saved = self.save_parameter_set(&set, replacing)?;
        Ok(saved.name)
    }
}

/// One searched row as `main.py tune` reads it.
fn space_json(param: &TuneParam) -> Value {
    let integer = hyperparameter(&param.key).is_some_and(|h| h.integer);
    match &param.space {
        TuneSpace::Range { low, high, log } => json!({ "key": param.key, "kind": "range", "low": low, "high": high, "log": log, "int": integer }),
        TuneSpace::Candidates(values) => {
            let values: Vec<Value> = values
                .iter()
                .map(|v| match v {
                    TuneValue::Number(n) if integer => json!(n.round() as i64),
                    TuneValue::Number(n) => json!(n),
                    TuneValue::Text(t) => json!(t),
                })
                .collect();
            json!({ "key": param.key, "kind": "candidates", "values": values })
        }
    }
}

fn save_record(dir: &Path, record: &TuneRecord) -> Result<(), crate::Error> {
    fs::write(dir.join(RECORD_FILE), toml::to_string(record)?)?;
    Ok(())
}

/// A trial's values from a `@modeloid` event.
fn params_of(event: &Value) -> BTreeMap<String, TuneValue> {
    event["params"]
        .as_object()
        .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), serde_json::from_value::<TuneValue>(v.clone()).ok()?))).collect())
        .unwrap_or_default()
}

/// The finished trial with the highest score (the earliest of equal ones).
pub fn best_trial(results: &[TrialResult]) -> Option<&TrialResult> {
    results.iter().filter(|r| r.value.is_some_and(f64::is_finite)).fold(None, |best: Option<&TrialResult>, r| match best {
        Some(b) if b.value >= r.value => Some(b),
        _ => Some(r),
    })
}

/// Runs a prepared tuning in `env`: output to `console.log` and `reporter`,
/// a "Trials" stage, `tune.toml` saved after every trial. When it ends (or is
/// stopped) with a finished trial, the best values are saved into the
/// parameter set (`ws.save_tuned`). Stops (killing Python) when cancelled.
pub fn run(ws: &Workspace, env: &PyEnv, prepared: &PreparedTune, reporter: &dyn Reporter) -> Result<TuneRecord, crate::Error> {
    let mut record = prepared.record.clone();
    let mut console = File::options().create(true).append(true).open(prepared.dir.join(CONSOLE_FILE))?;
    let trials = reporter.start("Trials", Unit::Steps);
    trials.set_total(u64::from(record.trials));
    let mut last_error: Option<String> = None;

    let cmd = env.service_command([std::ffi::OsStr::new("tune"), prepared.config.as_os_str()])?;
    let result = pyenv::stream(cmd, trials.as_ref(), |out| match out {
        Output::Line(line) => {
            let line = strip_ansi(&line);
            if let Some(event) = line.strip_prefix("@modeloid ").and_then(|j| serde_json::from_str::<Value>(j).ok()) {
                match event["event"].as_str() {
                    Some("trial-start") => {
                        let n = event["number"].as_u64().unwrap_or(0);
                        let _ = writeln!(console, "\n[modeloid] Trial {n} of {}", record.trials);
                        reporter.message(&format!("Trial {n} of {}", record.trials));
                    }
                    Some("trial") => {
                        let failed = event["state"].as_str() != Some("done");
                        let result = TrialResult {
                            number: event["number"].as_u64().unwrap_or(0) as u32,
                            value: event["value"].as_f64().filter(|v| v.is_finite()),
                            failed,
                            params: params_of(&event),
                            error: event["error"].as_str().map(str::to_string),
                        };
                        let _ = writeln!(
                            console,
                            "[modeloid] Trial {} {}",
                            result.number,
                            match (result.failed, result.value) {
                                (true, _) => format!("failed: {}", result.error.as_deref().unwrap_or("error")),
                                (false, Some(v)) => format!("scored {v:.4}"),
                                (false, None) => "finished without a score".into(),
                            }
                        );
                        record.results.push(result);
                        record.best = best_trial(&record.results).cloned();
                        trials.add(1);
                        let _ = save_record(&prepared.dir, &record);
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
    trials.finish();

    record.finished = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    record.best = best_trial(&record.results).cloned();
    record.status = match &result {
        Ok(()) => TrainStatus::Done,
        Err(e) if e.downcast_ref::<crate::Cancelled>().is_some() => TrainStatus::Stopped,
        Err(_) => TrainStatus::Failed,
    };
    if let Err(e) = &result {
        record.error = Some(last_error.clone().unwrap_or_else(|| e.to_string()));
    }
    // The best so far is kept even when the run was stopped or a later step failed.
    if let Some(best) = record.best.clone() {
        match ws.save_tuned(&prepared.base, record.new_set, &best) {
            Ok(name) => {
                record.saved = Some(name.clone());
                let _ = writeln!(console, "\n[modeloid] Best: trial {} ({:.4}). Saved into parameter set '{name}'.", best.number, best.value.unwrap_or_default());
            }
            Err(e) => {
                let message = format!("The best values couldn't be saved: {e}");
                let _ = writeln!(console, "\n[modeloid] {message}");
                record.error.get_or_insert(message);
                if record.status == TrainStatus::Done {
                    record.status = TrainStatus::Failed;
                }
            }
        }
    } else if record.error.is_none() {
        record.error = Some("No trial finished, so nothing was saved.".into());
        record.status = TrainStatus::Failed;
    }
    if let Some(error) = &record.error {
        let _ = writeln!(console, "\n[modeloid] {error}");
    }
    save_record(&prepared.dir, &record)?;
    match (&result, record.status) {
        (Err(e), TrainStatus::Stopped) => Err(e.to_string().into()),
        (_, TrainStatus::Done) => Ok(record),
        _ => Err(record.error.clone().unwrap_or_default().into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::parameters::Task;

    #[test]
    fn builds_the_search_space_for_python() {
        let lr = TuneParam { key: "lr0".into(), enabled: true, space: TuneSpace::Range { low: 1e-4, high: 0.1, log: true } };
        assert_eq!(space_json(&lr), json!({ "key": "lr0", "kind": "range", "low": 1e-4, "high": 0.1, "log": true, "int": false }));
        let batch = TuneParam { key: "batch".into(), enabled: true, space: TuneSpace::Candidates(vec![TuneValue::Number(16.0)]) };
        assert_eq!(space_json(&batch)["values"], json!([16]));
        let ev = json!({ "params": { "lr0": 0.01, "optimizer": "SGD" } });
        let p = params_of(&ev);
        assert_eq!(p["optimizer"], TuneValue::Text("SGD".into()));
        assert_eq!(p["lr0"], TuneValue::Number(0.01));
    }

    #[test]
    fn saves_the_best_trial_into_a_parameter_set() {
        let dir = std::env::temp_dir().join(format!("modeloid-tune-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let ws = Workspace::new("ws".into(), dir.clone());
        ws.make_to_dir().unwrap();
        assert!(ws.tunings_dir().is_dir(), "the workspace has training/tuning");

        let base = ParameterSet::new("base");
        ws.save_parameter_set(&base, None).unwrap();
        let results = vec![
            TrialResult { number: 1, value: Some(0.4), params: BTreeMap::from([("lr0".into(), TuneValue::Number(0.02))]), ..Default::default() },
            TrialResult { number: 2, failed: true, ..Default::default() },
            TrialResult {
                number: 3,
                value: Some(0.6),
                params: BTreeMap::from([("lr0".into(), TuneValue::Number(0.005)), ("optimizer".into(), TuneValue::Text("AdamW".into()))]),
                ..Default::default()
            },
        ];
        let best = best_trial(&results).unwrap();
        assert_eq!(best.number, 3);
        let tied = [TrialResult { number: 1, value: Some(0.0), ..Default::default() }, TrialResult { number: 2, value: Some(0.0), ..Default::default() }];
        assert_eq!(best_trial(&tied).unwrap().number, 1, "the earliest of equal scores");

        // Into the set it started from.
        assert_eq!(ws.save_tuned(&base, false, best).unwrap(), "base");
        let (sets, _) = ws.parameter_sets().unwrap();
        let saved = ultra(sets.iter().find(|s| s.name == "base").unwrap());
        assert_eq!(saved.training.lr0, 0.005);
        assert_eq!(saved.training.optimizer.id(), "AdamW");

        // As a new set; a taken name is refused.
        let mut fresh = ParameterSet::new("tuned");
        let FrameworkParams::Ultralytics(p) = &mut fresh.params;
        p.model.task = Task::Detect;
        assert_eq!(ws.save_tuned(&fresh, true, best).unwrap(), "tuned");
        assert!(ws.save_tuned(&fresh, true, best).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
