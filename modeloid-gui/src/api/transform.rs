//! Transform: preview a plan on a dataset's labels and run it
//! (`JobKind::Transform`). The copies are made by Albumentations in the
//! Python environment (see the core's `core::transform`).

use dioxus::prelude::*;

pub use modeloid_types::transform::*;

use crate::api::datasets::OutputTarget;
use crate::api::jobs::Job;

/// What `plan` gives on its input, counted exactly (fast once its labels are read).
#[post("/api/transform/preview")]
pub async fn transform_preview(plan: TransformPlan) -> Result<TransformPreview, ServerFnError> {
    backend::preview(plan).await.map_err(ServerFnError::new)
}

/// Starts making the augmented dataset (a `Transform` job).
#[post("/api/transform/run")]
pub async fn start_transform(plan: TransformPlan, output: OutputTarget) -> Result<Job, ServerFnError> {
    backend::run(plan, output).map_err(ServerFnError::new)
}

/// Shown when the Python environment can't run Albumentations yet.
pub const ALBUMENTATIONS_MISSING: &str = "Albumentations isn't installed yet. Update the Python environment in Settings to add it.";

#[cfg(feature = "server")]
mod backend {
    use modeloid::{PyEnv, Workspace};

    use super::{TransformPlan, TransformPreview, ALBUMENTATIONS_MISSING};
    use crate::api::datasets::{resolve_output, OutputTarget};
    use crate::api::explore::cached_labels;
    use crate::api::jobs::{self, Job, JobKind};
    use crate::api::{activity, workspace};

    fn open() -> Result<Workspace, String> {
        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))
    }

    /// The environment, if it has Albumentations.
    fn python() -> Result<PyEnv, String> {
        let env = PyEnv::locate().map_err(|e| e.to_string())?;
        match env.status().info() {
            Some(info) if !info.albumentations.is_empty() => Ok(env),
            _ => Err(ALBUMENTATIONS_MISSING.into()),
        }
    }

    pub async fn preview(plan: TransformPlan) -> Result<TransformPreview, String> {
        activity::track("transform-preview", "Previewing transform", move |ctx| {
            let ws = open()?;
            if let Ok((_, dataset)) = ws.find_dataset(plan.input) {
                ctx.set_label(format!("Previewing transform · {}", dataset.name()));
            }
            let (_, set) = cached_labels(&ws, plan.input, &ctx.progress())?;
            Ok(ws.transform_preview_of(&plan, &set))
        })
        .await
    }

    pub fn run(plan: TransformPlan, output: OutputTarget) -> Result<Job, String> {
        let ws = open()?;
        let env = python()?;
        let name = ws.find_dataset(plan.input).map(|(_, d)| d.name().to_string()).map_err(|e| e.to_string())?;
        let output = resolve_output(output, &[plan.input])?;
        if let Some(problem) = plan.problems().into_iter().next() {
            return Err(problem);
        }
        jobs::start(JobKind::Transform, vec![name], jobs::Touches::Datasets(vec![plan.input]), move |ctx| {
            let reporter = ctx.reporter();
            let (dir, s) = ws.apply_transform(&plan, &output, &env, reporter.as_ref()).map_err(|e| e.to_string())?;
            let mut message = format!("{} augmented images added", s.added);
            if s.failed > 0 {
                message.push_str(&format!("; {} images couldn't be read ({})", s.failed, s.first_error.unwrap_or_default()));
            }
            reporter.message(&message);
            Ok(Some(dir.display().to_string()))
        })
    }
}
