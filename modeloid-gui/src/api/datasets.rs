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
    pub size_bytes: u64,
}

/// Where a tool should put its result, as chosen in `OutputChoice`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OutputTarget {
    /// A new dataset with this folder name.
    New { name: String },
    /// Replace the (single) input dataset.
    Overwrite,
}

pub use modeloid_types::validate_dataset_name;

/// Datasets of the open workspace ordered by id; empty when none is open.
#[get("/api/datasets")]
pub async fn list_datasets() -> Result<Vec<DatasetRow>, ServerFnError> {
    backend::list().map_err(ServerFnError::new)
}

/// Starts deleting the datasets `ids` from the workspace, with all their
/// images and labels (`JobKind::Delete`). Refused while a job works on one of them.
#[post("/api/datasets/delete")]
pub async fn start_delete(ids: Vec<u32>) -> Result<crate::api::jobs::Job, ServerFnError> {
    backend::delete(ids).map_err(ServerFnError::new)
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
mod backend {
    use modeloid::{Output, Workspace};

    use super::{DatasetRow, OutputTarget};
    use crate::api::jobs::{self, Job, JobKind};
    use crate::api::workspace;

    pub fn delete(ids: Vec<u32>) -> Result<Job, String> {
        if ids.is_empty() {
            return Err("Select at least one dataset.".into());
        }
        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        let ws = Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))?;
        let names: Vec<String> = ids
            .iter()
            .map(|&id| ws.find_dataset(id).map(|(_, d)| d.name().to_string()).map_err(|e| e.to_string()))
            .collect::<Result<_, _>>()?;
        jobs::start(JobKind::Delete, names, jobs::Touches::Datasets(ids.clone()), move |ctx| {
            ws.delete_dataset(ids[ctx.index()]).map(|_| None).map_err(|e| e.to_string())
        })
    }

    /// Core `Output` for a tool run on `inputs`. Overwriting needs exactly one input.
    #[allow(dead_code)] // used by the tools still to come
    pub fn resolve_output(target: OutputTarget, inputs: &[u32]) -> Result<Output, String> {
        match (target, inputs) {
            (OutputTarget::New { name }, _) => {
                modeloid::validate_dataset_name(&name)?;
                Ok(Output::New { name })
            }
            (OutputTarget::Overwrite, &[id]) => Ok(Output::Overwrite { id }),
            (OutputTarget::Overwrite, _) => Err("Overwriting works with exactly one input dataset.".into()),
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
        assert_eq!(resolve_output(new, &[1, 2]), Ok(Output::New { name: "cats".into() }));
        assert_eq!(resolve_output(OutputTarget::Overwrite, &[3]), Ok(Output::Overwrite { id: 3 }));
        assert!(resolve_output(OutputTarget::Overwrite, &[1, 2]).is_err());
        assert!(resolve_output(OutputTarget::New { name: "a/b".into() }, &[1]).is_err());
    }
}
