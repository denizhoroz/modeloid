//! Dataset splits.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Split {
    Train,
    Val,
    Test,
}

impl Split {
    /// The split a dataset-relative path belongs to, from folder names like `train/` or `valid/`.
    pub fn of_path(rel: &Path) -> Option<Split> {
        rel.components().find_map(|c| match c.as_os_str().to_str()?.to_ascii_lowercase().as_str() {
            "train" => Some(Split::Train),
            "valid" | "val" | "validation" => Some(Split::Val),
            "test" => Some(Split::Test),
            _ => None,
        })
    }

    /// Folder name used when writing (Roboflow's names).
    pub fn dir_name(self) -> &'static str {
        match self {
            Split::Train => "train",
            Split::Val => "valid",
            Split::Test => "test",
        }
    }

    /// Name to show, e.g. "Train".
    pub fn label(split: Option<Split>) -> &'static str {
        match split {
            Some(Split::Train) => "Train",
            Some(Split::Val) => "Val",
            Some(Split::Test) => "Test",
            None => "No split",
        }
    }
}
