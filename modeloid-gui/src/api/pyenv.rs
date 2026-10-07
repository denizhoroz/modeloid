//! The Python environment training runs in (see the core's `core::pyenv`):
//! its status, setting it up, and switching the PyTorch build (jobs).
//! Ultralytics, Optuna and the rest are always installed and can't be
//! removed from the app; only torch and torchvision change between builds.

use dioxus::prelude::*;

pub use modeloid_types::pyenv::*;

use crate::api::jobs::Job;

/// Status, NVIDIA GPU and folder, for the setup screen.
#[post("/api/python")]
pub async fn python_env_overview() -> Result<EnvOverview, ServerFnError> {
    backend::overview().await.map_err(ServerFnError::new)
}

/// Sets the environment up when it needs it (the app calls this on launch):
/// Python and every library, with the CPU build of PyTorch; a half-installed
/// one is repaired, an outdated one updated with the build it has. A
/// `PythonSetup` job, or `None` when there was nothing to do.
#[post("/api/python/prepare")]
pub async fn prepare_python_env() -> Result<Option<Job>, ServerFnError> {
    crate::api::blocking(backend::prepare).await
}

/// Switches to `variant`'s PyTorch build (only torch and torchvision change;
/// also repairs), as a `PythonSetup` job. Training then runs on that build's
/// device. The GPU's "Remove" is this with the CPU build.
#[post("/api/python/setup")]
pub async fn start_python_setup(variant: TorchVariant) -> Result<Job, ServerFnError> {
    crate::api::blocking(move || backend::setup(variant)).await
}

/// Chooses what training runs on. No reinstall: the CUDA build runs on the
/// CPU too (choosing the GPU without the CUDA build needs an install first).
#[post("/api/python/device")]
pub async fn set_train_device(device: TrainDevice) -> Result<(), ServerFnError> {
    crate::api::blocking(move || backend::update_settings(move |s| s.set_train_device(device))).await
}

#[cfg(feature = "server")]
mod backend {
    use std::path::PathBuf;

    use modeloid::core::pyenv::{detect_gpu, PyEnv};
    use modeloid::Settings;

    use super::{EnvOverview, TorchVariant};
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
        if setup_running() {
            return Ok(None);
        }
        let Some(variant) = env.pending_install() else { return Ok(None) };
        start_install(env, variant, "Setting up Python".into()).map(Some)
    }

    pub fn setup(variant: TorchVariant) -> Result<Job, String> {
        let env = locate()?;
        if variant == TorchVariant::Cuda && !detect_gpu().is_some_and(|g| g.cuda_problem().is_none()) {
            return Err("No CUDA compatible device found.".into());
        }
        let label = match variant {
            TorchVariant::Cuda => "Installing GPU libraries",
            TorchVariant::Cpu => "Switching to CPU libraries",
        };
        start_install(env, variant, label.into())
    }

    fn start_install(env: PyEnv, variant: TorchVariant, label: String) -> Result<Job, String> {
        jobs::start(JobKind::PythonSetup, vec![label], jobs::Touches::Nothing, move |ctx| {
            let info = env.install(variant, &uv(&env, ctx)?, ctx.reporter().as_ref()).map_err(|e| e.to_string())?;
            update_settings(|s| s.set_train_device(variant.device()))?;
            let device = if info.cuda_available || info.mps_available { info.device_name.clone() } else { "CPU".into() };
            Ok(Some(format!("Python {} · torch {} · {device}", info.python, info.torch)))
        })
    }

    pub fn update_settings(change: impl FnOnce(&mut Settings)) -> Result<(), String> {
        let path = crate::api::settings::settings_path()?;
        let mut settings = Settings::load(&path).map_err(|e| format!("Couldn't read the settings: {e}"))?;
        change(&mut settings);
        settings.save(&path).map_err(|e| format!("Couldn't save the settings: {e}"))
    }
}
