//! Convert: write a dataset in another label format (YOLO, COCO, Pascal VOC)
//! and / or for another task (outlines → boxes, objects → classification crops).

use serde::{Deserialize, Serialize};

/// Annotation format name of a classification dataset: one folder per class.
pub const FOLDERS_FORMAT: &str = "folders";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetTask {
    /// Labels stay as they are (boxes, and outlines where the format has them).
    #[default]
    Same,
    /// Outlines become their bounding boxes.
    Detection,
    /// Every object is cut out into a folder named after its class.
    Classification,
}

impl TargetTask {
    /// Name for the lineage record and the CLI.
    pub fn name(self) -> &'static str {
        match self {
            TargetTask::Same => "same",
            TargetTask::Detection => "detection",
            TargetTask::Classification => "classification",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertPlan {
    /// Input dataset id.
    pub input: u32,
    #[serde(default)]
    pub task: TargetTask,
    /// Output label format: "yolo", "coco" or "voc". Not used for classification.
    pub format: String,
    /// Classification: context kept around each box, in percent of its size.
    #[serde(default)]
    pub padding_percent: u32,
    /// Classification: crops narrower or lower than this many pixels are skipped.
    #[serde(default)]
    pub min_size: u32,
}

/// Largest padding a plan may ask for.
pub const MAX_PADDING: u32 = 100;

/// Usual `min_size`: smaller crops are too small to learn from.
pub const DEFAULT_MIN_CROP: u32 = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassCount {
    pub name: String,
    pub labels: u32,
}

/// What a plan would write, from the input's statistics (no image is read).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertPreview {
    /// The input's format family ("yolo", "coco", "voc"); empty when it isn't one of them.
    pub source_format: String,
    pub images: u32,
    pub labels: u32,
    /// Objects of the input that have an outline.
    pub outlines: u32,
    /// Outlines that become plain boxes (detection task, or a format without outlines).
    pub outlines_lost: u32,
    /// Labels per class (classification: crops per class folder, before the size limit).
    pub classes: Vec<ClassCount>,
    /// Things to know that don't stop the run.
    pub notes: Vec<String>,
    /// Problems that stop the run.
    pub problems: Vec<String>,
}

impl ConvertPreview {
    pub fn can_run(&self) -> bool {
        self.problems.is_empty()
    }
}
