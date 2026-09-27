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

/// Datasets of the open workspace ordered by id; empty when none is open.
#[get("/api/datasets")]
pub async fn list_datasets() -> Result<Vec<DatasetRow>, ServerFnError> {
    backend::list().map_err(ServerFnError::new)
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
mod backend {
    use modeloid::Workspace;

    use super::DatasetRow;
    use crate::api::workspace;

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
