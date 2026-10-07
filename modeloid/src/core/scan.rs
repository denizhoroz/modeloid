use std::{
    collections::{BTreeSet, HashSet},
    fs, io,
    path::{Path, PathBuf},
};

use modeloid_types::convert::FOLDERS_FORMAT;
use serde_json::Value;

use crate::{
    core::{
        labels::{Split, via_class, via_entries, via_regions, via_shape},
        progress::{Cancelled, Progress},
    },
    models::dataset::DATASET_FILE,
};

pub(crate) const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "bmp", "webp", "tif", "tiff"];

/// Everything about a dataset that can be derived from its files on disk.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DatasetStats {
    pub dataset_type: String,
    pub image_format: String,
    pub annotation_format: String,
    pub image_count: u32,
    pub label_count: u32,
    pub class_names: Vec<String>,
    pub size_bytes: u64,
    /// The part of `size_bytes` that is image files, when known (scans). A
    /// dataset link's images are shared with its source, so they aren't its size.
    pub image_bytes: u64,
    pub train_count: u32,
    pub test_count: u32,
    pub val_count: u32,
}

#[derive(Default)]
struct Annotations {
    coco: Vec<PathBuf>,
    yolo_labels: Vec<PathBuf>,
    yaml: Vec<PathBuf>,
    voc: Vec<PathBuf>,
}

/// Walks `dir` and collects image, split, annotation and class statistics,
/// counting each visited file on `progress`.
pub fn scan_dataset(dir: &Path, progress: &dyn Progress) -> io::Result<DatasetStats> {
    let mut stats = DatasetStats::default();
    let mut image_formats = BTreeSet::new();
    let mut found = Annotations::default();
    // Folders holding images, as class names if this turns out to be a classification dataset.
    let mut class_folders = BTreeSet::new();
    let mut images_in_class_folders = 0;

    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            let path = entry.path();
            let meta = entry.metadata()?;
            if meta.is_dir() {
                pending.push(path);
                continue;
            }
            if current == dir && entry.file_name() == DATASET_FILE {
                continue;
            }
            stats.size_bytes += meta.len();
            progress.add(1);
            if progress.is_cancelled() {
                return Err(io::Error::other(Cancelled));
            }

            let rel = path.strip_prefix(dir).unwrap_or(&path);
            let ext = extension(&path);
            if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
                stats.image_count += 1;
                stats.image_bytes += meta.len();
                match Split::of_path(rel) {
                    Some(Split::Train) => stats.train_count += 1,
                    Some(Split::Val) => stats.val_count += 1,
                    Some(Split::Test) => stats.test_count += 1,
                    None => {}
                }
                image_formats.insert(ext);
                if let Some(class) = class_folder(rel) {
                    class_folders.insert(class);
                    images_in_class_folders += 1;
                }
                continue;
            }
            match ext.as_str() {
                "json" => found.coco.push(path),
                "txt" if has_component(rel, "labels") => found.yolo_labels.push(path),
                "yaml" | "yml" => found.yaml.push(path),
                "xml" => found.voc.push(path),
                _ => {}
            }
        }
    }

    stats.image_format = image_formats.into_iter().collect::<Vec<_>>().join(",");
    let mut classes = ClassNames::default();

    if scan_coco(&found.coco, &mut stats, &mut classes)? {
        stats.annotation_format = "coco".into();
    } else if scan_via(&found.coco, &mut stats, &mut classes)? {
        stats.annotation_format = "via".into();
    } else if !found.yolo_labels.is_empty() || !found.yaml.is_empty() {
        stats.annotation_format = "yolo".into();
        scan_yolo(&found, &mut stats, &mut classes)?;
    } else if !found.voc.is_empty() {
        stats.annotation_format = "voc".into();
        scan_voc(&found.voc, &mut stats, &mut classes)?;
    } else if !class_folders.is_empty() {
        // No label files: one folder per class (ImageFolder layout).
        stats.annotation_format = FOLDERS_FORMAT.into();
        stats.dataset_type = "classification".into();
        stats.label_count = images_in_class_folders;
        class_folders.iter().for_each(|name| classes.push(name));
    } else {
        stats.annotation_format = "unknown".into();
        stats.dataset_type = "unknown".into();
    }

    stats.class_names = classes.names;
    Ok(stats)
}

/// Returns false when none of the json files is a COCO annotation file.
fn scan_coco(files: &[PathBuf], stats: &mut DatasetStats, classes: &mut ClassNames) -> io::Result<bool> {
    let mut is_coco = false;
    let mut segmentation = false;

    for path in files {
        let Ok(json) = serde_json::from_slice::<Value>(&fs::read(path)?) else { continue };
        let (Some(_), Some(annotations)) = (json["images"].as_array(), json["annotations"].as_array()) else {
            continue;
        };
        is_coco = true;
        stats.label_count += annotations.len() as u32;
        segmentation |= annotations
            .iter()
            .any(|a| a["segmentation"].as_array().is_some_and(|s| !s.is_empty()));

        let categories = json["categories"].as_array().map(Vec::as_slice).unwrap_or_default();
        for name in coco_class_names(categories) {
            classes.push(name);
        }
    }

    if is_coco {
        stats.dataset_type = detection_type(segmentation);
    }
    Ok(is_coco)
}

/// Returns false when none of the json files is a VIA (VGG Image Annotator) file.
fn scan_via(files: &[PathBuf], stats: &mut DatasetStats, classes: &mut ClassNames) -> io::Result<bool> {
    let mut is_via = false;
    let mut segmentation = false;
    for path in files {
        let Ok(json) = serde_json::from_slice::<Value>(&fs::read(path)?) else { continue };
        let Some(entries) = via_entries(&json) else { continue };
        is_via = true;
        for region in entries.values().flat_map(via_regions) {
            let (Some((_, polygon)), Some(class)) = (via_shape(region), via_class(region)) else { continue };
            stats.label_count += 1;
            segmentation |= polygon.is_some();
            classes.push(&class);
        }
    }
    if is_via {
        stats.dataset_type = detection_type(segmentation);
    }
    Ok(is_via)
}

// Roboflow adds a root category (supercategory "none") that every real class points to.
fn coco_class_names(categories: &[Value]) -> Vec<&str> {
    let is_root = |c: &Value| {
        let name = c["name"].as_str();
        c["supercategory"].as_str() == Some("none")
            && categories.iter().any(|o| o["supercategory"].as_str() == name && name.is_some())
    };
    categories
        .iter()
        .filter(|c| !is_root(c))
        .filter_map(|c| c["name"].as_str())
        .collect()
}

fn scan_yolo(found: &Annotations, stats: &mut DatasetStats, classes: &mut ClassNames) -> io::Result<()> {
    let mut segmentation = false;
    for path in &found.yolo_labels {
        let text = read_lossy(path)?;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            stats.label_count += 1;
            segmentation |= line.split_whitespace().count() > 5;
        }
    }

    // Prefer data.yaml, the file YOLO exports put class names in.
    let mut yamls: Vec<&PathBuf> = found.yaml.iter().collect();
    yamls.sort_by_key(|p| p.file_name().is_none_or(|n| n != "data.yaml"));
    for path in yamls {
        let names = parse_yaml_names(&read_lossy(path)?);
        if !names.is_empty() {
            names.iter().for_each(|name| classes.push(name));
            break;
        }
    }

    stats.dataset_type = detection_type(segmentation);
    Ok(())
}

fn scan_voc(files: &[PathBuf], stats: &mut DatasetStats, classes: &mut ClassNames) -> io::Result<()> {
    for path in files {
        let text = read_lossy(path)?;
        for object in text.split("<object>").skip(1) {
            stats.label_count += 1;
            if let Some(name) = xml_text(object, "name") {
                classes.push(name);
            }
        }
    }
    stats.dataset_type = detection_type(false);
    Ok(())
}

/// Reads the `names` key of a YOLO data.yaml: `[a, 'b']`, a `- a` list, or a `0: a` map.
pub(crate) fn parse_yaml_names(text: &str) -> Vec<String> {
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix("names:") else { continue };
        let rest = rest.trim();

        if rest.starts_with('[') {
            let mut flow = rest.to_string();
            while !flow.contains(']') {
                match lines.next() {
                    Some(more) => flow.push_str(more),
                    None => break,
                }
            }
            let inner = flow.trim_start_matches('[').split(']').next().unwrap_or_default();
            return inner.split(',').map(unquote).filter(|n| !n.is_empty()).collect();
        }
        if !rest.is_empty() && !rest.starts_with('#') {
            return Vec::new();
        }

        let mut names = Vec::new();
        for line in lines {
            let item = line.trim();
            if item.is_empty() || item.starts_with('#') {
                continue;
            }
            if !line.starts_with([' ', '\t', '-']) {
                break;
            }
            if let Some(value) = item.strip_prefix('-') {
                names.push(unquote(value));
            } else if let Some((_, value)) = item.split_once(':') {
                names.push(unquote(value));
            } else {
                break;
            }
        }
        return names;
    }
    Vec::new()
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    let quoted = value.len() >= 2
        && ((value.starts_with('\'') && value.ends_with('\'')) || (value.starts_with('"') && value.ends_with('"')));
    if !quoted {
        return value.to_string();
    }
    let inner = &value[1..value.len() - 1];
    // YAML escapes a quote inside single quotes by doubling it.
    if value.starts_with('\'') { inner.replace("''", "'") } else { inner.to_string() }
}

fn xml_text<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let start = xml.find(&format!("<{tag}>"))? + tag.len() + 2;
    let end = start + xml[start..].find(&format!("</{tag}>"))?;
    Some(xml[start..end].trim())
}

/// The class folder an image sits in: its parent folder, unless that is the
/// dataset root, a split folder or an `images` folder.
fn class_folder(rel: &Path) -> Option<String> {
    let parent = rel.parent()?.file_name()?.to_str()?;
    let not_a_class = Split::of_path(Path::new(parent)).is_some() || parent.eq_ignore_ascii_case("images");
    (!not_a_class).then(|| parent.to_string())
}

fn detection_type(segmentation: bool) -> String {
    if segmentation { "instance-segmentation" } else { "object-detection" }.into()
}

fn has_component(rel: &Path, name: &str) -> bool {
    rel.components().any(|c| c.as_os_str().eq_ignore_ascii_case(name))
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn read_lossy(path: &Path) -> io::Result<String> {
    Ok(String::from_utf8_lossy(&fs::read(path)?).into_owned())
}

/// Class names in first-seen order, without duplicates.
#[derive(Default)]
struct ClassNames {
    names: Vec<String>,
    seen: HashSet<String>,
}

impl ClassNames {
    fn push(&mut self, name: &str) {
        if !name.is_empty() && self.seen.insert(name.to_string()) {
            self.names.push(name.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::progress::Silent;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("modeloid-scan-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn write(&self, rel: &str, contents: &str) {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn scans_yolo_layout() {
        let tmp = TempDir::new("yolo");
        tmp.write("data.yaml", "train: ../train/images\nnc: 2\nnames: ['cat', 'dog']\n");
        tmp.write("train/images/a.jpg", "img");
        tmp.write("train/images/b.jpg", "img");
        tmp.write("train/labels/a.txt", "0 0.5 0.5 0.1 0.1\n1 0.2 0.2 0.1 0.1\n");
        tmp.write("train/labels/b.txt", "\n");
        tmp.write("valid/images/c.png", "img");
        tmp.write("valid/labels/c.txt", "1 0.5 0.5 0.1 0.1\n");
        tmp.write("dataset.toml", "ignored");

        let stats = scan_dataset(&tmp.0, &Silent).unwrap();
        assert_eq!(stats.annotation_format, "yolo");
        assert_eq!(stats.dataset_type, "object-detection");
        assert_eq!(stats.image_format, "jpg,png");
        assert_eq!((stats.image_count, stats.train_count, stats.val_count, stats.test_count), (3, 2, 1, 0));
        assert_eq!(stats.label_count, 3);
        assert_eq!(stats.class_names, ["cat", "dog"]);
    }

    #[test]
    fn scans_coco_layout() {
        let tmp = TempDir::new("coco");
        tmp.write(
            "train/_annotations.coco.json",
            r#"{
                "images": [{"id": 0}, {"id": 1}],
                "categories": [
                    {"id": 0, "name": "pets", "supercategory": "none"},
                    {"id": 1, "name": "cat", "supercategory": "pets"},
                    {"id": 2, "name": "dog", "supercategory": "pets"}
                ],
                "annotations": [
                    {"id": 0, "segmentation": []},
                    {"id": 1, "segmentation": [[0, 0, 1, 1, 1, 0]]}
                ]
            }"#,
        );
        tmp.write("train/a.jpg", "img");
        tmp.write("train/b.jpg", "img");
        tmp.write("test/c.jpg", "img");

        let stats = scan_dataset(&tmp.0, &Silent).unwrap();
        assert_eq!(stats.annotation_format, "coco");
        assert_eq!(stats.dataset_type, "instance-segmentation");
        assert_eq!((stats.image_count, stats.train_count, stats.test_count), (3, 2, 1));
        assert_eq!(stats.label_count, 2);
        assert_eq!(stats.class_names, ["cat", "dog"]);
    }

    #[test]
    fn scans_voc_layout() {
        let tmp = TempDir::new("voc");
        tmp.write(
            "train/a.xml",
            "<annotation><object><name>cat</name></object><object><name>dog</name></object></annotation>",
        );
        tmp.write("train/a.jpg", "img");

        let stats = scan_dataset(&tmp.0, &Silent).unwrap();
        assert_eq!(stats.annotation_format, "voc");
        assert_eq!(stats.label_count, 2);
        assert_eq!(stats.class_names, ["cat", "dog"]);
    }

    #[test]
    fn scans_class_folders() {
        let tmp = TempDir::new("folders");
        tmp.write("train/cat/a.jpg", "img");
        tmp.write("train/cat/b.jpg", "img");
        tmp.write("train/dog/c.jpg", "img");
        tmp.write("valid/dog/d.jpg", "img");

        let stats = scan_dataset(&tmp.0, &Silent).unwrap();
        assert_eq!((stats.annotation_format.as_str(), stats.dataset_type.as_str()), ("folders", "classification"));
        assert_eq!((stats.image_count, stats.label_count, stats.train_count, stats.val_count), (4, 4, 3, 1));
        assert_eq!(stats.class_names, ["cat", "dog"]);

        // Images in split or `images` folders only aren't classes.
        let plain = TempDir::new("plain");
        plain.write("train/images/a.jpg", "img");
        plain.write("b.jpg", "img");
        assert_eq!(scan_dataset(&plain.0, &Silent).unwrap().annotation_format, "unknown");
    }

    #[test]
    fn parses_yaml_names() {
        assert_eq!(parse_yaml_names("nc: 2\nnames: ['a', \"b\", c]\n"), ["a", "b", "c"]);
        assert_eq!(parse_yaml_names("names: [a,\n  b]\n"), ["a", "b"]);
        assert_eq!(parse_yaml_names("names:\n  - a\n  - 'b'\nnc: 2\n"), ["a", "b"]);
        assert_eq!(parse_yaml_names("names:\n- a\n- b\n"), ["a", "b"]);
        assert_eq!(parse_yaml_names("names:\n  0: a\n  1: b\npath: x\n"), ["a", "b"]);
        assert!(parse_yaml_names("nc: 0\n").is_empty());
        assert_eq!(parse_yaml_names("names: ['it''s']\n"), ["it's"]);
    }
}
