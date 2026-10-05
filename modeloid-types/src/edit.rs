//! Datasets > Create and Edit: making an empty dataset, changing its
//! details, adding images and how far its labelling is.

use serde::{Deserialize, Serialize};

/// Dataset types an empty dataset can be created as: (`dataset_type`, label).
pub const CREATE_TYPES: &[(&str, &str)] = &[
    ("object-detection", "Object detection"),
    ("instance-segmentation", "Instance segmentation"),
];

/// What Create asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewDataset {
    pub name: String,
    pub description: String,
    /// One of `CREATE_TYPES`.
    pub dataset_type: String,
    /// A format id Modeloid can write (`formats::writable`).
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

/// One label of the labelling interface, in image pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelObject {
    /// Index into the class list.
    pub class: usize,
    /// x, y (top left), width, height.
    pub bbox: [f64; 4],
    /// Outline of a segmentation label; shown, not drawn (boxes only for now).
    pub polygon: Option<Vec<[f64; 2]>>,
}

/// An image opened in the labelling interface, with its labels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelImage {
    /// Position in the dataset's images (stable while its files don't change).
    pub index: usize,
    pub name: String,
    pub split: Option<crate::Split>,
    pub width: u32,
    pub height: u32,
    pub objects: Vec<LabelObject>,
}

/// What the labelling interface needs about a dataset when it opens.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelSession {
    /// The dataset's name, sent back with every change: ids are renumbered
    /// after jobs, so a stale page is refused rather than changing another dataset.
    pub dataset: String,
    /// Label format, e.g. "yolo".
    pub format: String,
    pub classes: Vec<String>,
    /// Labels of each class over the whole dataset.
    pub totals: Vec<u32>,
    /// Whether each image has at least one label, by index.
    pub labelled: Vec<bool>,
    /// Why the dataset can't be changed right now (a job works on it).
    pub busy: Option<String>,
}

/// What saving one image's labels changed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedLabels {
    pub totals: Vec<u32>,
    pub labelled: bool,
}

/// A new or renamed class name, trimmed: not empty, at most 64 characters,
/// no line breaks, and not one of `existing` (ignoring case) other than the
/// one at `except` (the class being renamed).
pub fn validate_class_name(name: &str, existing: &[String], except: Option<usize>) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a class name.".into());
    }
    if name.chars().count() > 64 {
        return Err("Class names can be at most 64 characters.".into());
    }
    if name.chars().any(char::is_control) {
        return Err("Class names can't contain line breaks or tabs.".into());
    }
    let taken = existing.iter().enumerate().any(|(i, c)| Some(i) != except && c.to_lowercase() == name.to_lowercase());
    if taken {
        return Err(format!("There already is a class called '{name}'."));
    }
    Ok(name.to_string())
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
    fn validates_class_names() {
        let existing = vec!["Car".to_string(), "bus".to_string()];
        assert_eq!(validate_class_name("  truck ", &existing, None), Ok("truck".into()));
        assert!(validate_class_name(" ", &existing, None).is_err());
        assert!(validate_class_name("car", &existing, None).is_err(), "case doesn't make it new");
        assert_eq!(validate_class_name("car", &existing, Some(0)), Ok("car".into()), "renaming itself");
        assert!(validate_class_name("a\nb", &existing, None).is_err());
        assert!(validate_class_name(&"x".repeat(65), &existing, None).is_err());
    }

    #[test]
    fn knows_image_names() {
        assert!(is_image_name("a.JPG"));
        assert!(is_image_name("x.y.webp"));
        assert!(!is_image_name("notes.txt"));
        assert!(!is_image_name("png"));
    }
}
