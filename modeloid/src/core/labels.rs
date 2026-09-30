//! Reads and writes object labels in YOLO, COCO and Pascal VOC layouts, so every
//! dataset tool works on one in-memory form (`LabelSet`) instead of on files.
//!
//! Coordinates are in pixels. Reading YOLO needs each image's size, which is taken
//! from the image header. Problems that only affect one image or line (unreadable
//! image, bad label line) are collected in `LabelSet::warnings` instead of failing.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering::Relaxed},
    },
};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::{
    io::run_parallel,
    progress::{Cancelled, Progress, check_cancel},
    scan::{DatasetStats, IMAGE_EXTENSIONS, parse_yaml_names},
};

/// Name of the COCO annotation file in each split folder (Roboflow's layout).
pub const COCO_FILE: &str = "_annotations.coco.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelFormat {
    Yolo,
    Coco,
    Voc,
}

impl LabelFormat {
    /// Accepts the names used in `dataset.toml` and by Roboflow exports,
    /// e.g. "yolov8", "coco-segmentation", "voc".
    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        if name.starts_with("yolo") {
            Some(LabelFormat::Yolo)
        } else if name.starts_with("coco") {
            Some(LabelFormat::Coco)
        } else if name.starts_with("voc") || name == "pascal-voc" {
            Some(LabelFormat::Voc)
        } else {
            None
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            LabelFormat::Yolo => "yolo",
            LabelFormat::Coco => "coco",
            LabelFormat::Voc => "voc",
        }
    }
}

pub use modeloid_types::Split;

/// Axis-aligned box in pixels: top-left corner, width and height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl BBox {
    pub fn area(&self) -> f64 {
        self.w * self.h
    }

    fn around(points: &[[f64; 2]]) -> BBox {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for [x, y] in points {
            x0 = x0.min(*x);
            y0 = y0.min(*y);
            x1 = x1.max(*x);
            y1 = y1.max(*y);
        }
        BBox { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    /// Index into `LabelSet::classes`.
    pub class: usize,
    pub bbox: BBox,
    /// Segmentation outline in pixels, if the dataset has one.
    pub polygon: Option<Vec<[f64; 2]>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageLabels {
    /// Where the image is now. Writing copies it from here.
    pub file: PathBuf,
    /// File name to write it as (made unique per split when writing).
    pub name: String,
    pub split: Option<Split>,
    pub width: u32,
    pub height: u32,
    pub objects: Vec<Object>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LabelSet {
    pub classes: Vec<String>,
    pub images: Vec<ImageLabels>,
    pub warnings: Vec<String>,
    /// How many classes the dataset's own class list names (YOLO `data.yaml`).
    /// Ids at or above it are unknown classes. `None` when there's no list to
    /// check against (COCO and VOC only produce named classes; YOLO without `data.yaml`).
    pub declared_classes: Option<usize>,
}

impl LabelSet {
    /// Index of `name` in `classes`, adding it if missing.
    pub fn class_index(&mut self, name: &str) -> usize {
        match self.classes.iter().position(|c| c == name) {
            Some(i) => i,
            None => {
                self.classes.push(name.to_string());
                self.classes.len() - 1
            }
        }
    }

    pub fn has_polygons(&self) -> bool {
        self.images.iter().flat_map(|i| &i.objects).any(|o| o.polygon.is_some())
    }
}

/// Reads the labels of the dataset in `dir`, counting each image on `progress`.
pub fn read(dir: &Path, format: LabelFormat, progress: &dyn Progress) -> io::Result<LabelSet> {
    match format {
        LabelFormat::Yolo => read_yolo(dir, progress),
        LabelFormat::Coco => read_coco(dir, progress),
        LabelFormat::Voc => read_voc(dir, progress),
    }
}

/// What `write` put on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Written {
    /// Size of every file written: images, label files, `data.yaml`.
    pub bytes: u64,
}

/// Writes `set` into `dir` (created if missing): images are copied and labels
/// written in `format`'s layout, one folder per split. Files are copied on
/// the worker threads.
pub fn write(set: &LabelSet, dir: &Path, format: LabelFormat, progress: &dyn Progress) -> io::Result<Written> {
    progress.set_total(set.images.len() as u64);
    fs::create_dir_all(dir)?;
    match format {
        LabelFormat::Yolo => write_yolo(set, dir, progress),
        LabelFormat::Coco => write_coco(set, dir, progress),
        LabelFormat::Voc => write_voc(set, dir, progress),
    }
}

/// What `scan_dataset` would find in the folder `set` was just written into
/// in `format`, without reading the folder again. `None` when there are no
/// images to go by; scan the folder then.
pub fn describe(set: &LabelSet, format: LabelFormat, written: Written) -> Option<DatasetStats> {
    // The scanner counts files with an image extension.
    let images: Vec<&ImageLabels> = set.images.iter().filter(|i| is_image(Path::new(&i.name))).collect();
    if images.is_empty() {
        return None;
    }
    let in_split = |split: Split| images.iter().filter(|i| i.split == Some(split)).count() as u32;
    let formats: BTreeSet<String> = images.iter().map(|i| extension(Path::new(&i.name))).collect();
    // VOC files name the class of each object, so only classes in use are found.
    let used: HashSet<usize> = set.images.iter().flat_map(|i| &i.objects).map(|o| o.class).collect();
    let mut seen = HashSet::new();
    let class_names = set
        .classes
        .iter()
        .enumerate()
        .filter(|(id, name)| !name.is_empty() && (format != LabelFormat::Voc || used.contains(id)))
        .filter(|(_, name)| seen.insert(name.as_str()))
        .map(|(_, name)| name.clone())
        .collect();
    let outlines = format != LabelFormat::Voc && set.has_polygons();

    Some(DatasetStats {
        dataset_type: if outlines { "instance-segmentation" } else { "object-detection" }.into(),
        image_format: formats.into_iter().collect::<Vec<_>>().join(","),
        annotation_format: format.name().into(),
        image_count: images.len() as u32,
        label_count: set.images.iter().map(|i| i.objects.len() as u32).sum(),
        class_names,
        size_bytes: written.bytes,
        train_count: in_split(Split::Train),
        test_count: in_split(Split::Test),
        val_count: in_split(Split::Val),
    })
}

/// `io::run_parallel` for label files: `work` runs over `items` on the worker
/// threads, counting each on `progress`. A cancel comes back as `Interrupted`.
fn in_parallel<T: Sync>(items: &[T], progress: &dyn Progress, work: impl Fn(&T) -> io::Result<()> + Sync) -> io::Result<()> {
    run_parallel(items, progress, || Ok(()), |_, item| Ok(work(item)?)).map_err(|e| {
        if e.downcast_ref::<Cancelled>().is_some() {
            return io::Error::new(io::ErrorKind::Interrupted, e);
        }
        match e.downcast::<io::Error>() {
            Ok(error) => *error,
            Err(other) => io::Error::other(other),
        }
    })
}

/// Reads every file of `files` with `read` on the worker threads. Results
/// come back in the order of `files`, each with the warnings it raised.
fn read_each<T: Send>(
    files: &[&PathBuf],
    progress: &dyn Progress,
    read: impl Fn(&Path, &mut Vec<String>) -> io::Result<T> + Sync,
) -> io::Result<Vec<(T, Vec<String>)>> {
    let slots: Vec<Mutex<Option<(T, Vec<String>)>>> = files.iter().map(|_| Mutex::new(None)).collect();
    let indexes: Vec<usize> = (0..files.len()).collect();
    in_parallel(&indexes, progress, |&i| {
        let mut warnings = Vec::new();
        let value = read(files[i], &mut warnings)?;
        *slots[i].lock().unwrap() = Some((value, warnings));
        Ok(())
    })?;
    Ok(slots.into_iter().filter_map(|slot| slot.into_inner().unwrap()).collect())
}

/// One image to copy into the output, with its label file if it has its own.
struct FileJob<'a> {
    from: &'a Path,
    to: PathBuf,
    label: Option<(PathBuf, String)>,
}

/// Copies the images (and writes their label files) on the worker threads.
/// Returns the bytes written.
fn write_files(jobs: &[FileJob], progress: &dyn Progress) -> io::Result<u64> {
    let bytes = AtomicU64::new(0);
    in_parallel(jobs, progress, |job| {
        let mut written = fs::copy(job.from, &job.to)?;
        if let Some((path, text)) = &job.label {
            fs::write(path, text)?;
            written += text.len() as u64;
        }
        bytes.fetch_add(written, Relaxed);
        Ok(())
    })?;
    Ok(bytes.into_inner())
}

// ---------- YOLO ----------

fn read_yolo(dir: &Path, progress: &dyn Progress) -> io::Result<LabelSet> {
    let mut set = LabelSet::default();
    let files = walk(dir)?;

    // Class names from data.yaml (preferred) or any other yaml with `names`.
    let mut yamls: Vec<&PathBuf> = files.iter().filter(|p| matches!(extension(p).as_str(), "yaml" | "yml")).collect();
    yamls.sort_by_key(|p| p.file_name().is_none_or(|n| n != "data.yaml"));
    for path in yamls {
        let names = parse_yaml_names(&read_lossy(path)?);
        if !names.is_empty() {
            set.declared_classes = Some(names.len());
            set.classes = names;
            break;
        }
    }

    // Each image's size (from its header) and label file, read on the worker threads.
    let images: Vec<&PathBuf> = files.iter().filter(|p| is_image(p)).collect();
    let read = read_each(&images, progress, |file, warnings| {
        let rel = file.strip_prefix(dir).unwrap_or(file);
        let Some((width, height)) = image_size(file, warnings) else { return Ok(None) };
        let mut image = new_image(file, rel, width, height);

        let label_path = yolo_label_path(file);
        if label_path.is_file() {
            for (n, line) in read_lossy(&label_path)?.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                match parse_yolo_line(line, width as f64, height as f64) {
                    Some(object) => image.objects.push(object),
                    None => warnings.push(format!("{}:{}: invalid label line", label_path.display(), n + 1)),
                }
            }
        }
        Ok(Some(image))
    })?;
    for (image, warnings) in read {
        set.warnings.extend(warnings);
        set.images.extend(image);
    }

    // No names found: number the classes that appear.
    let max_class = set.images.iter().flat_map(|i| &i.objects).map(|o| o.class).max();
    if let Some(max) = max_class {
        for i in set.classes.len()..=max {
            set.classes.push(i.to_string());
        }
    }
    Ok(set)
}

/// `.../images/x.jpg` → `.../labels/x.txt`; without an `images` folder, `x.txt` next to the image.
fn yolo_label_path(image: &Path) -> PathBuf {
    let parts: Vec<_> = image.components().collect();
    let images_at = parts.iter().rposition(|c| c.as_os_str().eq_ignore_ascii_case("images"));
    let path = match images_at {
        Some(i) => {
            let mut path = PathBuf::new();
            for (j, part) in parts.iter().enumerate() {
                path.push(if j == i { "labels".as_ref() } else { part.as_os_str() });
            }
            path
        }
        None => image.to_path_buf(),
    };
    path.with_extension("txt")
}

/// `class cx cy w h` or `class x1 y1 x2 y2 ...` (polygon), normalized to 0..1.
fn parse_yolo_line(line: &str, width: f64, height: f64) -> Option<Object> {
    let mut tokens = line.split_whitespace();
    let class: usize = tokens.next()?.parse().ok()?;
    let values: Vec<f64> = tokens.map(str::parse).collect::<Result<_, _>>().ok()?;

    if values.len() == 4 {
        let [cx, cy, w, h] = [values[0], values[1], values[2], values[3]];
        let bbox = BBox { x: (cx - w / 2.0) * width, y: (cy - h / 2.0) * height, w: w * width, h: h * height };
        return Some(Object { class, bbox, polygon: None });
    }
    if values.len() >= 6 && values.len() % 2 == 0 {
        let points: Vec<[f64; 2]> = values.chunks(2).map(|p| [p[0] * width, p[1] * height]).collect();
        return Some(Object { class, bbox: BBox::around(&points), polygon: Some(points) });
    }
    None
}

fn write_yolo(set: &LabelSet, dir: &Path, progress: &dyn Progress) -> io::Result<Written> {
    let mut names = UniqueNames::default();
    let mut splits = BTreeSet::new();
    // Names, folders and label text first, in order; the files are then written on every thread.
    let mut jobs = Vec::with_capacity(set.images.len());

    for image in &set.images {
        let split_dir = split_dir(dir, image.split);
        let (images_dir, labels_dir) = (split_dir.join("images"), split_dir.join("labels"));
        if splits.insert(image.split) {
            fs::create_dir_all(&images_dir)?;
            fs::create_dir_all(&labels_dir)?;
        }

        let name = names.take(image.split, &image.name);

        let (w, h) = (image.width.max(1) as f64, image.height.max(1) as f64);
        let mut text = String::new();
        for object in &image.objects {
            text.push_str(&object.class.to_string());
            match &object.polygon {
                Some(points) => {
                    for [x, y] in points {
                        text.push_str(&format!(" {} {}", num(x / w), num(y / h)));
                    }
                }
                None => {
                    let b = object.bbox;
                    let (cx, cy) = ((b.x + b.w / 2.0) / w, (b.y + b.h / 2.0) / h);
                    text.push_str(&format!(" {} {} {} {}", num(cx), num(cy), num(b.w / w), num(b.h / h)));
                }
            }
            text.push('\n');
        }
        let label = labels_dir.join(Path::new(&name).with_extension("txt"));
        jobs.push(FileJob { from: &image.file, to: images_dir.join(&name), label: Some((label, text)) });
    }

    let yaml = data_yaml(&set.classes, &splits);
    fs::write(dir.join("data.yaml"), &yaml)?;
    Ok(Written { bytes: write_files(&jobs, progress)? + yaml.len() as u64 })
}

/// Ultralytics `data.yaml`. Split paths are relative to the yaml's folder.
pub fn data_yaml(classes: &[String], splits: &BTreeSet<Option<Split>>) -> String {
    let path = |split: Option<Split>| match split {
        Some(s) => format!("{}/images", s.dir_name()),
        None => "images".to_string(),
    };
    let unsplit = splits.contains(&None);
    let pick = |s: Split| splits.contains(&Some(s)).then(|| path(Some(s)));
    let train = pick(Split::Train).or(unsplit.then(|| path(None)));
    // Ultralytics needs a val entry; fall back to train images.
    let val = pick(Split::Val).or_else(|| train.clone());

    let mut yaml = String::new();
    if let Some(train) = train {
        yaml.push_str(&format!("train: {train}\n"));
    }
    if let Some(val) = val {
        yaml.push_str(&format!("val: {val}\n"));
    }
    if let Some(test) = pick(Split::Test) {
        yaml.push_str(&format!("test: {test}\n"));
    }
    yaml.push_str(&format!("\nnc: {}\n", classes.len()));
    let quoted: Vec<String> = classes.iter().map(|c| format!("'{}'", c.replace('\'', "''"))).collect();
    yaml.push_str(&format!("names: [{}]\n", quoted.join(", ")));
    yaml
}

// ---------- COCO ----------

#[derive(Deserialize)]
struct CocoFile {
    images: Vec<CocoImage>,
    annotations: Vec<CocoAnnotation>,
    #[serde(default)]
    categories: Vec<Value>,
}

#[derive(Deserialize)]
struct CocoImage {
    id: i64,
    file_name: String,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

#[derive(Deserialize)]
struct CocoAnnotation {
    image_id: i64,
    category_id: i64,
    #[serde(default)]
    bbox: Vec<f64>,
    #[serde(default)]
    segmentation: Value,
}

fn read_coco(dir: &Path, progress: &dyn Progress) -> io::Result<LabelSet> {
    let mut set = LabelSet::default();

    for path in walk(dir)?.iter().filter(|p| extension(p) == "json") {
        let Ok(coco) = serde_json::from_slice::<CocoFile>(&fs::read(path)?) else { continue };
        let json_dir = path.parent().unwrap_or(dir);

        let categories = coco_categories(&coco.categories);
        let class_of: HashMap<i64, usize> = categories.iter().map(|(id, name)| (*id, set.class_index(name))).collect();

        let mut index_of = HashMap::new();
        for img in &coco.images {
            tick(progress)?;
            let file = json_dir.join(&img.file_name);
            let (width, height) = if img.width > 0 && img.height > 0 {
                (img.width, img.height)
            } else {
                match image_size(&file, &mut set.warnings) {
                    Some(size) => size,
                    None => continue,
                }
            };
            let rel = file.strip_prefix(dir).unwrap_or(&file);
            index_of.insert(img.id, set.images.len());
            set.images.push(new_image(&file, rel, width, height));
        }

        for ann in &coco.annotations {
            let Some(&image) = index_of.get(&ann.image_id) else { continue };
            let Some(&class) = class_of.get(&ann.category_id) else {
                set.warnings.push(format!("{}: annotation with unknown category {} skipped", path.display(), ann.category_id));
                continue;
            };
            let polygon = largest_polygon(&ann.segmentation);
            let bbox = match (ann.bbox.as_slice(), &polygon) {
                (&[x, y, w, h], _) => BBox { x, y, w, h },
                (_, Some(points)) => BBox::around(points),
                _ => {
                    set.warnings.push(format!("{}: annotation without bbox skipped", path.display()));
                    continue;
                }
            };
            set.images[image].objects.push(Object { class, bbox, polygon });
        }
    }
    Ok(set)
}

/// (id, name) of real classes. Roboflow adds a root category (supercategory
/// "none") that every real class points to; it's left out.
fn coco_categories(categories: &[Value]) -> Vec<(i64, String)> {
    let is_root = |c: &Value| {
        let name = c["name"].as_str();
        c["supercategory"].as_str() == Some("none")
            && name.is_some()
            && categories.iter().any(|o| o["supercategory"].as_str() == name)
    };
    categories
        .iter()
        .filter(|c| !is_root(c))
        .filter_map(|c| Some((c["id"].as_i64()?, c["name"].as_str()?.to_string())))
        .collect()
}

/// COCO polygons are `[[x1, y1, x2, y2, ...], ...]`; only the largest part is kept.
/// RLE masks (objects) give `None`.
fn largest_polygon(segmentation: &Value) -> Option<Vec<[f64; 2]>> {
    let parts = segmentation.as_array()?;
    let part = parts.iter().filter_map(Value::as_array).max_by_key(|p| p.len())?;
    let values: Vec<f64> = part.iter().filter_map(Value::as_f64).collect();
    (values.len() >= 6).then(|| values.chunks_exact(2).map(|p| [p[0], p[1]]).collect())
}

fn write_coco(set: &LabelSet, dir: &Path, progress: &dyn Progress) -> io::Result<Written> {
    let mut names = UniqueNames::default();
    let mut splits: HashMap<Option<Split>, (Vec<Value>, Vec<Value>)> = HashMap::new();
    let mut annotation_id = 0;
    let mut jobs = Vec::with_capacity(set.images.len());

    for (image_id, image) in set.images.iter().enumerate() {
        let split_dir = split_dir(dir, image.split);
        if !splits.contains_key(&image.split) {
            fs::create_dir_all(&split_dir)?;
        }
        let name = names.take(image.split, &image.name);
        jobs.push(FileJob { from: &image.file, to: split_dir.join(&name), label: None });

        let (images, annotations) = splits.entry(image.split).or_default();
        images.push(json!({ "id": image_id, "file_name": name, "width": image.width, "height": image.height }));
        for object in &image.objects {
            let b = object.bbox;
            let (segmentation, area) = match &object.polygon {
                Some(points) => (json!([points.iter().flatten().collect::<Vec<_>>()]), polygon_area(points)),
                None => (json!([]), b.area()),
            };
            annotations.push(json!({
                "id": annotation_id,
                "image_id": image_id,
                "category_id": object.class + 1,
                "bbox": [b.x, b.y, b.w, b.h],
                "area": area,
                "segmentation": segmentation,
                "iscrowd": 0,
            }));
            annotation_id += 1;
        }
    }
    let mut bytes = write_files(&jobs, progress)?;

    let categories: Vec<Value> = set
        .classes
        .iter()
        .enumerate()
        .map(|(i, name)| json!({ "id": i + 1, "name": name, "supercategory": "none" }))
        .collect();
    for (split, (images, annotations)) in splits {
        let file = json!({ "images": images, "annotations": annotations, "categories": categories });
        let text = serde_json::to_string(&file).map_err(io::Error::other)?;
        bytes += text.len() as u64;
        fs::write(split_dir(dir, split).join(COCO_FILE), text)?;
    }
    Ok(Written { bytes })
}

fn polygon_area(points: &[[f64; 2]]) -> f64 {
    let n = points.len();
    let twice: f64 = (0..n).map(|i| points[i][0] * points[(i + 1) % n][1] - points[(i + 1) % n][0] * points[i][1]).sum();
    twice.abs() / 2.0
}

// ---------- Pascal VOC ----------

fn read_voc(dir: &Path, progress: &dyn Progress) -> io::Result<LabelSet> {
    let mut set = LabelSet::default();

    let files = walk(dir)?;
    let images: Vec<&PathBuf> = files.iter().filter(|p| is_image(p)).collect();
    // Each image with its objects as (class name, box); class ids are given
    // afterwards, in image order, so they don't depend on thread timing.
    let read = read_each(&images, progress, |file, warnings| {
        let rel = file.strip_prefix(dir).unwrap_or(file);
        let xml_path = file.with_extension("xml");
        let xml = if xml_path.is_file() { Some(read_lossy(&xml_path)?) } else { None };

        let size = xml.as_deref().and_then(|x| {
            let size = xml_text(x, "size")?;
            Some((xml_text(size, "width")?.parse().ok()?, xml_text(size, "height")?.parse().ok()?))
        });
        let (width, height) = match size.filter(|&(w, h): &(u32, u32)| w > 0 && h > 0) {
            Some(size) => size,
            None => match image_size(file, warnings) {
                Some(size) => size,
                None => return Ok(None),
            },
        };
        let image = new_image(file, rel, width, height);
        let mut objects = Vec::new();

        for object in xml.as_deref().unwrap_or_default().split("<object>").skip(1) {
            let parsed = (|| {
                let name = xml_text(object, "name")?;
                let bndbox = xml_text(object, "bndbox")?;
                let v = |tag| xml_text(bndbox, tag)?.parse::<f64>().ok();
                let (x0, y0, x1, y1) = (v("xmin")?, v("ymin")?, v("xmax")?, v("ymax")?);
                Some((unescape_xml(name), BBox { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }))
            })();
            match parsed {
                Some(found) => objects.push(found),
                None => warnings.push(format!("{}: invalid <object> skipped", xml_path.display())),
            }
        }
        Ok(Some((image, objects)))
    })?;
    for (found, warnings) in read {
        set.warnings.extend(warnings);
        let Some((mut image, objects)) = found else { continue };
        for (name, bbox) in objects {
            let class = set.class_index(&name);
            image.objects.push(Object { class, bbox, polygon: None });
        }
        set.images.push(image);
    }
    Ok(set)
}

fn write_voc(set: &LabelSet, dir: &Path, progress: &dyn Progress) -> io::Result<Written> {
    let mut names = UniqueNames::default();
    let mut splits = HashSet::new();
    let mut jobs = Vec::with_capacity(set.images.len());

    for image in &set.images {
        let split_dir = split_dir(dir, image.split);
        if splits.insert(image.split) {
            fs::create_dir_all(&split_dir)?;
        }
        let name = names.take(image.split, &image.name);

        let mut xml = format!(
            "<annotation>\n\t<filename>{}</filename>\n\t<size>\n\t\t<width>{}</width>\n\t\t<height>{}</height>\n\t\t<depth>3</depth>\n\t</size>\n",
            escape_xml(&name),
            image.width,
            image.height
        );
        for object in &image.objects {
            let b = object.bbox;
            let class = set.classes.get(object.class).map_or("unknown", String::as_str);
            xml.push_str(&format!(
                "\t<object>\n\t\t<name>{}</name>\n\t\t<bndbox>\n\t\t\t<xmin>{}</xmin>\n\t\t\t<ymin>{}</ymin>\n\t\t\t<xmax>{}</xmax>\n\t\t\t<ymax>{}</ymax>\n\t\t</bndbox>\n\t</object>\n",
                escape_xml(class),
                num(b.x),
                num(b.y),
                num(b.x + b.w),
                num(b.y + b.h)
            ));
        }
        xml.push_str("</annotation>\n");
        let label = split_dir.join(Path::new(&name).with_extension("xml"));
        jobs.push(FileJob { from: &image.file, to: split_dir.join(&name), label: Some((label, xml)) });
    }
    Ok(Written { bytes: write_files(&jobs, progress)? })
}

/// Text inside the first `<tag>...</tag>`. Enough for VOC files, not a general XML parser.
pub(crate) fn xml_text<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let start = xml.find(&format!("<{tag}>"))? + tag.len() + 2;
    let end = start + xml[start..].find(&format!("</{tag}>"))?;
    Some(xml[start..end].trim())
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn unescape_xml(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

// ---------- label files without images ----------

/// Label files (or COCO image entries) whose image doesn't exist, as paths
/// relative to `dir`. Reading skips these silently; Health Check reports them.
pub fn orphan_labels(dir: &Path, format: LabelFormat) -> io::Result<Vec<PathBuf>> {
    let files = walk(dir)?;
    let has_image = |stem_path: &Path| IMAGE_EXTENSIONS.iter().any(|ext| stem_path.with_extension(ext).is_file());
    let rel = |p: &Path| p.strip_prefix(dir).unwrap_or(p).to_path_buf();

    Ok(match format {
        LabelFormat::Yolo => files
            .iter()
            .filter(|p| extension(p) == "txt")
            .filter_map(|label| {
                // Only files in a `labels` folder: other .txt files (READMEs) aren't labels.
                let parts: Vec<_> = label.components().collect();
                let at = parts.iter().rposition(|c| c.as_os_str().eq_ignore_ascii_case("labels"))?;
                let image: PathBuf = parts
                    .iter()
                    .enumerate()
                    .map(|(j, c)| if j == at { "images".as_ref() } else { c.as_os_str() })
                    .collect();
                (!has_image(&image)).then(|| rel(label))
            })
            .collect(),
        LabelFormat::Voc => files.iter().filter(|p| extension(p) == "xml" && !has_image(p)).map(|p| rel(p)).collect(),
        LabelFormat::Coco => {
            let mut missing = Vec::new();
            for path in files.iter().filter(|p| extension(p) == "json") {
                let Ok(coco) = serde_json::from_slice::<CocoFile>(&fs::read(path)?) else { continue };
                let json_dir = path.parent().unwrap_or(dir);
                missing.extend(coco.images.iter().map(|i| json_dir.join(&i.file_name)).filter(|f| !f.is_file()).map(|f| rel(&f)));
            }
            missing
        }
    })
}

// ---------- shared helpers ----------

/// Counts one image read; stops reading once the user cancelled.
fn tick(progress: &dyn Progress) -> io::Result<()> {
    check_cancel(progress.is_cancelled()).map_err(|e| io::Error::new(io::ErrorKind::Interrupted, e))?;
    progress.add(1);
    Ok(())
}

fn new_image(file: &Path, rel: &Path, width: u32, height: u32) -> ImageLabels {
    ImageLabels {
        file: file.to_path_buf(),
        name: file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        split: Split::of_path(rel),
        width,
        height,
        objects: Vec::new(),
    }
}

fn image_size(file: &Path, warnings: &mut Vec<String>) -> Option<(u32, u32)> {
    match imagesize::size(file) {
        Ok(size) if size.width > 0 && size.height > 0 => Some((size.width as u32, size.height as u32)),
        Ok(_) => {
            warnings.push(format!("{}: image has no size, skipped", file.display()));
            None
        }
        Err(e) => {
            warnings.push(format!("{}: can't read image ({e}), skipped", file.display()));
            None
        }
    }
}

fn split_dir(dir: &Path, split: Option<Split>) -> PathBuf {
    match split {
        Some(s) => dir.join(s.dir_name()),
        None => dir.to_path_buf(),
    }
}

/// Hands out file names unique within each split: `a.jpg`, then `a_1.jpg`, ...
#[derive(Default)]
pub(crate) struct UniqueNames(HashMap<Option<Split>, HashSet<String>>);

impl UniqueNames {
    pub(crate) fn take(&mut self, split: Option<Split>, name: &str) -> String {
        let used = self.0.entry(split).or_default();
        let path = Path::new(name);
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let mut candidate = name.to_string();
        let mut n = 1;
        // Compare case-insensitively: Windows file names are.
        while !used.insert(candidate.to_lowercase()) {
            candidate = format!("{stem}_{n}{ext}");
            n += 1;
        }
        candidate
    }
}

/// All files under `dir`, sorted so reads are deterministic.
fn walk(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    Ok(files)
}

fn is_image(path: &Path) -> bool {
    IMAGE_EXTENSIONS.contains(&extension(path).as_str())
}

fn extension(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn read_lossy(path: &Path) -> io::Result<String> {
    Ok(String::from_utf8_lossy(&fs::read(path)?).into_owned())
}

/// Up to 6 decimals, without trailing zeros.
fn num(value: f64) -> String {
    let text = format!("{value:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".into() } else { text.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::progress::Silent;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("modeloid-labels-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn write(&self, rel: &str, contents: impl AsRef<[u8]>) {
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

    fn assert_close(a: BBox, b: BBox) {
        let close = |x: f64, y: f64| (x - y).abs() < 1e-3;
        assert!(close(a.x, b.x) && close(a.y, b.y) && close(a.w, b.w) && close(a.h, b.h), "{a:?} != {b:?}");
    }

    /// Just enough PNG for a header-only size read.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    fn yolo_dataset(name: &str) -> TempDir {
        let tmp = TempDir::new(name);
        tmp.write("data.yaml", "names: ['cat', 'dog']\n");
        tmp.write("train/images/a.png", png(100, 50));
        tmp.write("train/labels/a.txt", "0 0.5 0.5 0.2 0.4\n1 0.1 0.1 0.5 0.1 0.5 0.5\n");
        tmp.write("valid/images/b.png", png(200, 100));
        tmp.write("valid/labels/b.txt", "garbage\n");
        tmp.write("valid/images/broken.png", "not an image");
        tmp
    }

    #[test]
    fn stops_reading_when_cancelled() {
        struct Stopped;
        impl Progress for Stopped {
            fn set_total(&self, _: u64) {}
            fn add(&self, _: u64) {}
            fn finish(&self) {}
            fn is_cancelled(&self) -> bool {
                true
            }
        }
        let tmp = yolo_dataset("read-cancelled");
        let err = read(&tmp.0, LabelFormat::Yolo, &Stopped).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn parses_format_names() {
        assert_eq!(LabelFormat::parse("yolov8"), Some(LabelFormat::Yolo));
        assert_eq!(LabelFormat::parse("coco-segmentation"), Some(LabelFormat::Coco));
        assert_eq!(LabelFormat::parse("voc"), Some(LabelFormat::Voc));
        assert_eq!(LabelFormat::parse("createml"), None);
    }

    #[test]
    fn reads_yolo() {
        let tmp = yolo_dataset("read-yolo");
        let set = read(&tmp.0, LabelFormat::Yolo, &Silent).unwrap();

        assert_eq!(set.classes, ["cat", "dog"]);
        assert_eq!(set.images.len(), 2);
        assert_eq!(set.warnings.len(), 2, "{:?}", set.warnings); // broken image + garbage line

        let a = &set.images[0];
        assert_eq!((a.name.as_str(), a.split, a.width, a.height), ("a.png", Some(Split::Train), 100, 50));
        assert_close(a.objects[0].bbox, BBox { x: 40.0, y: 15.0, w: 20.0, h: 20.0 });
        let polygon = a.objects[1].polygon.as_ref().unwrap();
        assert_eq!(polygon.len(), 3);
        assert_close(a.objects[1].bbox, BBox { x: 10.0, y: 5.0, w: 40.0, h: 20.0 });
        assert_eq!(set.images[1].split, Some(Split::Val));
    }

    #[test]
    fn roundtrips_through_every_format() {
        let tmp = yolo_dataset("roundtrip-src");
        let original = read(&tmp.0, LabelFormat::Yolo, &Silent).unwrap();

        let out = TempDir::new("roundtrip-out");
        let mut set = original.clone();
        for format in [LabelFormat::Coco, LabelFormat::Voc, LabelFormat::Yolo] {
            let dir = out.0.join(format.name());
            write(&set, &dir, format, &Silent).unwrap();
            set = read(&dir, format, &Silent).unwrap();
            assert!(set.warnings.is_empty(), "{format:?}: {:?}", set.warnings);
            assert_eq!(set.images.len(), 2, "{format:?}");
            assert_eq!(set.classes, ["cat", "dog"], "{format:?}");
        }

        // VOC keeps boxes only; boxes survive the whole trip.
        let (a, b) = (&original.images[0], &set.images[0]);
        assert_eq!(b.name, "a.png");
        assert_eq!(b.split, Some(Split::Train));
        assert_eq!(a.objects.len(), b.objects.len());
        for (x, y) in a.objects.iter().zip(&b.objects) {
            assert_eq!(x.class, y.class);
            assert_close(x.bbox, y.bbox);
        }
        assert!(!set.has_polygons());
        assert!(out.0.join("yolo/data.yaml").is_file());
    }

    #[test]
    fn describes_what_a_scan_would_find() {
        use crate::core::scan::scan_dataset;

        let tmp = yolo_dataset("describe-src");
        let set = read(&tmp.0, LabelFormat::Yolo, &Silent).unwrap();
        let out = TempDir::new("describe-out");
        for format in [LabelFormat::Yolo, LabelFormat::Coco, LabelFormat::Voc] {
            let dir = out.0.join(format.name());
            let written = write(&set, &dir, format, &Silent).unwrap();
            let described = describe(&set, format, written).unwrap();
            let scanned = scan_dataset(&dir, &Silent).unwrap();
            assert_eq!(described, scanned, "{format:?}");
        }
        assert_eq!(describe(&LabelSet::default(), LabelFormat::Yolo, Written { bytes: 0 }), None);
    }

    #[test]
    fn coco_keeps_polygons_and_skips_roboflow_root() {
        let tmp = TempDir::new("coco");
        tmp.write("train/a.png", png(10, 10));
        tmp.write(
            "train/_annotations.coco.json",
            r#"{
                "images": [{"id": 5, "file_name": "a.png", "width": 10, "height": 10}],
                "categories": [
                    {"id": 0, "name": "pets", "supercategory": "none"},
                    {"id": 1, "name": "cat", "supercategory": "pets"}
                ],
                "annotations": [{"id": 0, "image_id": 5, "category_id": 1, "bbox": [1, 2, 3, 4],
                                 "segmentation": [[1, 2, 4, 2, 4, 6]]}]
            }"#,
        );
        let set = read(&tmp.0, LabelFormat::Coco, &Silent).unwrap();
        assert_eq!(set.classes, ["cat"]);
        let object = &set.images[0].objects[0];
        assert_eq!(object.bbox, BBox { x: 1.0, y: 2.0, w: 3.0, h: 4.0 });
        assert_eq!(object.polygon.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn makes_names_unique_per_split() {
        let mut names = UniqueNames::default();
        assert_eq!(names.take(Some(Split::Train), "a.jpg"), "a.jpg");
        assert_eq!(names.take(Some(Split::Train), "A.jpg"), "A_1.jpg");
        assert_eq!(names.take(Some(Split::Train), "a.jpg"), "a_2.jpg");
        assert_eq!(names.take(Some(Split::Val), "a.jpg"), "a.jpg");
    }

    #[test]
    fn writes_data_yaml() {
        let splits = BTreeSet::from([Some(Split::Train), Some(Split::Test)]);
        let yaml = data_yaml(&["it's".into(), "b".into()], &splits);
        assert_eq!(yaml, "train: train/images\nval: train/images\ntest: test/images\n\nnc: 2\nnames: ['it''s', 'b']\n");
        assert_eq!(parse_yaml_names(&yaml), ["it's", "b"]);
    }

    #[test]
    fn formats_numbers() {
        assert_eq!(num(0.5), "0.5");
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.1234567), "0.123457");
        assert_eq!(num(-0.0000001), "0");
    }
}
