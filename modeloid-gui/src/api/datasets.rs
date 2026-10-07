//! Datasets in the open workspace, read from each folder's `dataset.toml`.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// One row of the datasets panel: a summary, not every `dataset.toml` field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetRow {
    pub id: u32,
    pub name: String,
    /// e.g. "object-detection", "instance-segmentation".
    pub task: String,
    /// Annotation format, e.g. "coco", "yolov8".
    pub format: String,
    pub images: u32,
    pub labels: u32,
    pub classes: usize,
    /// What this dataset links to (its images are theirs); empty for a
    /// dataset with its own images. A deleted source reads "<name> (removed)".
    #[serde(default)]
    pub linked_to: Vec<String>,
    pub size_bytes: u64,
}

/// Where a tool should put its result, as chosen in `OutputChoice`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OutputTarget {
    /// A new dataset with this folder name.
    New { name: String },
    /// Replace the (single) input dataset.
    Overwrite,
    /// A new dataset link with this folder name (Classes & Merge).
    Link { name: String },
}

pub use modeloid_types::validate_dataset_name;

/// Datasets of the open workspace ordered by id; empty when none is open.
#[get("/api/datasets")]
pub async fn list_datasets() -> Result<Vec<DatasetRow>, ServerFnError> {
    backend::list().map_err(ServerFnError::new)
}

/// Starts deleting the datasets `ids` from the workspace, with all their
/// images and labels (`JobKind::Delete`). `names` are the names the page
/// showed for them: ids are renumbered after jobs, so a page that hasn't
/// caught up yet is refused rather than deleting another dataset. Refused
/// while a job works on one of them.
#[post("/api/datasets/delete")]
pub async fn start_delete(ids: Vec<u32>, names: Vec<String>) -> Result<crate::api::jobs::Job, ServerFnError> {
    backend::delete(ids, names).map_err(ServerFnError::new)
}

/// Short label for a task name, for the narrow Task column.
pub fn short_task(task: &str) -> &str {
    match task {
        "object-detection" => "Detection",
        "instance-segmentation" => "Inst. seg",
        "semantic-segmentation" => "Sem. seg",
        "classification" | "single-label-classification" => "Classify",
        "multi-label-classification" => "Multi-label",
        "keypoint-detection" => "Keypoints",
        "" => "—",
        other => other,
    }
}

#[cfg(feature = "server")]
#[allow(unused_imports)] // used by the tools still to come
pub use backend::resolve_output;
#[cfg(feature = "server")]
pub use backend::renumber;

#[cfg(feature = "server")]
mod backend {
    use modeloid::{Output, Workspace};

    use super::{DatasetRow, OutputTarget};
    use crate::api::jobs::{self, Job, JobKind};
    use crate::api::workspace;

    pub fn delete(ids: Vec<u32>, expected: Vec<String>) -> Result<Job, String> {
        if ids.is_empty() {
            return Err("Select at least one dataset.".into());
        }
        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        let ws = Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))?;
        let names: Vec<String> = ids
            .iter()
            .map(|&id| ws.find_dataset(id).map(|(_, d)| d.name().to_string()).map_err(|e| e.to_string()))
            .collect::<Result<_, _>>()?;
        if names != expected {
            return Err("The datasets have changed meanwhile. Check the selection and try again.".into());
        }
        jobs::start(JobKind::Delete, names, jobs::Touches::Datasets(ids.clone()), move |ctx| {
            ws.delete_dataset(ids[ctx.index()]).map(|_| None).map_err(|e| e.to_string())
        })
    }

    /// Core `Output` for a tool run on `inputs`. Overwriting needs exactly one
    /// input; a dataset link only where the tool makes them (`allow_link`).
    #[allow(dead_code)] // used by the tools still to come
    pub fn resolve_output(target: OutputTarget, inputs: &[u32], allow_link: bool) -> Result<Output, String> {
        match (target, inputs) {
            (OutputTarget::New { name }, _) => {
                modeloid::validate_dataset_name(&name)?;
                Ok(Output::New { name })
            }
            (OutputTarget::Link { name }, _) if allow_link => {
                modeloid::validate_dataset_name(&name)?;
                Ok(Output::Link { name })
            }
            (OutputTarget::Link { .. }, _) => Err("Only Classes & Merge makes dataset links.".into()),
            (OutputTarget::Overwrite, &[id]) => Ok(Output::Overwrite { id }),
            (OutputTarget::Overwrite, _) => Err("Overwriting works with exactly one input dataset.".into()),
        }
    }

    /// Gives the open workspace's datasets the ids 1, 2, 3… again. Call only
    /// while no job runs (`jobs::renumber_when_idle`); a failure is only logged.
    pub fn renumber() {
        let Some(dir) = workspace::current_dir() else { return };
        let renumbered = Workspace::load_from_dir(&dir).and_then(|ws| ws.renumber_datasets());
        if let Err(e) = renumbered {
            eprintln!("Couldn't renumber the datasets: {e}");
        }
    }

    pub fn list() -> Result<Vec<DatasetRow>, String> {
        let Some(dir) = workspace::current_dir() else { return Ok(Vec::new()) };
        let ws = Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))?;
        let datasets = ws.list_datasets().map_err(|e| format!("Couldn't read the datasets: {e}"))?;

        Ok(datasets
            .into_iter()
            .map(|(_, d)| DatasetRow {
                id: d.id(),
                name: d.name().to_string(),
                task: d.dataset_type().to_string(),
                format: d.annotation_format().to_string(),
                images: d.image_count(),
                labels: d.label_count(),
                classes: d.class_names().len(),
                linked_to: d
                    .linked_to()
                    .iter()
                    .map(|s| if s.id == 0 { format!("{} (removed)", s.name) } else { s.name.clone() })
                    .collect(),
                size_bytes: d.size_bytes(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "server")]
    #[test]
    fn resolves_output() {
        use modeloid::Output;

        let new = OutputTarget::New { name: "cats".into() };
        assert_eq!(resolve_output(new, &[1, 2], false), Ok(Output::New { name: "cats".into() }));
        assert_eq!(resolve_output(OutputTarget::Overwrite, &[3], false), Ok(Output::Overwrite { id: 3 }));
        assert!(resolve_output(OutputTarget::Overwrite, &[1, 2], false).is_err());
        assert!(resolve_output(OutputTarget::New { name: "a/b".into() }, &[1], false).is_err());
        let link = OutputTarget::Link { name: "cars".into() };
        assert_eq!(resolve_output(link.clone(), &[1], true), Ok(Output::Link { name: "cars".into() }));
        assert!(resolve_output(link, &[1], false).is_err(), "only where the tool makes links");
    }
}
