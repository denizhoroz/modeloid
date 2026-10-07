//! Labelling: changes to one image's labels and to a dataset's class list,
//! written straight into the dataset's own label files (no staging copy).
//! Each change also updates the `LabelSet` it was made on, so callers can
//! keep a read set instead of reading every label file again.

use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};

pub use modeloid_types::edit::validate_class_name;

use crate::core::{
    io as fsio,
    labels::{
        COCO_FILE, ImageLabels, LabelFormat, LabelSet, Object, VOC_CLASSES_FILE, coco_annotation, coco_categories,
        data_yaml, extension, read_lossy, voc_xml, walk, yolo_label_path, yolo_text,
    },
    scan::parse_yaml_names,
};
use crate::models::{dataset::Dataset, workspace::Workspace};

/// Labels of each class over all images.
pub fn class_totals(set: &LabelSet) -> Vec<u32> {
    let mut totals = vec![0; set.classes.len()];
    for object in set.images.iter().flat_map(|i| &i.objects) {
        if let Some(total) = totals.get_mut(object.class) {
            *total += 1;
        }
    }
    totals
}

impl Workspace {
    /// Dataset `id`'s folder, record and label format, for a labelling change.
    fn labelling(&self, id: u32) -> Result<(PathBuf, Dataset, LabelFormat), crate::Error> {
        let (dir, dataset) = self.find_dataset(id)?;
        let format = LabelFormat::parse(dataset.annotation_format())
            .filter(|f| *f != LabelFormat::Via)
            .ok_or_else(|| format!("'{}' labels can't be edited here; YOLO, COCO and Pascal VOC can. Convert the dataset first.", dataset.annotation_format()))?;
        Ok((dir, dataset, format))
    }

    /// Replaces the labels of image `index` of dataset `id` (read into `set`)
    /// with `objects` and writes its label file.
    pub fn save_image_labels(&self, id: u32, set: &mut LabelSet, index: usize, objects: Vec<Object>) -> Result<(), crate::Error> {
        let (dir, dataset, format) = self.labelling(id)?;
        let image = set.images.get(index).ok_or("that image isn't in the dataset any more")?;
        if let Some(bad) = objects.iter().find(|o| o.class >= set.classes.len()) {
            return Err(format!("class {} doesn't exist", bad.class).into());
        }
        let mut image = image.clone();
        image.objects = objects;

        match format {
            LabelFormat::Via => unreachable!("labelling() refuses VIA datasets"),
            LabelFormat::Yolo => {
                let path = yolo_label_path(&image.file);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, yolo_text(&image))?;
            }
            LabelFormat::Voc => fs::write(image.file.with_extension("xml"), voc_xml(set, &image, &image.name))?,
            LabelFormat::Coco => save_coco_image(&dir, set, &image)?,
        }
        set.images[index] = image;
        save_counts(&dir, dataset, set)
    }

    /// Adds a class called `name` to the end of dataset `id`'s class list; returns its index.
    pub fn add_label_class(&self, id: u32, set: &mut LabelSet, name: &str) -> Result<usize, crate::Error> {
        let (dir, dataset, format) = self.labelling(id)?;
        let name = validate_class_name(name, &set.classes, None)?;
        set.classes.push(name);
        if set.declared_classes.is_some() {
            set.declared_classes = Some(set.classes.len());
        }
        match format {
            LabelFormat::Via => unreachable!("labelling() refuses VIA datasets"),
            LabelFormat::Yolo => write_yolo_names(&dir, set)?,
            LabelFormat::Voc => write_voc_classes(&dir, set)?,
            LabelFormat::Coco => {
                let mut files = coco_files(&dir)?;
                if files.is_empty() {
                    // A class list needs a file to live in; images are added to it as they're labelled.
                    let folder = set.images.first().and_then(|i| i.file.parent()).unwrap_or(&dir).to_path_buf();
                    files.push((folder.join(COCO_FILE), json!({ "images": [], "annotations": [], "categories": [] })));
                }
                for (path, mut coco) in files {
                    category_ids(&mut coco, &set.classes);
                    write_json(&path, &coco)?;
                }
            }
        }
        save_counts(&dir, dataset, set)?;
        Ok(set.classes.len() - 1)
    }

    /// Renames class `class` of dataset `id` to `name`.
    pub fn rename_label_class(&self, id: u32, set: &mut LabelSet, class: usize, name: &str) -> Result<(), crate::Error> {
        let (dir, dataset, format) = self.labelling(id)?;
        let old = set.classes.get(class).ok_or("that class doesn't exist")?.clone();
        let name = validate_class_name(name, &set.classes, Some(class))?;
        set.classes[class] = name.clone();
        match format {
            LabelFormat::Via => unreachable!("labelling() refuses VIA datasets"),
            LabelFormat::Yolo => write_yolo_names(&dir, set)?,
            LabelFormat::Voc => {
                write_voc_classes(&dir, set)?;
                // VOC files name the class of each object.
                for image in set.images.iter().filter(|i| i.objects.iter().any(|o| o.class == class)) {
                    fs::write(image.file.with_extension("xml"), voc_xml(set, image, &image.name))?;
                }
            }
            LabelFormat::Coco => {
                for (path, mut coco) in coco_files(&dir)? {
                    let mut changed = false;
                    for category in coco["categories"].as_array_mut().into_iter().flatten() {
                        if category["name"].as_str() == Some(old.as_str()) {
                            category["name"] = json!(name);
                            changed = true;
                        }
                    }
                    if changed {
                        write_json(&path, &coco)?;
                    }
                }
            }
        }
        let mut dataset = dataset;
        dataset.rename_class_color(&old, &name);
        save_counts(&dir, dataset, set)
    }

    /// Removes `classes` from dataset `id` with all their labels; the classes
    /// after them move up (YOLO ids are rewritten). Returns how many labels went.
    pub fn remove_label_classes(&self, id: u32, set: &mut LabelSet, classes: &[usize]) -> Result<u32, crate::Error> {
        let (dir, dataset, format) = self.labelling(id)?;
        let removed: BTreeSet<usize> = classes.iter().copied().collect();
        if let Some(bad) = removed.iter().find(|&&c| c >= set.classes.len()) {
            return Err(format!("class {bad} doesn't exist").into());
        }
        let removed_names: Vec<String> = removed.iter().map(|&c| set.classes[c].clone()).collect();

        // Old index → new index, `None` for removed classes.
        let mut next = 0;
        let new_index: Vec<Option<usize>> = (0..set.classes.len())
            .map(|c| {
                if removed.contains(&c) {
                    None
                } else {
                    next += 1;
                    Some(next - 1)
                }
            })
            .collect();

        let mut dropped = 0;
        let mut changed = Vec::new();
        for (i, image) in set.images.iter_mut().enumerate() {
            let before = image.objects.len();
            let mut renumbered = false;
            image.objects.retain_mut(|o| match new_index.get(o.class).copied() {
                Some(None) => false,
                Some(Some(n)) => {
                    renumbered |= n != o.class;
                    o.class = n;
                    true
                }
                None => true,
            });
            dropped += (before - image.objects.len()) as u32;
            if renumbered || image.objects.len() != before {
                changed.push(i);
            }
        }
        set.classes = set.classes.iter().enumerate().filter(|(c, _)| !removed.contains(c)).map(|(_, n)| n.clone()).collect();
        if set.declared_classes.is_some() {
            set.declared_classes = Some(set.classes.len());
        }

        match format {
            LabelFormat::Via => unreachable!("labelling() refuses VIA datasets"),
            LabelFormat::Yolo => {
                for &i in &changed {
                    let image = &set.images[i];
                    fs::write(yolo_label_path(&image.file), yolo_text(image))?;
                }
                write_yolo_names(&dir, set)?;
            }
            LabelFormat::Voc => {
                for &i in &changed {
                    let image = &set.images[i];
                    fs::write(image.file.with_extension("xml"), voc_xml(set, image, &image.name))?;
                }
                write_voc_classes(&dir, set)?;
            }
            LabelFormat::Coco => {
                for (path, mut coco) in coco_files(&dir)? {
                    let gone: Vec<i64> = coco_categories(coco["categories"].as_array().map_or(&[][..], Vec::as_slice))
                        .into_iter()
                        .filter(|(_, name)| removed_names.contains(name))
                        .map(|(id, _)| id)
                        .collect();
                    if gone.is_empty() {
                        continue;
                    }
                    if let Some(categories) = coco["categories"].as_array_mut() {
                        categories.retain(|c| !c["id"].as_i64().is_some_and(|id| gone.contains(&id)));
                    }
                    if let Some(annotations) = coco["annotations"].as_array_mut() {
                        annotations.retain(|a| !a["category_id"].as_i64().is_some_and(|id| gone.contains(&id)));
                    }
                    write_json(&path, &coco)?;
                }
            }
        }
        save_counts(&dir, dataset, set)?;
        Ok(dropped)
    }
}

/// Updates `dataset.toml` with `set`'s label count and classes.
fn save_counts(dir: &Path, mut dataset: Dataset, set: &LabelSet) -> Result<(), crate::Error> {
    let labels = set.images.iter().map(|i| i.objects.len() as u32).sum();
    dataset.set_labels(labels, set.classes.clone());
    dataset.save_to_dir(dir)
}

// ---------- YOLO ----------

/// Writes the class list into the dataset's yaml with `names` (data.yaml
/// first), keeping its other keys; a dataset without one gets a data.yaml.
fn write_yolo_names(dir: &Path, set: &LabelSet) -> Result<(), crate::Error> {
    let files = walk(dir)?;
    let mut yamls: Vec<&PathBuf> = files.iter().filter(|p| matches!(extension(p).as_str(), "yaml" | "yml")).collect();
    yamls.sort_by_key(|p| p.file_name().is_none_or(|n| n != "data.yaml"));
    let mut named = None;
    for path in &yamls {
        if !parse_yaml_names(&read_lossy(path)?).is_empty() {
            named = Some(*path);
            break;
        }
    }
    let data_yaml_file = yamls.iter().find(|p| p.file_name().is_some_and(|n| n == "data.yaml"));
    match named.or_else(|| data_yaml_file.copied()) {
        Some(path) => fs::write(path, replace_yaml_names(&read_lossy(path)?, &set.classes))?,
        None => {
            let splits = set.images.iter().map(|i| i.split).collect();
            fs::write(dir.join("data.yaml"), data_yaml(&set.classes, &splits))?;
        }
    }
    Ok(())
}

/// `text` (a YOLO yaml) with its `nc` and `names` replaced by `classes`.
fn replace_yaml_names(text: &str, classes: &[String]) -> String {
    let mut kept: Vec<&str> = Vec::new();
    // Inside a `names:` value: a block list / map, or a flow list until `]`.
    let (mut in_block, mut in_flow) = (false, false);
    for line in text.lines() {
        if in_flow {
            in_flow = !line.contains(']');
            continue;
        }
        if in_block {
            if line.trim().is_empty() || line.starts_with([' ', '\t', '-']) {
                continue;
            }
            in_block = false;
        }
        if let Some(rest) = line.strip_prefix("names:") {
            let rest = rest.trim();
            in_flow = rest.starts_with('[') && !rest.contains(']');
            in_block = rest.is_empty() || rest.starts_with('#');
            continue;
        }
        if line.starts_with("nc:") {
            continue;
        }
        kept.push(line);
    }
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    let quoted: Vec<String> = classes.iter().map(|c| format!("'{}'", c.replace('\'', "''"))).collect();
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&format!("nc: {}\nnames: [{}]\n", classes.len(), quoted.join(", ")));
    out
}

// ---------- Pascal VOC ----------

fn write_voc_classes(dir: &Path, set: &LabelSet) -> Result<(), crate::Error> {
    let mut text = set.classes.join("\n");
    text.push('\n');
    fs::write(dir.join(VOC_CLASSES_FILE), text)?;
    Ok(())
}

// ---------- COCO ----------

/// Every COCO annotation file of the dataset in `dir`, parsed.
fn coco_files(dir: &Path) -> Result<Vec<(PathBuf, Value)>, crate::Error> {
    let mut files = Vec::new();
    for path in walk(dir)?.into_iter().filter(|p| extension(p) == "json") {
        let Ok(value) = serde_json::from_slice::<Value>(&fs::read(&path)?) else { continue };
        if value["images"].is_array() && value["annotations"].is_array() {
            files.push((path, value));
        }
    }
    Ok(files)
}

fn write_json(path: &Path, value: &Value) -> Result<(), crate::Error> {
    let text = serde_json::to_string(value)?;
    // Written next to it first, so a failed write can't leave half a file.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text)?;
    fsio::rename(&tmp, path)?;
    Ok(())
}

/// The category id of each of `classes` in `coco`, by name; missing classes
/// are added as new categories.
fn category_ids(coco: &mut Value, classes: &[String]) -> HashMap<usize, i64> {
    if !coco["categories"].is_array() {
        coco["categories"] = json!([]);
    }
    let known = coco_categories(coco["categories"].as_array().map_or(&[][..], Vec::as_slice));
    let mut next = coco["categories"].as_array().into_iter().flatten().filter_map(|c| c["id"].as_i64()).max().unwrap_or(0) + 1;
    let mut ids = HashMap::new();
    for (class, name) in classes.iter().enumerate() {
        let id = match known.iter().find(|(_, n)| n == name) {
            Some((id, _)) => *id,
            None => {
                coco["categories"].as_array_mut().expect("made an array above").push(json!({ "id": next, "name": name, "supercategory": "none" }));
                next += 1;
                next - 1
            }
        };
        ids.insert(class, id);
    }
    ids
}

/// Writes `image`'s labels into the COCO file that lists it, or, for an image
/// no file lists yet, into the `_annotations.coco.json` of its folder.
fn save_coco_image(dir: &Path, set: &LabelSet, image: &ImageLabels) -> Result<(), crate::Error> {
    let files = coco_files(dir)?;
    let lists = |path: &Path, coco: &Value| {
        let json_dir = path.parent().unwrap_or(dir);
        coco["images"].as_array().into_iter().flatten().any(|i| i["file_name"].as_str().is_some_and(|n| json_dir.join(n) == image.file))
    };
    let folder = image.file.parent().unwrap_or(dir);
    let (path, mut coco) = match files.iter().position(|(p, c)| lists(p, c)) {
        Some(found) => files.into_iter().nth(found).expect("found above"),
        None => {
            let path = folder.join(COCO_FILE);
            let coco = files
                .into_iter()
                .find(|(p, _)| *p == path)
                .map(|(_, c)| c)
                .unwrap_or_else(|| json!({ "images": [], "annotations": [], "categories": [] }));
            (path, coco)
        }
    };
    let json_dir = path.parent().unwrap_or(dir).to_path_buf();
    let ids = category_ids(&mut coco, &set.classes);

    let max_id = |list: &Value| list.as_array().into_iter().flatten().filter_map(|v| v["id"].as_i64()).max().unwrap_or(-1);
    let found = coco["images"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|i| i["file_name"].as_str().is_some_and(|n| json_dir.join(n) == image.file))
        .and_then(|i| i["id"].as_i64());
    let image_id = match found {
        Some(id) => id,
        None => {
            let id = max_id(&coco["images"]) + 1;
            let rel = image.file.strip_prefix(&json_dir).unwrap_or(&image.file);
            let file_name = rel.to_string_lossy().replace('\\', "/");
            coco["images"].as_array_mut().ok_or("COCO file without images")?.push(json!({
                "id": id,
                "file_name": file_name,
                "width": image.width,
                "height": image.height,
            }));
            id
        }
    };

    let mut next_id = max_id(&coco["annotations"]) + 1;
    let annotations = coco["annotations"].as_array_mut().ok_or("COCO file without annotations")?;
    annotations.retain(|a| a["image_id"].as_i64() != Some(image_id));
    for object in &image.objects {
        annotations.push(coco_annotation(object, next_id, image_id, ids[&object.class]));
        next_id += 1;
    }
    write_json(&path, &coco)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        labels::{BBox, read},
        progress::Silent,
    };
    use crate::models::workspace::ImportMode;
    use modeloid_types::edit::NewDataset;

    /// Just enough PNG for a header-only size read.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    /// A workspace with a created dataset in `format` holding a.png and b.png (100×50).
    fn dataset(name: &str, format: &str) -> Workspace {
        let dir = std::env::temp_dir().join(format!("modeloid-annotate-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let ws = Workspace::new(name.into(), dir);
        ws.make_to_dir().unwrap();
        let new = NewDataset {
            name: "d".into(),
            description: String::new(),
            dataset_type: "object-detection".into(),
            annotation_format: format.into(),
        };
        ws.create_dataset(&new).unwrap();
        let source = ws.path().join("source");
        fs::create_dir_all(&source).unwrap();
        let files: Vec<PathBuf> = ["a.png", "b.png"].iter().map(|n| source.join(n)).collect();
        for file in &files {
            fs::write(file, png(100, 50)).unwrap();
        }
        ws.add_images(1, &files, ImportMode::Move, &Silent).unwrap();
        ws
    }

    fn reread(ws: &Workspace) -> LabelSet {
        ws.read_labels(1, &Silent).unwrap().2
    }

    fn object(class: usize, x: f64) -> Object {
        Object { class, bbox: BBox { x, y: 10.0, w: 20.0, h: 30.0 }, polygon: None }
    }

    #[test]
    fn labels_and_edits_classes_in_every_format() {
        for format in ["yolo", "coco", "voc"] {
            let ws = dataset(&format!("all-{format}"), format);
            let mut set = reread(&ws);
            assert_eq!(set.images.len(), 2, "{format}");

            assert_eq!(ws.add_label_class(1, &mut set, "cat").unwrap(), 0);
            assert_eq!(ws.add_label_class(1, &mut set, "dog").unwrap(), 1);
            assert!(ws.add_label_class(1, &mut set, "Cat").is_err(), "{format}: taken");
            ws.add_label_class(1, &mut set, "bird").unwrap();
            assert_eq!(reread(&ws).classes, ["cat", "dog", "bird"], "{format}: unused classes are kept");

            ws.save_image_labels(1, &mut set, 0, vec![object(0, 5.0), object(2, 40.0)]).unwrap();
            ws.save_image_labels(1, &mut set, 1, vec![object(1, 1.0)]).unwrap();
            assert!(ws.save_image_labels(1, &mut set, 1, vec![object(7, 1.0)]).is_err(), "unknown class");
            ws.save_image_labels(1, &mut set, 1, vec![object(1, 1.0), object(2, 60.0)]).unwrap();

            let read_back = reread(&ws);
            assert_eq!(read_back.images[0].objects.len(), 2, "{format}");
            let b = read_back.images[1].objects[1].bbox;
            assert!((b.x - 60.0).abs() < 1e-3 && (b.w - 20.0).abs() < 1e-3 && (b.h - 30.0).abs() < 1e-3, "{format}: {b:?}");
            assert_eq!(read_back.images[1].objects[1].class, 2);
            assert_eq!(class_totals(&read_back), [1, 1, 2], "{format}");
            assert_eq!(ws.find_dataset(1).unwrap().1.label_count(), 4);

            ws.rename_label_class(1, &mut set, 2, "parrot").unwrap();
            assert_eq!(reread(&ws).classes, ["cat", "dog", "parrot"], "{format}");

            // Removing "cat" moves dog and parrot up and drops cat's label.
            assert_eq!(ws.remove_label_classes(1, &mut set, &[0]).unwrap(), 1);
            let read_back = reread(&ws);
            assert_eq!(read_back.classes, ["dog", "parrot"], "{format}");
            assert_eq!(read_back.images[0].objects.iter().map(|o| o.class).collect::<Vec<_>>(), [1], "{format}");
            assert_eq!(read_back.images[1].objects.iter().map(|o| o.class).collect::<Vec<_>>(), [0, 1], "{format}");
            assert_eq!(set.classes, read_back.classes);
            assert_eq!(set.images[1].objects.iter().map(|o| o.class).collect::<Vec<_>>(), [0, 1], "{format}: set follows");
            assert_eq!(ws.find_dataset(1).unwrap().1.class_names(), ["dog", "parrot"]);

            // Clearing an image leaves it without labels.
            ws.save_image_labels(1, &mut set, 0, Vec::new()).unwrap();
            assert!(reread(&ws).images[0].objects.is_empty(), "{format}");
            let _ = fs::remove_dir_all(ws.path());
        }
    }

    #[test]
    fn keeps_other_yaml_keys() {
        let text = "path: x\ntrain: train/images\nnc: 2\nnames:\n  - a\n  - b\ntest: t\n";
        let out = replace_yaml_names(text, &["a".into(), "it's".into(), "c".into()]);
        assert_eq!(out, "path: x\ntrain: train/images\ntest: t\n\nnc: 3\nnames: ['a', 'it''s', 'c']\n");
        assert_eq!(parse_yaml_names(&out), ["a", "it's", "c"]);
        let flow = replace_yaml_names("names: [a,\n  b]\nval: v\n", &["z".into()]);
        assert_eq!(flow, "val: v\n\nnc: 1\nnames: ['z']\n");
    }

    #[test]
    fn coco_keeps_other_files_and_ids() {
        let ws = dataset("coco-existing", "coco");
        let dir = ws.find_dataset(1).unwrap().0;
        // An existing file listing a.png under its own category ids.
        fs::write(
            dir.join("images").join(COCO_FILE),
            r#"{"info": {"x": 1}, "images": [{"id": 7, "file_name": "a.png", "width": 100, "height": 50}],
                "categories": [{"id": 3, "name": "cat", "supercategory": "none"}],
                "annotations": [{"id": 9, "image_id": 7, "category_id": 3, "bbox": [1, 2, 3, 4]}]}"#,
        )
        .unwrap();
        let mut set = read(&dir, LabelFormat::Coco, &Silent).unwrap();
        assert_eq!(set.images.len(), 2);
        let a = set.images.iter().position(|i| i.name == "a.png").unwrap();
        let b = set.images.iter().position(|i| i.name == "b.png").unwrap();
        ws.save_image_labels(1, &mut set, b, vec![object(0, 1.0)]).unwrap();
        ws.save_image_labels(1, &mut set, a, vec![object(0, 2.0), object(0, 3.0)]).unwrap();

        let coco: Value = serde_json::from_slice(&fs::read(dir.join("images").join(COCO_FILE)).unwrap()).unwrap();
        assert_eq!(coco["info"]["x"], 1, "other keys stay");
        assert_eq!(coco["images"].as_array().unwrap().len(), 2, "b.png added");
        assert!(coco["annotations"].as_array().unwrap().iter().all(|a| a["category_id"] == 3));
        let read_back = read(&dir, LabelFormat::Coco, &Silent).unwrap();
        assert_eq!(read_back.images[a].objects.len(), 2);
        assert_eq!(read_back.images[b].objects.len(), 1);
        let _ = fs::remove_dir_all(ws.path());
    }
}
