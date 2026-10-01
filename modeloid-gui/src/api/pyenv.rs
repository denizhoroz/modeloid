//! The Python environment training runs in (see the core's `core::pyenv`):
//! its status, creating it, and installing or removing the training
//! libraries (jobs). The environment itself is never deleted from the app.

use dioxus::prelude::*;

pub use modeloid_types::pyenv::*;

use crate::api::jobs::Job;

/// Status, NVIDIA GPU and folder, for the setup screen.
#[post("/api/python")]
pub async fn python_env_overview() -> Result<EnvOverview, ServerFnError> {
    backend::overview().await.map_err(ServerFnError::new)
}

/// Creates the environment with only Python in it when there's none yet (the
/// app calls this on launch), as a `PythonSetup` job. `None` when there was
/// nothing to do. The training libraries are installed later, from Settings.
#[post("/api/python/prepare")]
pub async fn prepare_python_env() -> Result<Option<Job>, ServerFnError> {
    backend::prepare().map_err(ServerFnError::new)
}

/// Installs the training libraries with `variant`'s PyTorch build (replacing
/// the other build; also repairs or updates them), as a `PythonSetup` job.
/// Training then runs on that build's device.
#[post("/api/python/setup")]
pub async fn start_python_setup(variant: TorchVariant) -> Result<Job, ServerFnError> {
    backend::setup(variant).map_err(ServerFnError::new)
}

/// Uninstalls PyTorch, torchvision and Ultralytics (whichever build), as a
/// `PythonSetup` job. The environment and its Python stay.
#[post("/api/python/remove-libraries")]
pub async fn remove_python_libraries() -> Result<Job, ServerFnError> {
    backend::remove_libraries().map_err(ServerFnError::new)
}

/// Chooses what training runs on. No reinstall: the CUDA build runs on the
/// CPU too (choosing the GPU without the CUDA build needs an install first).
#[post("/api/python/device")]
pub async fn set_train_device(device: TrainDevice) -> Result<(), ServerFnError> {
    backend::update_settings(move |s| s.set_train_device(device)).map_err(ServerFnError::new)
}

#[cfg(feature = "server")]
mod backend {
    use std::path::PathBuf;

    use modeloid::core::pyenv::{detect_gpu, PyEnv};
    use modeloid::Settings;

    use super::{EnvOverview, EnvStatus, TorchVariant};
    use crate::api::jobs::{self, ItemCtx, Job, JobKind};

    fn locate() -> Result<PyEnv, String> {
        PyEnv::locate().map_err(|e| e.to_string())
    }

    /// The uv to run, downloaded on first use.
    fn uv(env: &PyEnv, ctx: &ItemCtx) -> Result<PathBuf, String> {
        match env.find_uv() {
            Some(uv) => Ok(uv),
            None => ctx.block_on(env.fetch_uv(ctx.reporter().as_ref())).map_err(|e| format!("Couldn't download uv: {e}")),
        }
    }

    fn setup_running() -> bool {
        jobs::latest_all().iter().any(|j| j.kind == JobKind::PythonSetup && !j.finished)
    }

    pub async fn overview() -> Result<EnvOverview, String> {
        tokio::task::spawn_blocking(|| Ok(locate()?.overview(&Settings::load_default()))).await.map_err(|e| e.to_string())?
    }

    pub fn prepare() -> Result<Option<Job>, String> {
        let env = locate()?;
        if setup_running() || !matches!(env.status(), EnvStatus::Missing | EnvStatus::Broken(_)) {
            return Ok(None);
        }
        jobs::start(JobKind::PythonSetup, vec!["Python environment".into()], jobs::Touches::Nothing, move |ctx| {
            let version = env.create(&uv(&env, ctx)?, ctx.reporter().as_ref()).map_err(|e| e.to_string())?;
            Ok(Some(format!("Python {version}")))
        })
        .map(Some)
    }

    pub fn setup(variant: TorchVariant) -> Result<Job, String> {
        let env = locate()?;
        if variant == TorchVariant::Cuda && !detect_gpu().is_some_and(|g| g.cuda_problem().is_none()) {
            return Err("No CUDA compatible device found.".into());
        }
        let label = format!("Installing {} libraries", variant.device().label());
        jobs::start(JobKind::PythonSetup, vec![label], jobs::Touches::Nothing, move |ctx| {
            let info = env.install(variant, &uv(&env, ctx)?, ctx.reporter().as_ref()).map_err(|e| e.to_string())?;
            update_settings(|s| s.set_train_device(variant.device()))?;
            let device = if info.cuda_available || info.mps_available { info.device_name.clone() } else { "CPU".into() };
            Ok(Some(format!("Python {} · torch {} · {device}", info.python, info.torch)))
        })
    }

    pub fn remove_libraries() -> Result<Job, String> {
        let env = locate()?;
        if !env.status().has_python() {
            return Err("There are no training libraries to remove.".into());
        }
        jobs::start(JobKind::PythonSetup, vec!["Removing training libraries".into()], jobs::Touches::Nothing, move |ctx| {
            env.remove_libraries(&uv(&env, ctx)?, ctx.reporter().as_ref()).map_err(|e| e.to_string())?;
            Ok(Some("PyTorch, torchvision and Ultralytics removed".into()))
        })
    }

    pub fn update_settings(change: impl FnOnce(&mut Settings)) -> Result<(), String> {
        let path = Settings::default_path().ok_or("Couldn't find a config folder on this computer.")?;
        let mut settings = Settings::load(&path).map_err(|e| format!("Couldn't read the settings: {e}"))?;
        change(&mut settings);
        settings.save(&path).map_err(|e| format!("Couldn't save the settings: {e}"))
    }
}
