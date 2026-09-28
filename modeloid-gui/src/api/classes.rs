//! Classes & Merge: class mapping suggestions, previews, saved class lists,
//! and running the mapping (`JobKind::ClassesMerge`).

use dioxus::prelude::*;

pub use modeloid_types::classes::{ClassList, MergePlan, MergePreview, SplitMode};

use crate::api::datasets::OutputTarget;
use crate::api::jobs::Job;

/// A first mapping for these inputs: same-looking class names joined.
#[post("/api/classes/suggest")]
pub async fn class_merge_suggestion(inputs: Vec<u32>) -> Result<MergePlan, ServerFnError> {
    backend::blocking(move |ws| ws.class_merge_suggestion(&inputs).map_err(|e| e.to_string())).await
}

/// What `plan` would produce (from cached statistics; fast).
#[post("/api/classes/preview")]
pub async fn class_merge_preview(plan: MergePlan) -> Result<MergePreview, ServerFnError> {
    backend::blocking(move |ws| ws.class_merge_preview(&plan).map_err(|e| e.to_string())).await
}

/// Starts writing the mapped (and merged) dataset.
#[post("/api/classes/run")]
pub async fn start_class_merge(plan: MergePlan, output: OutputTarget) -> Result<Job, ServerFnError> {
    backend::run(plan, output).map_err(ServerFnError::new)
}

#[get("/api/classes/lists")]
pub async fn class_lists() -> Result<Vec<ClassList>, ServerFnError> {
    backend::blocking(|ws| ws.class_lists().map_err(|e| e.to_string())).await
}

#[post("/api/classes/lists/save")]
pub async fn save_class_list(list: ClassList) -> Result<(), ServerFnError> {
    backend::blocking(move |ws| ws.save_class_list(&list).map_err(|e| e.to_string())).await
}

#[post("/api/classes/lists/delete")]
pub async fn delete_class_list(name: String) -> Result<(), ServerFnError> {
    backend::blocking(move |ws| ws.delete_class_list(&name).map_err(|e| e.to_string())).await
}

#[cfg(feature = "server")]
mod backend {
    use dioxus::prelude::ServerFnError;
    use modeloid::Workspace;

    use super::MergePlan;
    use crate::api::datasets::{resolve_output, OutputTarget};
    use crate::api::jobs::{self, Job, JobKind};
    use crate::api::workspace;

    fn open() -> Result<Workspace, String> {
        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))
    }

    /// Runs `f` with the open workspace on a blocking thread.
    pub async fn blocking<T: Send + 'static>(
        f: impl FnOnce(&Workspace) -> Result<T, String> + Send + 'static,
    ) -> Result<T, ServerFnError> {
        tokio::task::spawn_blocking(move || f(&open()?))
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?
            .map_err(ServerFnError::new)
    }

    pub fn run(plan: MergePlan, output: OutputTarget) -> Result<Job, String> {
        let ws = open()?;
        let output = resolve_output(output, &plan.inputs)?;
        let preview = ws.class_merge_preview(&plan).map_err(|e| e.to_string())?;
        if let Some(problem) = preview.problems.first() {
            return Err(problem.clone());
        }
        if let Some((_, class)) = preview.unmapped.first() {
            return Err(format!("Choose what happens to \"{class}\" first."));
        }
        let label = preview.input_names.join(" + ");
        jobs::start(JobKind::ClassesMerge, vec![label], move |ctx| {
            let reporter = ctx.reporter();
            let (dir, s) = ws.apply_class_merge(&plan, &output, reporter.as_ref()).map_err(|e| e.to_string())?;
            reporter.message(&format!("{} images, {} removed, {} objects dropped", s.images, s.removed_images, s.dropped_objects));
            Ok(Some(dir.display().to_string()))
        })
    }
}
