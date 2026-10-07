//! Classes & Merge: class mapping suggestions, previews, saved class lists,
//! and running the mapping (`JobKind::ClassesMerge`).

use dioxus::prelude::*;

pub use modeloid_types::classes::{ClassList, MergePlan, MergePreview, SplitMode};

use crate::api::datasets::OutputTarget;
use crate::api::jobs::Job;

/// A first mapping for these inputs: same-looking class names joined.
#[post("/api/classes/suggest")]
pub async fn class_merge_suggestion(inputs: Vec<u32>) -> Result<MergePlan, ServerFnError> {
    backend::tracked("classes-suggest", "Suggesting class mapping", move |ws, progress| {
        ws.class_merge_suggestion(&inputs, progress).map_err(|e| e.to_string())
    })
    .await
}

/// What `plan` would produce (from cached statistics; fast).
#[post("/api/classes/preview")]
pub async fn class_merge_preview(plan: MergePlan) -> Result<MergePreview, ServerFnError> {
    backend::tracked("classes-preview", "Previewing class mapping", move |ws, progress| {
        ws.class_merge_preview(&plan, progress).map_err(|e| e.to_string())
    })
    .await
}

/// Starts writing the mapped (and merged) dataset. `delete_inputs` deletes
/// the input datasets once the new one is written (merges into a new dataset only).
#[post("/api/classes/run")]
pub async fn start_class_merge(plan: MergePlan, output: OutputTarget, delete_inputs: bool) -> Result<Job, ServerFnError> {
    backend::run(plan, output, delete_inputs).map_err(ServerFnError::new)
}

#[get("/api/classes/lists")]
pub async fn class_lists() -> Result<Vec<ClassList>, ServerFnError> {
    backend::blocking(|ws| ws.class_lists().map_err(|e| e.to_string())).await
}

#[post("/api/classes/lists/save")]
pub async fn save_class_list(list: ClassList) -> Result<(), ServerFnError> {
    backend::blocking(move |ws| {
        ws.save_class_list(&list).map_err(|e| e.to_string())?;
        #[cfg(feature = "server")]
        crate::api::history::record("Dataset Manager", "Classes & Merge", "Save class list", list.name.clone(), None);
        Ok(())
    })
    .await
}

#[post("/api/classes/lists/delete")]
pub async fn delete_class_list(name: String) -> Result<(), ServerFnError> {
    backend::blocking(move |ws| {
        ws.delete_class_list(&name).map_err(|e| e.to_string())?;
        #[cfg(feature = "server")]
        crate::api::history::record("Dataset Manager", "Classes & Merge", "Delete class list", name.clone(), None);
        Ok(())
    })
    .await
}

#[cfg(feature = "server")]
mod backend {
    use dioxus::prelude::ServerFnError;
    use modeloid::{Progress, Silent, Workspace};

    use super::MergePlan;
    use crate::api::datasets::{resolve_output, OutputTarget};
    use crate::api::jobs::{self, Job, JobKind};
    use crate::api::{activity, workspace};

    fn open() -> Result<Workspace, String> {
        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))
    }

    /// Like `blocking`, but listed in the status bar and cancelled by a newer
    /// call in the same `slot`. For reads only: a cancelled write would be lost.
    pub async fn tracked<T: Send + 'static>(
        slot: &'static str,
        label: &'static str,
        f: impl FnOnce(&Workspace, &dyn Progress) -> Result<T, String> + Send + 'static,
    ) -> Result<T, ServerFnError> {
        activity::track(slot, label, move |ctx| f(&open()?, &ctx.progress())).await.map_err(ServerFnError::new)
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

    pub fn run(plan: MergePlan, output: OutputTarget, delete_inputs: bool) -> Result<Job, String> {
        let ws = open()?;
        if delete_inputs && (plan.inputs.len() < 2 || matches!(output, OutputTarget::Overwrite)) {
            return Err("Deleting the inputs works only when merging several datasets into a new one.".into());
        }
        let output = resolve_output(output, &plan.inputs, true)?;
        let preview = ws.class_merge_preview(&plan, &Silent).map_err(|e| e.to_string())?;
        if let Some(problem) = preview.problems.first() {
            return Err(problem.clone());
        }
        if let Some((_, class)) = preview.unmapped.first() {
            return Err(format!("Choose what happens to \"{class}\" first."));
        }
        let label = match preview.input_names.as_slice() {
            [one] => one.clone(),
            many => format!("{} datasets", many.len()),
        };
        jobs::start(JobKind::ClassesMerge, vec![label], jobs::Touches::Datasets(plan.inputs.clone()), move |ctx| {
            let reporter = ctx.reporter();
            let (dir, s) = ws.apply_class_merge(&plan, &output, reporter.as_ref()).map_err(|e| e.to_string())?;
            let mut message = format!("{} images, {} removed, {} objects dropped", s.images, s.removed_images, s.dropped_objects);
            if delete_inputs {
                // The merged dataset is written; a failure here leaves it and says which inputs stayed.
                let failed: Vec<String> = plan.inputs.iter().filter_map(|&id| ws.delete_dataset(id).err().map(|e| format!("#{id}: {e}"))).collect();
                if !failed.is_empty() {
                    return Err(format!("Merged into {}, but couldn't delete {}", dir.display(), failed.join("; ")));
                }
                message.push_str(&format!(", {} source datasets deleted", plan.inputs.len()));
            }
            reporter.message(&message);
            Ok(Some(dir.display().to_string()))
        })
    }
}
