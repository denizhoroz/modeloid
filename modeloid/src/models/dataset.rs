use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{error::Error, fs, path::Path};

use crate::core::scan::DatasetStats;

pub const DATASET_FILE: &str = "dataset.toml";

#[derive(Serialize, Deserialize)]
struct DatasetFile<T> {
    dataset: T,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dataset {
    id: u32,
    name: String,
    #[serde(default = "default_version")]
    format_version: u32,
    dataset_type: String, // object-detection, instance-segmentation etc.
    image_format: String, // jpg, png
    annotation_format: String, // coco, yolov8
    image_count: u32,
    label_count: u32, // annotation instances (boxes, masks)
    class_names: Vec<String>,
    size_bytes: u64,
    train_count: u32, // training images quantity
    test_count: u32, // test images quantity
    val_count: u32, // val images quantity
    added_at: DateTime<Utc>,
    last_updated: DateTime<Utc>,

    // optional
    #[serde(default)]
    description: String,
    #[serde(default)]
    dataset_link: String,
    #[serde(default)]
    license: String,
    #[serde(default)]
    tags: Vec<String>,
}

impl Dataset {
    pub fn from_stats(id: u32, name: String, stats: DatasetStats) -> Self {
        let now = Utc::now();
        Dataset {
            id,
            name,
            format_version: default_version(),
            dataset_type: stats.dataset_type,
            image_format: stats.image_format,
            annotation_format: stats.annotation_format,
            image_count: stats.image_count,
            label_count: stats.label_count,
            class_names: stats.class_names,
            size_bytes: stats.size_bytes,
            train_count: stats.train_count,
            test_count: stats.test_count,
            val_count: stats.val_count,
            added_at: now,
            last_updated: now,
            description: String::new(),
            dataset_link: String::new(),
            license: String::new(),
            tags: Vec::new(),
        }
    }

    pub fn load_from_dir(dir: &Path) -> Result<Self, Box<dyn Error>> {
        let path = dir.join(DATASET_FILE);
        let contents = fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;

        let file: DatasetFile<Dataset> =
            toml::from_str(&contents).map_err(|e| format!("invalid {}: {e}", path.display()))?;
        Ok(file.dataset)
    }

    pub fn save_to_dir(&self, dir: &Path) -> Result<(), Box<dyn Error>> {
        let file = &DatasetFile { dataset: self };
        let contents = toml::to_string(file)?;
        fs::write(dir.join(DATASET_FILE), contents)?;

        Ok(())
    }

    /// Marks the dataset as modified; call before saving after a merge or edit.
    pub fn touch(&mut self) {
        self.last_updated = Utc::now();
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn image_count(&self) -> u32 {
        self.image_count
    }

    pub fn label_count(&self) -> u32 {
        self.label_count
    }

    pub fn dataset_type(&self) -> &str {
        &self.dataset_type
    }

    pub fn annotation_format(&self) -> &str {
        &self.annotation_format
    }

    pub fn image_format(&self) -> &str {
        &self.image_format
    }

    pub fn class_names(&self) -> &[String] {
        &self.class_names
    }

    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    pub fn set_dataset_type(&mut self, dataset_type: String) {
        self.dataset_type = dataset_type;
    }

    pub fn set_annotation_format(&mut self, annotation_format: String) {
        self.annotation_format = annotation_format;
    }

    pub fn set_description(&mut self, description: String) {
        self.description = description;
    }

    pub fn set_dataset_link(&mut self, dataset_link: String) {
        self.dataset_link = dataset_link;
    }

    pub fn set_license(&mut self, license: String) {
        self.license = license;
    }

    pub fn set_tags(&mut self, tags: Vec<String>) {
        self.tags = tags;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats() -> DatasetStats {
        DatasetStats {
            dataset_type: "object-detection".into(),
            image_format: "jpg".into(),
            annotation_format: "yolo".into(),
            image_count: 10,
            label_count: 25,
            class_names: vec!["cat".into(), "dog".into()],
            size_bytes: 5_000_000_000,
            train_count: 7,
            test_count: 1,
            val_count: 2,
        }
    }

    #[test]
    fn roundtrips_through_toml() {
        let dir = std::env::temp_dir().join(format!("modeloid-dataset-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();

        let mut dataset = Dataset::from_stats(3, "cats".into(), stats());
        dataset.set_tags(vec!["pets".into()]);
        dataset.save_to_dir(&dir).unwrap();
        let loaded = Dataset::load_from_dir(&dir).unwrap();
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(loaded.id(), 3);
        assert_eq!(loaded.size_bytes, 5_000_000_000);
        assert_eq!(loaded.class_names, dataset.class_names);
        assert_eq!(loaded.tags, vec!["pets".to_string()]);
        assert_eq!(loaded.added_at, dataset.added_at);
    }

    #[test]
    fn loads_without_optional_fields() {
        let contents = r#"
            [dataset]
            id = 1
            name = "cats"
            dataset_type = "object-detection"
            image_format = "jpg"
            annotation_format = "coco"
            image_count = 1
            label_count = 1
            class_names = ["cat"]
            size_bytes = 100
            train_count = 1
            test_count = 0
            val_count = 0
            added_at = "2026-09-25T10:00:00Z"
            last_updated = "2026-09-25T10:00:00Z"
        "#;
        let file: DatasetFile<Dataset> = toml::from_str(contents).unwrap();
        assert_eq!(file.dataset.format_version, 1);
        assert!(file.dataset.tags.is_empty());
    }
}
