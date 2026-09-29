//! Small charts for Explore, built from HTML (no chart library): bar lists,
//! histograms and a heatmap. One data hue (`--viz-series`), a blue
//! sequential ramp for the heatmap, hover tooltips from `data-tip`.

use dioxus::prelude::*;

use crate::util::format_count;

/// One row of a `BarList`.
#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    pub label: String,
    pub value: u32,
    /// Extra text for the tooltip, e.g. "in 12 images".
    pub detail: Option<String>,
}

/// Horizontal bars, one per row, largest first as given. With `on_select`,
/// rows are buttons (e.g. click a class to see its images).
#[component]
pub fn BarList(
    bars: Vec<Bar>,
    /// Accessible name, e.g. "Labels per class".
    #[props(into)]
    label: String,
    #[props(default)] on_select: Option<EventHandler<usize>>,
    /// Row shown as selected.
    #[props(default)]
    selected: Option<usize>,
) -> Element {
    let max = bars.iter().map(|b| b.value).max().unwrap_or(0).max(1);

    rsx! {
        ul { class: "bar-list", aria_label: "{label}",
            for (i, bar) in bars.into_iter().enumerate() {
                {
                    let width = f64::from(bar.value) / f64::from(max) * 100.0;
                    let tip = match &bar.detail {
                        Some(detail) => format!("{}: {} {detail}", bar.label, format_count(bar.value)),
                        None => format!("{}: {}", bar.label, format_count(bar.value)),
                    };
                    let row_class = if selected == Some(i) { "bar-row is-selected" } else { "bar-row" };
                    let body = rsx! {
                        span { class: "bar-label", "{bar.label}" }
                        span { class: "bar-track",
                            span { class: "bar-fill", style: "width: {width:.2}%" }
                        }
                        span { class: "bar-value", "{format_count(bar.value)}" }
                    };
                    rsx! {
                        li { key: "{i}",
                            match on_select {
                                Some(handler) => rsx! {
                                    button {
                                        class: "{row_class}",
                                        r#type: "button",
                                        "data-tip": "{tip}",
                                        aria_pressed: if selected == Some(i) { "true" } else { "false" },
                                        onclick: move |_| handler.call(i),
                                        {body}
                                    }
                                },
                                None => rsx! {
                                    div { class: "{row_class}", "data-tip": "{tip}", {body} }
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Vertical bars over consecutive bins. `bin_labels` name each bin (tooltip);
/// `ticks` are axis labels at positions from 0 (left edge) to 1 (right edge).
#[component]
pub fn HistogramChart(
    counts: Vec<u32>,
    bin_labels: Vec<String>,
    ticks: Vec<(f64, String)>,
    #[props(into)] label: String,
    /// Word for the counted things in tooltips, e.g. "objects".
    #[props(into, default = "".to_string())]
    unit: String,
) -> Element {
    let max = counts.iter().copied().max().unwrap_or(0).max(1);

    rsx! {
        figure { class: "histogram", role: "img", aria_label: "{label}",
            div { class: "histogram-bars",
                for (i, count) in counts.iter().enumerate() {
                    {
                        let height = f64::from(*count) / f64::from(max) * 100.0;
                        let name = bin_labels.get(i).cloned().unwrap_or_default();
                        rsx! {
                            div {
                                key: "{i}",
                                class: "histogram-bin",
                                "data-tip": "{name}: {format_count(*count)} {unit}",
                                div { class: if *count > 0 { "histogram-bar" } else { "histogram-bar is-zero" }, style: "height: {height:.2}%" }
                            }
                        }
                    }
                }
            }
            div { class: "histogram-axis", aria_hidden: "true",
                for (pos, text) in ticks.iter() {
                    span { key: "{text}", class: "histogram-tick", style: "left: {pos * 100.0:.2}%", "{text}" }
                }
            }
        }
    }
}

/// Sequential steps, near the surface (fewest) to strongest (most). Colours
/// come from the theme (`--viz-ramp-*` in themes.css).
const RAMP: [&str; 6] = [
    "var(--viz-ramp-1)",
    "var(--viz-ramp-2)",
    "var(--viz-ramp-3)",
    "var(--viz-ramp-4)",
    "var(--viz-ramp-5)",
    "var(--viz-ramp-6)",
];

/// Square grid of counts (`size` × `size`, row by row). Empty cells stay the
/// surface colour; others step through the ramp by their share of the maximum.
#[component]
pub fn Heatmap(cells: Vec<u32>, size: usize, #[props(into)] label: String) -> Element {
    let max = cells.iter().copied().max().unwrap_or(0).max(1);
    let cell_pct = 100.0 / size.max(1) as f64;

    rsx! {
        figure { class: "heatmap", role: "img", aria_label: "{label}",
            div { class: "heatmap-grid", style: "grid-template-columns: repeat({size}, 1fr)",
                for (i, count) in cells.iter().enumerate() {
                    {
                        let fill = if *count == 0 {
                            "transparent"
                        } else {
                            // Square root spreads the few busy cells from the many quiet ones.
                            let share = (f64::from(*count) / f64::from(max)).sqrt();
                            RAMP[((share * (RAMP.len() - 1) as f64).round() as usize).min(RAMP.len() - 1)]
                        };
                        let (row, col) = (i / size, i % size);
                        let (x0, y0) = (col as f64 * cell_pct, row as f64 * cell_pct);
                        rsx! {
                            div {
                                key: "{i}",
                                class: "heatmap-cell",
                                style: "background: {fill}",
                                "data-tip": "{format_count(*count)} centres at {x0:.0}–{x0 + cell_pct:.0}% across, {y0:.0}–{y0 + cell_pct:.0}% down",
                            }
                        }
                    }
                }
            }
            div { class: "heatmap-legend", aria_hidden: "true",
                span { "fewer" }
                for color in RAMP {
                    span { key: "{color}", class: "heatmap-swatch", style: "background: {color}" }
                }
                span { "more" }
            }
        }
    }
}

/// Big number with a caption, for headline figures.
#[component]
pub fn StatTile(#[props(into)] label: String, #[props(into)] value: String, #[props(default)] detail: Option<String>) -> Element {
    rsx! {
        div { class: "stat-tile",
            span { class: "stat-label", "{label}" }
            span { class: "stat-value", "{value}" }
            if let Some(detail) = detail {
                span { class: "stat-detail", "{detail}" }
            }
        }
    }
}
