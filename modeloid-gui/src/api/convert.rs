//! Convert: preview a conversion from the dataset's statistics and run it
//! (`JobKind::Convert`).

use dioxus::prelude::*;

pub use modeloid_types::convert::{ConvertPlan, ConvertPreview, TargetTask, DEFAULT_MIN_CROP, MAX_PADDING};

use crate::api::datasets::OutputTarget;
use crate::api::jobs::Job;

/// What `plan` would write (from cached statistics; fast once they exist).
#[post("/api/convert/preview")]
pub async fn convert_preview(plan: ConvertPlan) -> Result<ConvertPreview, ServerFnError> {
    backend::preview(plan).await.map_err(ServerFnError::new)
}

/// Starts writing the converted dataset.
#[post("/api/convert/run")]
pub async fn start_convert(plan: ConvertPlan, output: OutputTarget) -> Result<Job, ServerFnError> {
    backend::run(plan, output).map_err(ServerFnError::new)
}

#[cfg(feature = "server")]
mod backend {
    use modeloid::core::convert::TargetTask;
    use modeloid::{Silent, Workspace};

    use super::{ConvertPlan, ConvertPreview};
    use crate::api::datasets::{resolve_output, OutputTarget};
    use crate::api::jobs::{self, Job, JobKind};
    use crate::api::{activity, workspace};

    fn open() -> Result<Workspace, String> {
        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))
    }

    fn name_of(ws: &Workspace, id: u32) -> Result<String, String> {
        ws.find_dataset(id).map(|(_, d)| d.name().to_string()).map_err(|e| e.to_string())
    }

    pub async fn preview(plan: ConvertPlan) -> Result<ConvertPreview, String> {
        activity::track("convert-preview", "Previewing conversion", move |ctx| {
            let ws = open()?;
            if let Ok(name) = name_of(&ws, plan.input) {
                ctx.set_label(format!("Previewing conversion · {name}"));
            }
            ws.convert_preview(&plan, &ctx.progress()).map_err(|e| e.to_string())
        })
        .await
    }

    pub fn run(plan: ConvertPlan, output: OutputTarget) -> Result<Job, String> {
        let ws = open()?;
        let name = name_of(&ws, plan.input)?;
        let output = resolve_output(output, &[plan.input])?;
        // Cached statistics: the job checks again on the labels it reads.
        let preview = ws.convert_preview(&plan, &Silent).map_err(|e| e.to_string())?;
        if let Some(problem) = preview.problems.first() {
            return Err(problem.clone());
        }
        jobs::start(JobKind::Convert, vec![name], move |ctx| {
            let reporter = ctx.reporter();
            let (dir, s) = ws.apply_convert(&plan, &output, reporter.as_ref()).map_err(|e| e.to_string())?;
            reporter.message(&if plan.task == TargetTask::Classification {
                format!("{} crops, {} too small, {} unreadable images", s.crops.crops, s.crops.skipped_small, s.crops.unreadable_images)
            } else {
                format!("{} images, {} outlines turned into boxes", s.images, s.outlines_lost)
            });
            Ok(Some(dir.display().to_string()))
        })
    }
}
