//! Convert: preview a conversion from the dataset's statistics and run it
//! (`JobKind::Convert`), for one dataset or several at once.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

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

/// Where a batch conversion puts each result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BatchOutput {
    /// A new dataset per input, named `<input name><suffix>`.
    New { suffix: String },
    /// Replace each input.
    Overwrite,
}

/// `plan` (its `input` aside) previewed for each of `inputs`, in their order.
#[post("/api/convert/batch-preview")]
pub async fn convert_batch_preview(plan: ConvertPlan, inputs: Vec<u32>) -> Result<Vec<ConvertPreview>, ServerFnError> {
    backend::batch_preview(plan, inputs).await.map_err(ServerFnError::new)
}

/// Starts converting each of `inputs` with `plan` (its `input` aside), one
/// job item each. Datasets where nothing would change are skipped.
#[post("/api/convert/run-batch")]
pub async fn start_convert_batch(plan: ConvertPlan, inputs: Vec<u32>, output: BatchOutput) -> Result<Job, ServerFnError> {
    backend::run_batch(plan, inputs, output).map_err(ServerFnError::new)
}

#[cfg(feature = "server")]
mod backend {
    use modeloid::core::convert::TargetTask;
    use modeloid::{Output, Silent, Workspace};

    use super::{BatchOutput, ConvertPlan, ConvertPreview};
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
        if let Some(problem) = preview.problems.first().or(preview.unchanged.as_ref()) {
            return Err(problem.clone());
        }
        jobs::start(JobKind::Convert, vec![name], jobs::Touches::Datasets(vec![plan.input]), move |ctx| {
            convert_one(&ws, &plan, &output, ctx)
        })
    }

    /// One job item: converts `plan.input` to `output`.
    fn convert_one(ws: &Workspace, plan: &ConvertPlan, output: &Output, ctx: &jobs::ItemCtx) -> Result<Option<String>, String> {
        let reporter = ctx.reporter();
        let (dir, s) = ws.apply_convert(plan, output, reporter.as_ref()).map_err(|e| e.to_string())?;
        reporter.message(&if plan.task == TargetTask::Classification {
            format!("{} crops, {} too small, {} unreadable images", s.crops.crops, s.crops.skipped_small, s.crops.unreadable_images)
        } else {
            format!("{} images, {} outlines turned into boxes", s.images, s.outlines_lost)
        });
        Ok(Some(dir.display().to_string()))
    }

    pub async fn batch_preview(plan: ConvertPlan, inputs: Vec<u32>) -> Result<Vec<ConvertPreview>, String> {
        activity::track("convert-preview", "Previewing conversion", move |ctx| {
            let ws = open()?;
            ctx.set_label(format!("Previewing conversion · {} datasets", inputs.len()));
            inputs
                .iter()
                .map(|&input| ws.convert_preview(&ConvertPlan { input, ..plan.clone() }, &ctx.progress()).map_err(|e| e.to_string()))
                .collect()
        })
        .await
    }

    pub fn run_batch(plan: ConvertPlan, inputs: Vec<u32>, output: BatchOutput) -> Result<Job, String> {
        if inputs.is_empty() {
            return Err("Select at least one dataset.".into());
        }
        let ws = open()?;
        let names: Vec<String> = inputs.iter().map(|&id| name_of(&ws, id)).collect::<Result<_, _>>()?;
        let previews: Vec<ConvertPreview> = inputs
            .iter()
            .map(|&input| ws.convert_preview(&ConvertPlan { input, ..plan.clone() }, &Silent).map_err(|e| e.to_string()))
            .collect::<Result<_, _>>()?;
        if !previews.iter().any(ConvertPreview::can_run) {
            return Err("Nothing to convert: every selected dataset is skipped or has a problem.".into());
        }

        let outputs: Vec<Output> = match &output {
            BatchOutput::Overwrite => inputs.iter().map(|&id| Output::Overwrite { id }).collect(),
            BatchOutput::New { suffix } => {
                let mut outputs = Vec::new();
                for (name, preview) in names.iter().zip(&previews) {
                    let new = format!("{name}{suffix}");
                    modeloid::validate_dataset_name(&new).map_err(|e| format!("{new}: {e}"))?;
                    if preview.can_run() && ws.datasets_dir().join(&new).exists() {
                        return Err(format!("A dataset named {new} exists already. Choose another suffix."));
                    }
                    outputs.push(Output::New { name: new });
                }
                outputs
            }
        };

        jobs::start(JobKind::Convert, names, jobs::Touches::Datasets(inputs.clone()), move |ctx| {
            let index = ctx.index();
            let plan = ConvertPlan { input: inputs[index], ..plan.clone() };
            // Statistics may have changed since the page's preview.
            let preview = ws.convert_preview(&plan, &Silent).map_err(|e| e.to_string())?;
            if let Some(reason) = &preview.unchanged {
                ctx.skip();
                return Err(reason.clone());
            }
            if let Some(problem) = preview.problems.first() {
                return Err(problem.clone());
            }
            convert_one(&ws, &plan, &outputs[index], ctx)
        })
    }
}
