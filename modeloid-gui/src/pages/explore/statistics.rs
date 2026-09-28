//! Statistics tab: headline numbers, then one card per question.

use dioxus::prelude::*;
use modeloid_types::stats::{ExploreStats, Histogram, HEATMAP_SIZE, SPLITS};
use modeloid_types::Split;

use crate::components::{Bar, BarList, Heatmap, HistogramChart, StatTile};
use crate::util::format_count;

/// Classes shown before "Show all".
const CLASS_ROWS: usize = 15;

#[component]
pub fn Statistics(stats: ExploreStats, on_class: EventHandler<usize>) -> Element {
    let mut show_all = use_signal(|| false);

    let empty: u32 = stats.empty_images.iter().sum();
    let used_classes = stats.classes.iter().filter(|c| c.total_labels() > 0).count();
    let per_image = if stats.image_count > 0 { f64::from(stats.label_count) / f64::from(stats.image_count) } else { 0.0 };

    // Classes by label count; the bar index maps back to the class index.
    let mut order: Vec<usize> = (0..stats.classes.len()).filter(|&i| stats.classes[i].total_labels() > 0).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(stats.classes[i].total_labels()));
    let hidden = order.len().saturating_sub(CLASS_ROWS);
    let visible: Vec<usize> = if show_all() { order.clone() } else { order.iter().copied().take(CLASS_ROWS).collect() };
    let class_bars: Vec<Bar> = visible
        .iter()
        .map(|&i| {
            let c = &stats.classes[i];
            Bar { label: c.name.clone(), value: c.total_labels(), detail: Some(format!("labels in {} images", format_count(c.images))) }
        })
        .collect();
    let unused = stats.classes.len() - used_classes;
    // Tall only when the list is long, so short lists don't leave a gap beside them.
    let class_card = if class_bars.len() > 8 { "section-card chart-card chart-card-tall" } else { "section-card chart-card" };

    let split_bars: Vec<Bar> = SPLITS
        .iter()
        .enumerate()
        .filter(|(i, _)| stats.images[*i] > 0)
        .map(|(i, split)| Bar {
            label: Split::label(*split).to_string(),
            value: stats.images[i],
            detail: Some(format!("images, {} without labels", format_count(stats.empty_images[i]))),
        })
        .collect();

    let [small, medium, large] = stats.size_classes;
    let size_bars = vec![
        Bar { label: "Small".into(), value: small, detail: Some("objects under 32×32 px".into()) },
        Bar { label: "Medium".into(), value: medium, detail: Some("objects under 96×96 px".into()) },
        Bar { label: "Large".into(), value: large, detail: Some("objects of 96×96 px or more".into()) },
    ];

    let mut image_size_bars: Vec<Bar> = stats
        .image_sizes
        .iter()
        .take(10)
        .map(|s| Bar { label: format!("{}×{}", s.width, s.height), value: s.count, detail: Some("images".into()) })
        .collect();
    let rest: u32 = stats.image_sizes.iter().skip(10).map(|s| s.count).sum::<u32>() + stats.other_sizes;
    if rest > 0 {
        image_size_bars.push(Bar { label: "Other sizes".into(), value: rest, detail: Some("images".into()) });
    }

    rsx! {
        div { class: "stat-tiles",
            StatTile { label: "Images", value: format_count(stats.image_count) }
            StatTile { label: "Labels", value: format_count(stats.label_count), detail: format!("{per_image:.1} per image") }
            StatTile { label: "Classes", value: format_count(used_classes as u32), detail: (unused > 0).then(|| format!("+{unused} without labels")) }
            StatTile { label: "Without labels", value: format_count(empty), detail: "images".to_string() }
            if stats.polygon_count > 0 {
                StatTile { label: "Outlines", value: format_count(stats.polygon_count), detail: "segmentation polygons".to_string() }
            }
        }

        if stats.warning_count > 0 {
            details { class: "notice notice-warning explore-warnings",
                summary { "{format_count(stats.warning_count)} problems while reading labels" }
                ul {
                    for (i, warning) in stats.warnings.iter().enumerate() {
                        li { key: "{i}", code { "{warning}" } }
                    }
                }
                if stats.warning_count as usize > stats.warnings.len() {
                    p { class: "field-hint", "Showing the first {stats.warnings.len()}. Health Check will list them all." }
                }
            }
        }

        div { class: "chart-grid",
            section { class: class_card, aria_labelledby: "chart-classes",
                div { class: "section-card-header",
                    h3 { id: "chart-classes", class: "section-card-title", "Labels per class" }
                    span { class: "section-card-meta", "Click a class to see its images" }
                }
                if class_bars.is_empty() {
                    p { class: "field-hint", "No labels in this dataset." }
                } else {
                    BarList {
                        bars: class_bars,
                        label: "Labels per class",
                        on_select: move |row: usize| on_class.call(visible[row]),
                    }
                }
                if hidden > 0 {
                    button {
                        class: "button button-ghost button-sm",
                        r#type: "button",
                        onclick: move |_| show_all.toggle(),
                        if show_all() { "Show top {CLASS_ROWS}" } else { "Show all {order.len()} classes" }
                    }
                }
                if unused > 0 {
                    p { class: "field-hint", "{unused} classes have no labels." }
                }
            }

            section { class: "section-card chart-card", aria_labelledby: "chart-splits",
                h3 { id: "chart-splits", class: "section-card-title", "Images per split" }
                BarList { bars: split_bars, label: "Images per split" }
            }

            section { class: "section-card chart-card", aria_labelledby: "chart-per-image",
                h3 { id: "chart-per-image", class: "section-card-title", "Objects per image" }
                HistogramChart {
                    counts: stats.labels_per_image.counts.clone(),
                    bin_labels: count_bins(&stats.labels_per_image),
                    // Labels under the middle of their bins (21 bins: 0..19 and 20+).
                    ticks: [0, 5, 10, 15, 20].map(|n| ((n as f64 + 0.5) / 21.0, if n == 20 { "20+".to_string() } else { n.to_string() })).to_vec(),
                    label: "Histogram of objects per image",
                    unit: "images",
                }
            }

            section { class: "section-card chart-card", aria_labelledby: "chart-sizes",
                h3 { id: "chart-sizes", class: "section-card-title", "Object size" }
                BarList { bars: size_bars, label: "Objects by size (COCO small, medium, large)" }
                div { class: "chart-pair",
                    div {
                        p { class: "chart-caption", "Width, share of image" }
                        HistogramChart {
                            counts: stats.box_width.counts.clone(),
                            bin_labels: share_bins(&stats.box_width),
                            ticks: vec![(0.0, "0".into()), (0.5, "50%".into()), (1.0, "100%".into())],
                            label: "Histogram of box widths relative to image width",
                            unit: "objects",
                        }
                    }
                    div {
                        p { class: "chart-caption", "Height, share of image" }
                        HistogramChart {
                            counts: stats.box_height.counts.clone(),
                            bin_labels: share_bins(&stats.box_height),
                            ticks: vec![(0.0, "0".into()), (0.5, "50%".into()), (1.0, "100%".into())],
                            label: "Histogram of box heights relative to image height",
                            unit: "objects",
                        }
                    }
                }
            }

            section { class: "section-card chart-card", aria_labelledby: "chart-aspect",
                h3 { id: "chart-aspect", class: "section-card-title", "Box shape" }
                HistogramChart {
                    counts: stats.aspect_ratio.counts.clone(),
                    bin_labels: aspect_bins(&stats.aspect_ratio),
                    ticks: vec![(0.0, "tall".into()), (0.25, "1:4".into()), (0.5, "1:1".into()), (0.75, "4:1".into()), (1.0, "wide".into())],
                    label: "Histogram of box aspect ratios, from tall to wide",
                    unit: "objects",
                }
            }

            section { class: "section-card chart-card", aria_labelledby: "chart-heatmap",
                h3 { id: "chart-heatmap", class: "section-card-title", "Where objects are" }
                p { class: "chart-caption", "Box centres across all images, top left to bottom right." }
                Heatmap { cells: stats.heatmap.clone(), size: HEATMAP_SIZE, label: "Heatmap of box centre positions" }
            }

            section { class: "section-card chart-card", aria_labelledby: "chart-image-sizes",
                h3 { id: "chart-image-sizes", class: "section-card-title", "Image sizes" }
                BarList { bars: image_size_bars, label: "Most common image sizes" }
            }
        }
    }
}

/// "3 objects" style names for the objects-per-image bins; the last one is open-ended.
fn count_bins(h: &Histogram) -> Vec<String> {
    let last = h.counts.len().saturating_sub(1);
    (0..h.counts.len())
        .map(|i| match i {
            _ if i == last => format!("{}+ objects", h.edges[i] as u32),
            1 => "1 object".into(),
            _ => format!("{} objects", h.edges[i] as u32),
        })
        .collect()
}

fn share_bins(h: &Histogram) -> Vec<String> {
    (0..h.counts.len()).map(|i| format!("{:.0}–{:.0}%", h.edges[i] * 100.0, h.edges[i + 1] * 100.0)).collect()
}

/// Bins are `log2(width / height)`; shown as width:height ratios.
fn aspect_bins(h: &Histogram) -> Vec<String> {
    let ratio = |v: f64| {
        let r = v.exp2();
        if r >= 1.0 { format!("{r:.1}:1") } else { format!("1:{:.1}", 1.0 / r) }
    };
    (0..h.counts.len()).map(|i| format!("{} to {}", ratio(h.edges[i]), ratio(h.edges[i + 1]))).collect()
}
