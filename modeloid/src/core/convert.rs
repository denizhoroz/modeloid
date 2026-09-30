//! Convert: preview a conversion from a dataset's statistics, turn outlines
//! into boxes, and cut objects out into class folders (classification).
//! Writing another label format is `labels::write` with that format.

use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicU32, Ordering::Relaxed},
};

use crate::core::{
    io::run_parallel,
    labels::{BBox, ImageLabels, LabelFormat, LabelSet, UniqueNames},
    progress::Progress,
    stats::ExploreStats,
};

pub use modeloid_types::convert::*;

/// Name to show for a label format.
pub fn format_label(format: LabelFormat) -> &'static str {
    match format {
        LabelFormat::Yolo => "YOLO",
        LabelFormat::Coco => "COCO",
        LabelFormat::Voc => "Pascal VOC",
    }
}

/// What `plan` would write for a dataset in `source` format with these statistics.
pub fn preview(plan: &ConvertPlan, source: LabelFormat, stats: &ExploreStats) -> ConvertPreview {
    let mut preview = ConvertPreview {
        source_format: source.name().to_string(),
        images: stats.image_count,
        labels: stats.label_count,
        outlines: stats.polygon_count,
        classes: stats
            .classes
            .iter()
            .filter(|c| c.total_labels() > 0)
            .map(|c| ClassCount { name: c.name.clone(), labels: c.total_labels() })
            .collect(),
        ..Default::default()
    };

    if plan.task == TargetTask::Classification {
        if stats.label_count == 0 {
            preview.problems.push("There are no objects to cut out.".into());
        }
        if plan.padding_percent > MAX_PADDING {
            preview.problems.push(format!("The padding can be {MAX_PADDING}% at most."));
        }
        return preview;
    }

    let Some(target) = LabelFormat::parse(&plan.format) else {
        preview.problems.push("Choose YOLO, COCO or Pascal VOC as the label format.".into());
        return preview;
    };
    let boxes_only = plan.task == TargetTask::Detection || target == LabelFormat::Voc;
    if boxes_only {
        preview.outlines_lost = stats.polygon_count;
    }
    if target == source && preview.outlines_lost == 0 {
        let what = if stats.polygon_count == 0 { " and has no outlines to turn into boxes" } else { "" };
        preview.problems.push(format!("Nothing would change: the dataset is already in {} format{what}.", format_label(source)));
    } else if plan.task == TargetTask::Same && preview.outlines_lost > 0 {
        preview.notes.push(format!("Pascal VOC has no outlines: {} outlines become boxes.", preview.outlines_lost));
    } else if plan.task == TargetTask::Detection && stats.polygon_count == 0 {
        preview.notes.push("This dataset has no outlines, so only the label format changes.".into());
    }
    preview
}

/// Replaces every outline by its bounding box. Returns how many there were.
pub fn strip_outlines(set: &mut LabelSet) -> u32 {
    let mut count = 0;
    for object in set.images.iter_mut().flat_map(|i| &mut i.objects) {
        if object.polygon.take().is_some() {
            count += 1;
        }
    }
    count
}

/// What `write_crops` wrote and left out.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CropSummary {
    pub crops: u32,
    /// Boxes smaller than the size limit (or empty after clipping to the image).
    pub skipped_small: u32,
    /// Images that couldn't be decoded; their objects are left out.
    pub unreadable_images: u32,
}

/// Cuts every object of `set` out of its image into
/// `dir/<split>/<class>/<image name>_<object number>.<ext>`: a classification
/// dataset with one folder per class. Boxes grow by `padding_percent` of their
/// width and height on each side and are clipped to the image; crops under
/// `min_size` pixels in either direction are skipped. Counts each image with
/// objects on `progress`.
pub fn write_crops(
    set: &LabelSet,
    dir: &Path,
    padding_percent: u32,
    min_size: u32,
    progress: &dyn Progress,
) -> Result<CropSummary, crate::Error> {
    // Output names are handed out up front, so they don't depend on thread timing.
    let mut names = UniqueNames::default();
    let jobs: Vec<(&ImageLabels, String)> =
        set.images.iter().filter(|i| !i.objects.is_empty()).map(|i| (i, names.take(i.split, &i.name))).collect();
    let (crops, skipped_small, unreadable_images) = (AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0));

    progress.set_total(jobs.len() as u64);
    run_parallel(&jobs, progress, || Ok(()), |_, (image, name)| {
        let Ok(pixels) = image::open(&image.file) else {
            unreadable_images.fetch_add(1, Relaxed);
            return Ok(());
        };
        let name = Path::new(name);
        let stem = name.file_stem().unwrap_or_default().to_string_lossy();
        let extension = name.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
        let split_dir = match image.split {
            Some(split) => dir.join(split.dir_name()),
            None => dir.to_path_buf(),
        };
        // Labels may assume another image size than the file has.
        let scale = (f64::from(pixels.width()) / f64::from(image.width.max(1)), f64::from(pixels.height()) / f64::from(image.height.max(1)));

        for (n, object) in image.objects.iter().enumerate() {
            let Some([x, y, w, h]) = crop_rect(object.bbox, scale, (pixels.width(), pixels.height()), padding_percent, min_size) else {
                skipped_small.fetch_add(1, Relaxed);
                continue;
            };
            let folder = split_dir.join(set.classes.get(object.class).map_or("unknown".to_string(), |c| folder_name(c)));
            fs::create_dir_all(&folder)?;
            // RGB: every supported format can store it (JPEG has no transparency).
            pixels.crop_imm(x, y, w, h).to_rgb8().save(folder.join(format!("{stem}_{n}.{extension}")))?;
            crops.fetch_add(1, Relaxed);
        }
        Ok(())
    })?;

    Ok(CropSummary { crops: crops.into_inner(), skipped_small: skipped_small.into_inner(), unreadable_images: unreadable_images.into_inner() })
}

/// The pixels to cut out for `bbox`: x, y, width, height. `None` when less
/// than `min_size` (or nothing) is left inside the image.
fn crop_rect(bbox: BBox, scale: (f64, f64), image: (u32, u32), padding_percent: u32, min_size: u32) -> Option<[u32; 4]> {
    let pad = f64::from(padding_percent) / 100.0;
    let (pad_x, pad_y) = (bbox.w * pad, bbox.h * pad);
    let x0 = ((bbox.x - pad_x) * scale.0).floor().max(0.0);
    let y0 = ((bbox.y - pad_y) * scale.1).floor().max(0.0);
    let x1 = ((bbox.x + bbox.w + pad_x) * scale.0).ceil().min(f64::from(image.0));
    let y1 = ((bbox.y + bbox.h + pad_y) * scale.1).ceil().min(f64::from(image.1));
    let smallest = f64::from(min_size.max(1));
    (x1 - x0 >= smallest && y1 - y0 >= smallest).then(|| [x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32])
}

/// A class name as a folder name: characters that file systems refuse become `_`.
fn folder_name(class: &str) -> String {
    let name: String = class.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let name = name.trim().trim_end_matches('.');
    if name.is_empty() { "unknown".into() } else { name.into() }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::core::labels::{Object, Split};
    use crate::core::progress::Silent;
    use crate::core::stats::analyze;

    fn object(class: usize, x: f64, y: f64, w: f64, h: f64, outline: bool) -> Object {
        let polygon = outline.then(|| vec![[x, y], [x + w, y], [x + w, y + h]]);
        Object { class, bbox: BBox { x, y, w, h }, polygon }
    }

    fn set(file: PathBuf, objects: Vec<Object>) -> LabelSet {
        LabelSet {
            classes: vec!["cat".into(), "a/b".into()],
            images: vec![ImageLabels { name: "pic.png".into(), file, split: Some(Split::Train), width: 40, height: 20, objects }],
            ..Default::default()
        }
    }

    fn plan(task: TargetTask, format: &str) -> ConvertPlan {
        ConvertPlan { input: 1, task, format: format.into(), padding_percent: 0, min_size: 0 }
    }

    #[test]
    fn previews_what_changes() {
        let boxes = analyze(&set(PathBuf::new(), vec![object(0, 0.0, 0.0, 5.0, 5.0, false)]));
        let outlined = analyze(&set(PathBuf::new(), vec![object(0, 0.0, 0.0, 5.0, 5.0, true), object(1, 1.0, 1.0, 2.0, 2.0, false)]));

        let same = preview(&plan(TargetTask::Same, "yolo"), LabelFormat::Yolo, &boxes);
        assert!(same.problems[0].contains("already in YOLO"), "{:?}", same.problems);
        assert!(preview(&plan(TargetTask::Same, "coco"), LabelFormat::Yolo, &boxes).can_run());
        assert!(!preview(&plan(TargetTask::Detection, "yolo"), LabelFormat::Yolo, &boxes).can_run(), "no outlines, same format");
        assert!(!preview(&plan(TargetTask::Same, "parquet"), LabelFormat::Yolo, &boxes).can_run());

        let to_boxes = preview(&plan(TargetTask::Detection, "yolo"), LabelFormat::Yolo, &outlined);
        assert!(to_boxes.can_run());
        assert_eq!((to_boxes.outlines, to_boxes.outlines_lost, to_boxes.labels), (1, 1, 2));
        assert_eq!(preview(&plan(TargetTask::Same, "coco"), LabelFormat::Yolo, &outlined).outlines_lost, 0);
        let voc = preview(&plan(TargetTask::Same, "voc"), LabelFormat::Yolo, &outlined);
        assert_eq!((voc.outlines_lost, voc.notes.len()), (1, 1));

        let crops = preview(&plan(TargetTask::Classification, ""), LabelFormat::Yolo, &outlined);
        assert!(crops.can_run());
        assert_eq!(crops.classes, [ClassCount { name: "cat".into(), labels: 1 }, ClassCount { name: "a/b".into(), labels: 1 }]);
        let empty = analyze(&set(PathBuf::new(), vec![]));
        assert!(!preview(&plan(TargetTask::Classification, ""), LabelFormat::Yolo, &empty).can_run());
    }

    #[test]
    fn strips_outlines() {
        let mut data = set(PathBuf::new(), vec![object(0, 0.0, 0.0, 5.0, 5.0, true), object(0, 1.0, 1.0, 2.0, 2.0, false)]);
        assert_eq!(strip_outlines(&mut data), 1);
        assert!(!data.has_polygons());
        assert_eq!(data.images[0].objects[0].bbox.w, 5.0);
    }

    #[test]
    fn crops_objects_into_class_folders() {
        let dir = std::env::temp_dir().join(format!("modeloid-crops-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        let file = dir.join("src/pic.png");
        image::RgbaImage::new(40, 20).save(&file).unwrap();

        // A cat, a class with a slash reaching past the edge, and a box too small to keep.
        let data = set(file, vec![object(0, 10.0, 5.0, 10.0, 8.0, false), object(1, 30.0, 10.0, 20.0, 20.0, true), object(0, 0.0, 0.0, 2.0, 2.0, false)]);
        let out = dir.join("out");
        let summary = write_crops(&data, &out, 50, 4, &Silent).unwrap();
        assert_eq!(summary, CropSummary { crops: 2, skipped_small: 1, unreadable_images: 0 });

        // 10x8 box with half its size added on each side: 20x16.
        let cat = image::open(out.join("train/cat/pic_0.png")).unwrap();
        assert_eq!((cat.width(), cat.height()), (20, 16));
        // x 20..40 (clipped from 60), y 0..20 (clipped from 40).
        let clipped = image::open(out.join("train/a_b/pic_1.png")).unwrap();
        assert_eq!((clipped.width(), clipped.height()), (20, 20));

        let mut broken = data.clone();
        broken.images[0].file = dir.join("missing.png");
        assert_eq!(write_crops(&broken, &dir.join("out2"), 0, 0, &Silent).unwrap().unreadable_images, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn makes_folder_names() {
        assert_eq!(folder_name("traffic light"), "traffic light");
        assert_eq!(folder_name("a/b:c"), "a_b_c");
        assert_eq!(folder_name(" . "), "unknown");
    }
}
