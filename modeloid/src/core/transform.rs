//! Transform: augmented copies of a share of a dataset's images, made by
//! `main.py augment` (Albumentations) in the Python environment. Labels go
//! along: boxes and outlines move with the pixels, and those pushed out of
//! the image are clipped or dropped.
//!
//! The chosen images are written to a work folder in `.cache/tmp` with their
//! labels in `config.json`; Python writes the copies next to it and reports
//! each one's labels. The result (the originals plus the copies, in the
//! input's label format) is saved like every tool's output.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::core::balance::Rng;
use crate::core::labels::{self, BBox, ImageLabels, LabelFormat, LabelSet, Object};
use crate::core::progress::{Reporter, Unit};
use crate::core::pyenv::{self, Output as PyOutput, PyEnv};
use crate::core::stats::split_slot;
use crate::core::train::strip_ansi;
use crate::models::dataset::Lineage;
use crate::models::workspace::{Output, Workspace};

pub use modeloid_types::transform::*;

/// What `Workspace::apply_transform` did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TransformSummary {
    /// Augmented images written.
    pub added: u32,
    /// Images Python couldn't augment, and the first reason.
    pub failed: u32,
    pub first_error: Option<String>,
    pub images_after: [u32; 4],
}

/// Whether `image` is one the plan augments (by split and class).
fn matches(plan: &TransformPlan, set: &LabelSet, image: &ImageLabels) -> bool {
    plan.splits[split_slot(image.split)]
        && (plan.classes.is_empty() || image.objects.iter().any(|o| set.classes.get(o.class).is_some_and(|c| plan.classes.contains(c))))
}

/// The images that get copies (positions in `set.images`, in their original
/// order) and the counts. Nothing is chosen when the preview has problems.
fn choose(plan: &TransformPlan, seed: u64, set: &LabelSet) -> (Vec<usize>, TransformPreview) {
    let mut before = [0u32; 4];
    for image in &set.images {
        before[split_slot(image.split)] += 1;
    }
    let mut preview = TransformPreview {
        seed,
        classes: set.classes.clone(),
        images_before: before,
        images_after: before,
        has_polygons: set.has_polygons(),
        problems: plan.problems(),
        ..Default::default()
    };
    for class in plan.classes.iter().filter(|c| !set.classes.contains(*c)) {
        preview.problems.push(format!("\"{class}\" isn't a class of this dataset."));
    }

    // From the file order, so the choice doesn't depend on how the labels were read.
    let mut order: Vec<usize> = (0..set.images.len()).filter(|&i| matches(plan, set, &set.images[i])).collect();
    order.sort_by(|&a, &b| set.images[a].file.cmp(&set.images[b].file));
    preview.matching = order.len() as u32;
    if order.is_empty() {
        preview.problems.push("No images match the chosen splits and classes.".into());
    }
    if !preview.problems.is_empty() {
        return (Vec::new(), preview);
    }

    if plan.percent < 100 {
        Rng(seed).shuffle(&mut order);
        let take = (order.len() as u64 * u64::from(plan.percent) + 50) / 100;
        order.truncate(take.max(1) as usize);
    }
    order.sort_unstable();

    preview.chosen = order.len() as u32;
    preview.added = preview.chosen * plan.copies;
    for &i in &order {
        let slot = split_slot(set.images[i].split);
        preview.images_after[slot] += plan.copies;
        if !plan.keep_originals {
            preview.images_after[slot] -= 1;
        }
    }
    (order, preview)
}

/// What `plan` (seeded with `seed` unless it has its own) gives on `set`.
pub fn preview(plan: &TransformPlan, seed: u64, set: &LabelSet) -> TransformPreview {
    choose(plan, plan.seed.unwrap_or(seed), set).1
}

/// A setting's value as Python gets it (whole numbers without decimals).
fn param_json(param: &AugParam, value: &AugValue) -> Value {
    let n = |v: f64| if param.integer { json!(v.round() as i64) } else { json!(v) };
    match value {
        AugValue::Pair([low, high]) => json!([n(*low), n(*high)]),
        AugValue::Number(v) => n(*v),
        AugValue::Text(t) => json!(t),
    }
}

/// The pipeline: the used rows in the order they are applied.
fn transforms_json(plan: &TransformPlan) -> Vec<Value> {
    plan.enabled()
        .into_iter()
        .filter_map(|row| {
            let a = augmentation(&row.key)?;
            let params: serde_json::Map<String, Value> = a.params.iter().map(|p| (p.key.to_string(), param_json(p, &row.value(p)))).collect();
            Some(json!({ "name": a.key, "p": row.p, "params": params }))
        })
        .collect()
}

fn image_json(index: usize, image: &ImageLabels, copies: u32) -> Value {
    let objects: Vec<Value> = image
        .objects
        .iter()
        .map(|o| json!({ "class": o.class, "bbox": [o.bbox.x, o.bbox.y, o.bbox.w, o.bbox.h], "polygon": o.polygon }))
        .collect();
    json!({
        "index": index,
        "file": image.file.display().to_string(),
        "width": image.width,
        "height": image.height,
        "copies": copies,
        "objects": objects,
    })
}

/// The labelled copies in an `image` event.
fn copies_of(event: &Value, source: &ImageLabels) -> Vec<ImageLabels> {
    let stem = Path::new(&source.name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
    let Some(outputs) = event["outputs"].as_array() else { return Vec::new() };
    outputs
        .iter()
        .enumerate()
        .filter_map(|(n, out)| {
            let file = PathBuf::from(out["file"].as_str()?);
            let ext = file.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "jpg".into());
            let objects = out["objects"]
                .as_array()
                .map(|objects| {
                    objects
                        .iter()
                        .filter_map(|o| {
                            let b: Vec<f64> = o["bbox"].as_array()?.iter().filter_map(Value::as_f64).collect();
                            let [x, y, w, h] = b[..] else { return None };
                            let polygon = o["polygon"]
                                .as_array()
                                .map(|points| points.iter().filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?])).collect::<Vec<_>>());
                            Some(Object { class: o["class"].as_u64()? as usize, bbox: BBox { x, y, w, h }, polygon })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(ImageLabels {
                file,
                name: format!("{stem}_aug{}.{ext}", n + 1),
                split: source.split,
                width: out["width"].as_u64()? as u32,
                height: out["height"].as_u64()? as u32,
                objects,
            })
        })
        .collect()
}

/// Runs `main.py augment` on the chosen images (`chosen`, positions in
/// `set.images`) in `work`: the copies with their labels, in the order of
/// `chosen`, and the failures.
fn run_python(
    env: &PyEnv,
    plan: &TransformPlan,
    seed: u64,
    set: &LabelSet,
    chosen: &[usize],
    work: &Path,
    reporter: &dyn Reporter,
) -> Result<(Vec<ImageLabels>, TransformSummary), crate::Error> {
    let out_dir = work.join("images");
    fs::create_dir_all(&out_dir)?;
    let config = work.join("config.json");
    let images: Vec<Value> = chosen.iter().map(|&i| image_json(i, &set.images[i], plan.copies)).collect();
    fs::write(
        &config,
        serde_json::to_vec(&json!({
            "seed": seed,
            "out_dir": out_dir.display().to_string(),
            "transforms": transforms_json(plan),
            "images": images,
        }))?,
    )?;

    let progress = reporter.start("Augmenting", Unit::Files);
    progress.set_total(chosen.len() as u64);
    let mut summary = TransformSummary::default();
    let mut copies = Vec::new();
    let mut last_error: Option<String> = None;
    let mut output_tail: Vec<String> = Vec::new();
    let cmd = env.service_command([std::ffi::OsStr::new("augment"), config.as_os_str()])?;
    let result = pyenv::stream(cmd, progress.as_ref(), |out| {
        let PyOutput::Line(line) = out else { return };
        let line = strip_ansi(&line);
        let Some(event) = line.strip_prefix("@modeloid ").and_then(|j| serde_json::from_str::<Value>(j).ok()) else {
            // Kept for the error message if Python fails without an event.
            if !line.trim().is_empty() {
                output_tail.push(line.trim_end().to_string());
                if output_tail.len() > 5 {
                    output_tail.remove(0);
                }
            }
            return;
        };
        match event["event"].as_str() {
            Some("image") => {
                let source = event["index"].as_u64().and_then(|i| set.images.get(i as usize));
                match (source, event["error"].as_str()) {
                    (Some(source), Some(error)) => {
                        summary.failed += 1;
                        summary.first_error.get_or_insert_with(|| format!("{}: {error}", source.name));
                    }
                    (Some(source), None) => {
                        let made = copies_of(&event, source);
                        summary.added += made.len() as u32;
                        copies.extend(made);
                    }
                    (None, _) => {}
                }
                progress.add(1);
            }
            Some("error") => last_error = event["error"].as_str().map(str::to_string),
            _ => {}
        }
    });
    progress.finish();
    if let Err(e) = result {
        if e.downcast_ref::<crate::Cancelled>().is_some() {
            return Err(e);
        }
        let detail = last_error.or_else(|| output_tail.last().cloned()).unwrap_or_else(|| e.to_string());
        return Err(format!("Augmenting failed: {detail}").into());
    }
    if summary.added == 0 {
        let why = summary.first_error.clone().unwrap_or_else(|| "Python made no copies".into());
        return Err(format!("No image could be augmented. {why}").into());
    }
    Ok((copies, summary))
}

impl Workspace {
    /// What Transform `plan` gives on `set` (its input's labels), with the
    /// workspace's seed unless the plan has its own.
    pub fn transform_preview_of(&self, plan: &TransformPlan, set: &LabelSet) -> TransformPreview {
        preview(plan, self.seed(), set)
    }

    /// Makes the augmented copies with Python (`env`) and saves the input's
    /// images plus the copies (without the chosen originals unless
    /// `keep_originals`) as `output`, in the input's label format.
    pub fn apply_transform(&self, plan: &TransformPlan, output: &Output, env: &PyEnv, reporter: &dyn Reporter) -> Result<(PathBuf, TransformSummary), crate::Error> {
        let reading = reporter.start("Reading labels", Unit::Files);
        let read = self.read_labels(plan.input, reading.as_ref());
        reading.finish();
        let (_, dataset, set) = read?;
        let format = LabelFormat::parse(dataset.annotation_format()).ok_or("unsupported label format")?;
        let seed = plan.seed.unwrap_or_else(|| self.seed());
        let (chosen, preview) = choose(plan, seed, &set);
        if let Some(problem) = preview.problems.first() {
            return Err(problem.clone().into());
        }

        let work = self.staging_dir("transform-work")?;
        let result = (|| {
            let (copies, mut summary) = run_python(env, plan, seed, &set, &chosen, &work, reporter)?;
            let mut out = LabelSet { classes: set.classes.clone(), warnings: Vec::new(), declared_classes: set.declared_classes, images: Vec::new() };
            let mut is_chosen = vec![false; set.images.len()];
            for &i in &chosen {
                is_chosen[i] = true;
            }
            out.images = set.images.iter().zip(&is_chosen).filter(|(_, chosen)| plan.keep_originals || !**chosen).map(|(image, _)| image.clone()).collect();
            out.images.extend(copies);
            for image in &out.images {
                summary.images_after[split_slot(image.split)] += 1;
            }

            let staging = self.staging_dir("transform")?;
            let writing = reporter.start("Writing", Unit::Files);
            let written = labels::write(&out, &staging, format, writing.as_ref());
            writing.finish();
            let described = match written {
                Ok(written) => labels::describe(&out, format, written),
                Err(e) => {
                    let _ = fs::remove_dir_all(&staging);
                    return Err(e.into());
                }
            };

            let names: Vec<String> = plan.enabled().iter().map(|r| r.key.clone()).collect();
            let splits: Vec<String> = plan
                .splits
                .iter()
                .zip(["train", "val", "test", "none"])
                .filter(|(on, _)| **on)
                .map(|(_, name)| name.to_string())
                .collect();
            let seed_value: toml::Value = i64::try_from(seed).map_or_else(|_| seed.to_string().into(), toml::Value::from);
            let mut lineage = Lineage::new("transform", &[&dataset])
                .with_param("augmentations", names)
                .with_param("splits", splits)
                .with_param("percent", i64::from(plan.percent))
                .with_param("copies", i64::from(plan.copies))
                .with_param("keep_originals", plan.keep_originals)
                .with_param("seed", seed_value)
                .with_param("added", i64::from(summary.added));
            if !plan.classes.is_empty() {
                lineage = lineage.with_param("classes", plan.classes.clone());
            }
            match self.save_described(&staging, output, lineage, described, reporter) {
                Ok(dir) => Ok((dir, summary)),
                Err(e) => {
                    let _ = fs::remove_dir_all(&staging);
                    Err(e)
                }
            }
        })();
        let _ = fs::remove_dir_all(&work);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::labels::Split;

    /// Classes a, b; one image per entry: its split and the classes of its objects.
    fn set(images: Vec<(Option<Split>, Vec<usize>)>) -> LabelSet {
        LabelSet {
            classes: vec!["a".into(), "b".into()],
            images: images
                .into_iter()
                .enumerate()
                .map(|(i, (split, objects))| ImageLabels {
                    file: PathBuf::from(format!("{i:03}.jpg")),
                    name: format!("{i:03}.jpg"),
                    split,
                    width: 100,
                    height: 100,
                    objects: objects.into_iter().map(|class| Object { class, bbox: BBox { x: 10.0, y: 10.0, w: 20.0, h: 20.0 }, polygon: None }).collect(),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn chooses_by_split_class_and_share() {
        // 10 train (the first 4 with b), 4 val.
        let mut images: Vec<_> = (0..10).map(|i| (Some(Split::Train), vec![if i < 4 { 1 } else { 0 }])).collect();
        images.extend((0..4).map(|_| (Some(Split::Val), vec![0])));
        let data = set(images);

        let plan = TransformPlan { copies: 2, ..TransformPlan::new(1) };
        let (chosen, p) = choose(&plan, 7, &data);
        assert!(p.can_run(), "{:?}", p.problems);
        assert_eq!((p.matching, p.chosen, p.added), (10, 10, 20));
        assert_eq!(p.images_after, [30, 4, 0, 0]);
        assert_eq!(chosen, (0..10).collect::<Vec<_>>());

        let half = TransformPlan { percent: 50, keep_originals: false, ..plan.clone() };
        let (chosen, p) = choose(&half, 7, &data);
        assert_eq!((p.chosen, p.images_after), (5, [15, 4, 0, 0]), "5 originals replaced by 10 copies");
        assert!(chosen.windows(2).all(|w| w[0] < w[1]), "original order");
        assert_eq!(choose(&half, 7, &data).0, chosen, "same seed, same choice");
        assert_ne!(choose(&half, 8, &data).0, chosen, "another seed, another choice");

        let only_b = TransformPlan { classes: vec!["b".into()], splits: [true, true, false, false], ..plan.clone() };
        assert_eq!(choose(&only_b, 7, &data).1.matching, 4);

        let unknown = TransformPlan { classes: vec!["c".into()], ..plan.clone() };
        assert!(choose(&unknown, 7, &data).1.problems.iter().any(|p| p.contains("\"c\"")));
        let test_only = TransformPlan { splits: [false, false, true, false], ..plan };
        let (chosen, p) = choose(&test_only, 7, &data);
        assert!(chosen.is_empty() && p.problems.iter().any(|p| p.contains("No images")));
    }

    #[test]
    fn builds_the_pipeline_for_python() {
        let mut plan = TransformPlan::new(1);
        for row in &mut plan.augmentations {
            row.enabled = matches!(row.key.as_str(), "Blur" | "Resize" | "HorizontalFlip");
        }
        let t = transforms_json(&plan);
        let names: Vec<&str> = t.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(names, ["Resize", "HorizontalFlip", "Blur"], "in the catalog's order");
        assert_eq!(t[0]["params"], json!({ "height": 640, "width": 640 }));
        assert_eq!(t[2]["params"]["blur_limit"], json!([3, 7]));
        assert_eq!(t[1]["p"], json!(0.5));
    }

    #[test]
    fn reads_the_copies_python_made() {
        let source = &set(vec![(Some(Split::Val), vec![0])]).images[0];
        let event = json!({ "event": "image", "index": 0, "outputs": [
            { "file": "/w/0_1.png", "width": 50, "height": 40, "objects": [
                { "class": 1, "bbox": [1.0, 2.0, 3.0, 4.0], "polygon": null },
                { "class": 0, "bbox": [0.0, 0.0, 5.0, 5.0], "polygon": [[0.0, 0.0], [5.0, 0.0], [5.0, 5.0]] }
            ] }
        ] });
        let copies = copies_of(&event, source);
        assert_eq!(copies.len(), 1);
        let c = &copies[0];
        assert_eq!((c.name.as_str(), c.split, c.width, c.height), ("000_aug1.png", Some(Split::Val), 50, 40));
        assert_eq!(c.objects[0].bbox, BBox { x: 1.0, y: 2.0, w: 3.0, h: 4.0 });
        assert_eq!(c.objects[1].polygon.as_ref().map(Vec::len), Some(3));
    }

    /// The whole run with the real Python environment:
    /// `cargo test -p modeloid transforms_a_dataset -- --ignored`.
    #[test]
    #[ignore = "needs the Python environment with Albumentations"]
    fn transforms_a_dataset() {
        let env = PyEnv::locate().unwrap();
        assert!(env.status().usable(), "no Python environment");
        let dir = std::env::temp_dir().join(format!("modeloid-transform-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let ws = Workspace::new("ws".into(), dir.clone());
        ws.make_to_dir().unwrap();

        // A YOLO dataset: 3 train images with one box each, 1 val image.
        let src = dir.join("src");
        for (split, n) in [("train", 3), ("valid", 1)] {
            fs::create_dir_all(src.join(split).join("images")).unwrap();
            fs::create_dir_all(src.join(split).join("labels")).unwrap();
            for i in 0..n {
                image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([(x * 4) as u8, (y * 5) as u8, 90])).save(src.join(split).join("images").join(format!("img{i}.jpg"))).unwrap();
                fs::write(src.join(split).join("labels").join(format!("img{i}.txt")), "0 0.25 0.5 0.25 0.5\n").unwrap();
            }
        }
        fs::write(src.join("data.yaml"), "names: ['thing']\nnc: 1\n").unwrap();
        ws.import_dir(&src, "shapes", crate::models::workspace::ImportMode::Copy, &crate::Silent).unwrap();
        let (_, dataset) = ws.list_datasets().unwrap().into_iter().find(|(_, d)| d.name() == "shapes").unwrap();

        let plan = TransformPlan { input: dataset.id(), copies: 2, ..TransformPlan::new(dataset.id()) };
        let (out, summary) = ws.apply_transform(&plan, &Output::New { name: "shapes-aug".into() }, &env, &crate::Silent).unwrap();
        assert_eq!((summary.added, summary.failed), (6, 0));
        assert_eq!(summary.images_after, [9, 1, 0, 0]);
        let set = labels::read(&out, LabelFormat::Yolo, &crate::Silent).unwrap();
        assert_eq!(set.images.len(), 10);
        let copies: Vec<_> = set.images.iter().filter(|i| i.name.contains("_aug")).collect();
        assert_eq!(copies.len(), 6);
        assert!(copies.iter().all(|i| i.split == Some(crate::core::labels::Split::Train)));
        assert!(copies.iter().map(|i| i.objects.len()).sum::<usize>() >= 5, "boxes go along (one may leave the image)");
        let toml = fs::read_to_string(out.join(crate::models::dataset::DATASET_FILE)).unwrap();
        assert!(toml.contains("transform"), "lineage recorded");
        let _ = fs::remove_dir_all(&dir);
    }
}
