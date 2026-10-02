//! Datasets > Create and Edit: making an empty dataset, changing its
//! details, adding images and how far its labelling is.

use serde::{Deserialize, Serialize};

/// Dataset types an empty dataset can be created as: (`dataset_type`, label).
pub const CREATE_TYPES: &[(&str, &str)] = &[
    ("object-detection", "Object detection"),
    ("instance-segmentation", "Instance segmentation"),
];

/// Label formats an empty dataset can be created in: (`annotation_format`, label).
pub const CREATE_FORMATS: &[(&str, &str)] = &[("yolo", "YOLO"), ("coco", "COCO"), ("voc", "Pascal VOC")];

/// What Create asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewDataset {
    pub name: String,
    pub description: String,
    /// One of `CREATE_TYPES`.
    pub dataset_type: String,
    /// One of `CREATE_FORMATS`.
    pub annotation_format: String,
}

/// The details Edit can change. Everything else comes from the files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetDetails {
    pub name: String,
    pub description: String,
    pub link: String,
    pub license: String,
    pub tags: Vec<String>,
}

/// Labelling progress of a dataset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelStatus {
    pub images: u32,
    /// Images without a single label.
    pub unlabelled: u32,
    /// Labels (boxes, outlines) over all images.
    pub labels: u32,
    pub classes: u32,
}

/// What adding images did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddedImages {
    pub added: u32,
    /// Added under another name because the dataset already had one like it.
    pub renamed: u32,
    /// Files that aren't images, by name.
    pub skipped: Vec<String>,
}

/// File extensions taken as images when adding them.
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "bmp", "webp", "tif", "tiff"];

/// Whether a file name has an image extension (`IMAGE_EXTENSIONS`).
pub fn is_image_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Tags typed as a comma separated list: trimmed, empty ones and repeats dropped.
pub fn parse_tags(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for tag in text.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        if !tags.iter().any(|t| t == tag) {
            tags.push(tag.to_string());
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tags() {
        assert_eq!(parse_tags(" cars, night ,,cars, "), ["cars", "night"]);
        assert!(parse_tags("  ").is_empty());
    }

    #[test]
    fn knows_image_names() {
        assert!(is_image_name("a.JPG"));
        assert!(is_image_name("x.y.webp"));
        assert!(!is_image_name("notes.txt"));
        assert!(!is_image_name("png"));
    }
}
