//! Training a model (`JobKind::Train`): preview, start, and the run's
//! console output. Runs live in `training/trains/<name>/` (see the core's
//! `core::train`).

use dioxus::prelude::*;

pub use modeloid_types::train::{TrainLog, TrainPreview};

use crate::api::jobs::Job;

/// What training `dataset` with parameter set `parameters` as `name` would
/// do, and what's in the way.
#[post("/api/train/preview")]
pub async fn train_preview(dataset: u32, parameters: String, name: String) -> Result<TrainPreview, ServerFnError> {
    backend::preview(dataset, parameters, name).await.map_err(ServerFnError::new)
}

/// Starts training (a `Train` job with one item, named after the model).
#[post("/api/train/start")]
pub async fn start_training(dataset: u32, parameters: String, name: String) -> Result<Job, ServerFnError> {
    backend::start(dataset, parameters, name).map_err(ServerFnError::new)
}

/// Console output of run `name` from byte `offset`.
#[post("/api/train/log")]
pub async fn train_log(name: String, offset: u64) -> Result<TrainLog, ServerFnError> {
    backend::log(name, offset).await.map_err(ServerFnError::new)
}

#[cfg(feature = "server")]
mod backend {
    use modeloid::core::pyenv::{PyEnv, TrainDevice};
    use modeloid::core::train;
    use modeloid::{ParameterSet, Settings, Workspace};

    use super::{TrainLog, TrainPreview};
    use crate::api::jobs::{self, Job, JobKind};
    use crate::api::{activity, workspace};

    /// Most console text sent in one answer.
    const LOG_CHUNK: usize = 256 * 1024;

    fn open() -> Result<Workspace, String> {
        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))
    }

    fn parameter_set(ws: &Workspace, name: &str) -> Result<ParameterSet, String> {
        let (sets, _) = ws.parameter_sets().map_err(|e| e.to_string())?;
        sets.into_iter().find(|s| s.name == name).ok_or_else(|| format!("There's no parameter set named '{name}' any more."))
    }

    /// The device "auto" means: the one chosen in Settings, if its libraries are there.
    fn device_choice(env: &PyEnv) -> TrainDevice {
        let overview = env.overview(&Settings::load_default());
        let device = overview.device();
        if overview.installed(device) { device } else { TrainDevice::Cpu }
    }

    pub async fn preview(dataset: u32, parameters: String, name: String) -> Result<TrainPreview, String> {
        activity::track("train-preview", "Checking the training setup", move |_| {
            let ws = open()?;
            let set = parameter_set(&ws, &parameters)?;
            let env = PyEnv::locate().map_err(|e| e.to_string())?;
            let mut preview = ws.train_preview(dataset, &set, &name, device_choice(&env)).map_err(|e| e.to_string())?;
            if !env.status().usable() {
                preview.problems.insert(0, "The training libraries (PyTorch, Ultralytics) aren't installed.".into());
            }
            Ok(preview)
        })
        .await
    }

    pub fn start(dataset: u32, parameters: String, name: String) -> Result<Job, String> {
        let ws = open()?;
        let set = parameter_set(&ws, &parameters)?;
        let env = PyEnv::locate().map_err(|e| e.to_string())?;
        if !env.status().usable() {
            return Err("The training libraries (PyTorch, Ultralytics) aren't installed. Install them in Settings → Python environment.".into());
        }
        let choice = device_choice(&env);
        // Checked now, so a mistake shows at once instead of as a failed job.
        let preview = ws.train_preview(dataset, &set, &name, choice).map_err(|e| e.to_string())?;
        if let Some(problem) = preview.problems.first() {
            return Err(problem.clone());
        }
        jobs::start(JobKind::Train, vec![name.clone()], jobs::Touches::Datasets(vec![dataset]), move |ctx| {
            let reporter = ctx.reporter();
            let prepared = ws.prepare_train(dataset, &set, &name, choice, reporter.as_ref()).map_err(|e| e.to_string())?;
            let record = train::run(&env, &prepared, reporter.as_ref()).map_err(|e| e.to_string())?;
            Ok(Some(match record.best {
                Some(best) => format!("Best weights: {best}"),
                None => format!("Results in {}", prepared.dir.display()),
            }))
        })
    }

    pub async fn log(name: String, offset: u64) -> Result<TrainLog, String> {
        tokio::task::spawn_blocking(move || open()?.train_log(&name, offset, LOG_CHUNK).map_err(|e| e.to_string()))
            .await
            .map_err(|e| e.to_string())?
    }
}
