//! Tuning hyperparameters (`JobKind::Tune`): preview, start, the run's
//! record and console output. Runs live in `training/tuning/<name>/` (see
//! the core's `core::tune`); the best values go into a parameter set.

use dioxus::prelude::*;

pub use modeloid_types::tune::*;

use crate::api::jobs::Job;
use crate::api::train::{TrainLog, TrainPreview};

/// What tuning `plan` would do, and what's in the way. `name_taken` is about
/// the tuning run's name.
#[post("/api/tune/preview")]
pub async fn tune_preview(plan: TunePlan) -> Result<TrainPreview, ServerFnError> {
    backend::preview(plan).await.map_err(ServerFnError::new)
}

/// Starts tuning (a `Tune` job with one item, named after the run).
#[post("/api/tune/start")]
pub async fn start_tune(plan: TunePlan) -> Result<Job, ServerFnError> {
    crate::api::blocking(move || backend::start(plan)).await
}

/// Tuning run `name`'s record: trials, scores, the best and where it was saved.
#[post("/api/tune/record")]
pub async fn tune_record(name: String) -> Result<TuneRecord, ServerFnError> {
    backend::record(name).await.map_err(ServerFnError::new)
}

/// Names of the workspace's tuning runs (taken names).
#[post("/api/tune/names")]
pub async fn tune_names() -> Result<Vec<String>, ServerFnError> {
    backend::names().await.map_err(ServerFnError::new)
}

/// Console output of tuning run `name` from byte `offset`.
#[post("/api/tune/log")]
pub async fn tune_log(name: String, offset: u64) -> Result<TrainLog, ServerFnError> {
    backend::log(name, offset).await.map_err(ServerFnError::new)
}

#[cfg(feature = "server")]
mod backend {
    use modeloid::core::pyenv::PyEnv;
    use modeloid::core::tune;

    use super::{TrainLog, TrainPreview, TunePlan, TuneRecord};
    use crate::api::jobs::{self, Job, JobKind};
    use crate::api::train::backend::{device_choice, open};

    const LOG_CHUNK: usize = 256 * 1024;

    pub async fn preview(plan: TunePlan) -> Result<TrainPreview, String> {
        tokio::task::spawn_blocking(move || {
            let ws = open()?;
            let env = PyEnv::locate().map_err(|e| e.to_string())?;
            let mut preview = ws.tune_preview(&plan, device_choice(&env)).map_err(|e| e.to_string())?;
            if !env.status().usable() {
                preview.problems.insert(0, "The training libraries (PyTorch, Ultralytics, Optuna) aren't installed yet.".into());
            }
            Ok(preview)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub fn start(plan: TunePlan) -> Result<Job, String> {
        let ws = open()?;
        let env = PyEnv::locate().map_err(|e| e.to_string())?;
        if !env.status().usable() {
            return Err("The training libraries (PyTorch, Ultralytics, Optuna) aren't installed yet. See Settings → Python environment.".into());
        }
        let choice = device_choice(&env);
        // Checked now, so a mistake shows at once instead of as a failed job.
        let preview = ws.tune_preview(&plan, choice).map_err(|e| e.to_string())?;
        if preview.name_taken {
            return Err(tune::tune_name_taken_message(&plan.name));
        }
        if let Some(problem) = preview.problems.first() {
            return Err(problem.clone());
        }
        let name = plan.name.clone();
        jobs::start(JobKind::Tune, vec![name], jobs::Touches::Datasets(vec![plan.dataset]), move |ctx| {
            let reporter = ctx.reporter();
            let prepared = ws.prepare_tune(&plan, choice, reporter.as_ref()).map_err(|e| e.to_string())?;
            let record = tune::run(&ws, &env, &prepared, reporter.as_ref()).map_err(|e| e.to_string())?;
            Ok(Some(match (&record.best, &record.saved) {
                (Some(best), Some(set)) => format!("Best: trial {} ({:.4}), saved into parameter set {set}", best.number, best.value.unwrap_or_default()),
                _ => format!("Results in {}", prepared.dir.display()),
            }))
        })
    }

    pub async fn record(name: String) -> Result<TuneRecord, String> {
        tokio::task::spawn_blocking(move || open()?.tune_record(&name).map_err(|e| e.to_string())).await.map_err(|e| e.to_string())?
    }

    pub async fn names() -> Result<Vec<String>, String> {
        tokio::task::spawn_blocking(move || Ok(open()?.tune_names())).await.map_err(|e| e.to_string())?
    }

    pub async fn log(name: String, offset: u64) -> Result<TrainLog, String> {
        tokio::task::spawn_blocking(move || open()?.tune_log(&name, offset, LOG_CHUNK).map_err(|e| e.to_string())).await.map_err(|e| e.to_string())?
    }
}
