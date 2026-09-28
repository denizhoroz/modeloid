//! Dataset statistics and gallery pages for Explore, computed from a
//! `LabelSet`. The result types live in `modeloid_types::stats` so the
//! browser side of the GUI can use them too.

use crate::core::labels::{ImageLabels, LabelSet};

pub use modeloid_types::stats::*;

/// Most distinct image sizes listed; the rest are summed into `other_sizes`.
const MAX_IMAGE_SIZES: usize = 30;
/// Most warnings kept; `warning_count` has the full number.
const MAX_WARNINGS: usize = 100;

/// Computes Explore's statistics for a dataset's labels.
pub fn analyze(set: &LabelSet) -> ExploreStats {
    let mut stats = ExploreStats {
        image_count: set.images.len() as u32,
        label_count: 0,
        images: [0; 4],
        empty_images: [0; 4],
        classes: set
            .classes
            .iter()
            .map(|name| ClassStats { name: name.clone(), labels: [0; 4], images: 0 })
            .collect(),
        labels_per_image: Histogram::linear(0.0, 21.0, 21),
        box_width: Histogram::linear(0.0, 1.0, 20),
        box_height: Histogram::linear(0.0, 1.0, 20),
        aspect_ratio: Histogram::linear(-4.0, 4.0, 16),
        size_classes: [0; 3],
        heatmap: vec![0; HEATMAP_SIZE * HEATMAP_SIZE],
        image_sizes: Vec::new(),
        other_sizes: 0,
        polygon_count: 0,
        warning_count: set.warnings.len() as u32,
        warnings: set.warnings.iter().take(MAX_WARNINGS).cloned().collect(),
    };
    let mut sizes = std::collections::HashMap::<(u32, u32), u32>::new();
    let mut seen = vec![false; set.classes.len()];

    for image in &set.images {
        let slot = split_slot(image.split);
        stats.images[slot] += 1;
        stats.label_count += image.objects.len() as u32;
        stats.labels_per_image.add(image.objects.len() as f64);
        if image.objects.is_empty() {
            stats.empty_images[slot] += 1;
        }
        *sizes.entry((image.width, image.height)).or_default() += 1;

        let (w, h) = (image.width.max(1) as f64, image.height.max(1) as f64);
        seen.iter_mut().for_each(|s| *s = false);
        for object in &image.objects {
            if let Some(class) = stats.classes.get_mut(object.class) {
                class.labels[slot] += 1;
                if !seen[object.class] {
                    seen[object.class] = true;
                    class.images += 1;
                }
            }
            let b = object.bbox;
            stats.box_width.add(b.w / w);
            stats.box_height.add(b.h / h);
            if b.w > 0.0 && b.h > 0.0 {
                stats.aspect_ratio.add((b.w / b.h).log2());
            }
            let area = b.area();
            stats.size_classes[if area < 32.0 * 32.0 { 0 } else if area < 96.0 * 96.0 { 1 } else { 2 }] += 1;
            let cell = |v: f64| ((v * HEATMAP_SIZE as f64).floor().max(0.0) as usize).min(HEATMAP_SIZE - 1);
            let (cx, cy) = (cell((b.x + b.w / 2.0) / w), cell((b.y + b.h / 2.0) / h));
            stats.heatmap[cy * HEATMAP_SIZE + cx] += 1;
            if object.polygon.is_some() {
                stats.polygon_count += 1;
            }
        }
    }

    let mut sizes: Vec<ImageSize> = sizes.into_iter().map(|((width, height), count)| ImageSize { width, height, count }).collect();
    sizes.sort_by(|a, b| b.count.cmp(&a.count).then((a.width, a.height).cmp(&(b.width, b.height))));
    stats.other_sizes = sizes.iter().skip(MAX_IMAGE_SIZES).map(|s| s.count).sum();
    sizes.truncate(MAX_IMAGE_SIZES);
    stats.image_sizes = sizes;
    stats
}

/// Whether `image` passes `filter`.
pub fn matches(filter: &GalleryFilter, image: &ImageLabels) -> bool {
    filter.split.is_none_or(|s| s == image.split)
        && (!filter.unlabeled || image.objects.is_empty())
        && filter.class.is_none_or(|c| image.objects.iter().any(|o| o.class == c))
}

/// Images `offset..offset + limit` of those matching `filter`.
pub fn gallery_page(set: &LabelSet, filter: &GalleryFilter, offset: usize, limit: usize) -> GalleryPage {
    let matching = set.images.iter().enumerate().filter(|(_, image)| matches(filter, image));
    let mut total = 0;
    let mut images = Vec::new();
    for (index, image) in matching {
        if total >= offset && images.len() < limit {
            let (w, h) = (image.width.max(1) as f64, image.height.max(1) as f64);
            images.push(GalleryImage {
                index,
                name: image.name.clone(),
                split: image.split,
                width: image.width,
                height: image.height,
                objects: image
                    .objects
                    .iter()
                    .map(|o| GalleryObject {
                        class: o.class,
                        bbox: [(o.bbox.x / w) as f32, (o.bbox.y / h) as f32, (o.bbox.w / w) as f32, (o.bbox.h / h) as f32],
                        polygon: o.polygon.as_ref().map(|p| p.iter().map(|[x, y]| [(x / w) as f32, (y / h) as f32]).collect()),
                    })
                    .collect(),
            });
        }
        total += 1;
    }
    GalleryPage { total, offset, images }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::core::labels::{BBox, Object, Split};

    fn image(split: Option<Split>, objects: Vec<(usize, BBox)>) -> ImageLabels {
        ImageLabels {
            file: PathBuf::from("x.jpg"),
            name: "x.jpg".into(),
            split,
            width: 100,
            height: 100,
            objects: objects.into_iter().map(|(class, bbox)| Object { class, bbox, polygon: None }).collect(),
        }
    }

    fn set() -> LabelSet {
        let small = BBox { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
        let wide = BBox { x: 40.0, y: 45.0, w: 40.0, h: 10.0 };
        let big = BBox { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        LabelSet {
            classes: vec!["cat".into(), "dog".into()],
            images: vec![
                image(Some(Split::Train), vec![(0, small), (0, wide), (1, big)]),
                image(Some(Split::Train), vec![]),
                image(Some(Split::Val), vec![(1, small)]),
                image(None, vec![(0, wide)]),
            ],
            warnings: vec!["bad line".into()],
            declared_classes: Some(2),
        }
    }

    #[test]
    fn counts_classes_and_splits() {
        let stats = analyze(&set());
        assert_eq!((stats.image_count, stats.label_count), (4, 5));
        assert_eq!(stats.images, [2, 1, 0, 1]);
        assert_eq!(stats.empty_images, [1, 0, 0, 0]);
        assert_eq!(stats.classes[0].labels, [2, 0, 0, 1]);
        assert_eq!(stats.classes[0].images, 2, "two cats in one image count once");
        assert_eq!(stats.classes[1].total_labels(), 2);
        assert_eq!(stats.warning_count, 1);
    }

    #[test]
    fn fills_histograms_and_heatmap() {
        let stats = analyze(&set());
        assert_eq!(stats.labels_per_image.counts[0..4], [1, 2, 0, 1]);
        assert_eq!(stats.box_width.total(), 5);
        assert_eq!(stats.heatmap.iter().sum::<u32>(), 5, "one cell per box");
        // Small boxes centre at (5, 5) → cell (1, 1); wide ones at (60, 50) → (19, 16).
        assert_eq!(stats.heatmap[HEATMAP_SIZE + 1], 2);
        assert_eq!(stats.heatmap[16 * HEATMAP_SIZE + 19], 2);
        assert_eq!(stats.size_classes, [4, 0, 1], "10x10 and 40x10 are small, 100x100 is large");
        // 40x10 is 4x wide: log2 = 2 → bin (2 + 4) / 8 * 16 = 12.
        assert_eq!(stats.aspect_ratio.counts[12], 2);
        assert_eq!(stats.image_sizes, [ImageSize { width: 100, height: 100, count: 4 }]);
    }

    #[test]
    fn histogram_clamps_out_of_range() {
        let mut h = Histogram::linear(0.0, 1.0, 4);
        for v in [-1.0, 0.0, 0.3, 0.99, 1.0, 5.0, f64::NAN] {
            h.add(v);
        }
        assert_eq!(h.counts, [2, 1, 0, 3]);
        assert_eq!(h.edges, [0.0, 0.25, 0.5, 0.75, 1.0]);
    }

    #[test]
    fn filters_and_pages_gallery() {
        let set = set();
        let all = gallery_page(&set, &GalleryFilter::default(), 1, 2);
        assert_eq!((all.total, all.images.len(), all.images[0].index), (4, 2, 1));

        let dogs = gallery_page(&set, &GalleryFilter { class: Some(1), ..Default::default() }, 0, 10);
        assert_eq!(dogs.images.iter().map(|i| i.index).collect::<Vec<_>>(), [0, 2]);

        let unsplit = GalleryFilter { split: Some(None), ..Default::default() };
        assert_eq!(gallery_page(&set, &unsplit, 0, 10).total, 1);
        let empty = GalleryFilter { unlabeled: true, ..Default::default() };
        assert_eq!(gallery_page(&set, &empty, 0, 10).images[0].index, 1);

        let wide = &gallery_page(&set, &GalleryFilter::default(), 3, 1).images[0].objects[0];
        assert_eq!(wide.bbox, [0.4, 0.45, 0.4, 0.1]);
    }
}
