//! Small charts, built from HTML and SVG (no chart library): bar lists,
//! histograms, a heatmap and line charts. One data hue (`--viz-series`), a
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

/// One line of a `LineChart`.
#[derive(Debug, Clone, PartialEq)]
pub struct LineSeries {
    /// e.g. "train", "val"; named in the legend and the tooltips.
    pub label: String,
    /// One value per x; NaN leaves a gap.
    pub values: Vec<f64>,
    /// Second line of a pair (e.g. validation): dashed, same hue, so the
    /// legend's words and the dash tell them apart, not a colour.
    pub dashed: bool,
}

/// Short number for axes and tooltips: 3 significant-ish digits.
pub fn format_value(v: f64) -> String {
    let a = v.abs();
    if !v.is_finite() {
        "–".into()
    } else if v.fract() == 0.0 && a < 1e9 {
        // Whole numbers (epochs, counts) without decimals.
        format!("{v:.0}")
    } else if a >= 100.0 {
        format!("{v:.0}")
    } else if a >= 1.0 {
        format!("{v:.2}")
    } else if a >= 0.001 {
        format!("{v:.3}")
    } else {
        format!("{v:.1e}")
    }
}

/// Lines over `x` (e.g. epochs), y scaled to the values' range. Each x has a
/// hover column with every series' value; `marker` draws a line at one x
/// (e.g. the best epoch); `reference` a dotted line at one y, named in the
/// legend (e.g. the final score).
#[component]
pub fn LineChart(
    x: Vec<f64>,
    series: Vec<LineSeries>,
    #[props(into)] label: String,
    /// Name of x in tooltips, e.g. "Epoch".
    #[props(into, default = "".to_string())]
    x_name: String,
    #[props(default)] marker: Option<usize>,
    #[props(default)] reference: Option<(String, f64)>,
) -> Element {
    let n = x.len();
    let reference = reference.filter(|(_, v)| v.is_finite());
    let finite = series.iter().flat_map(|s| s.values.iter().copied()).chain(reference.as_ref().map(|r| r.1)).filter(|v| v.is_finite());
    let (mut lo, mut hi) = finite.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
    if !lo.is_finite() {
        (lo, hi) = (0.0, 1.0);
    } else if hi - lo < 1e-12 {
        (lo, hi) = (lo - 0.5, hi + 0.5);
    }
    // x across 0..100, y from 100 (lowest) to 0 (highest), a little air.
    let px = |i: usize| if n > 1 { i as f64 / (n - 1) as f64 * 100.0 } else { 50.0 };
    let py = |v: f64| 96.0 - (v - lo) / (hi - lo) * 92.0;
    // A polyline per unbroken run of values.
    let paths: Vec<(bool, Vec<Vec<(f64, f64)>>)> = series
        .iter()
        .map(|s| {
            let mut runs = Vec::new();
            let mut run = Vec::new();
            for (i, v) in s.values.iter().enumerate().take(n) {
                if v.is_finite() {
                    run.push((px(i), py(*v)));
                } else if !run.is_empty() {
                    runs.push(std::mem::take(&mut run));
                }
            }
            if !run.is_empty() {
                runs.push(run);
            }
            (s.dashed, runs)
        })
        .collect();
    let first = x.first().copied().map(format_value).unwrap_or_default();
    let last = x.last().copied().map(format_value).unwrap_or_default();
    let legend = series.len() > 1 || reference.is_some();

    rsx! {
        figure { class: "line-chart", role: "img", aria_label: "{label}",
            div { class: "line-chart-plot",
                span { class: "line-chart-y line-chart-y-top", aria_hidden: "true", "{format_value(hi)}" }
                span { class: "line-chart-y line-chart-y-bottom", aria_hidden: "true", "{format_value(lo)}" }
                svg { class: "line-chart-svg", view_box: "0 0 100 100", preserve_aspect_ratio: "none", "aria-hidden": "true",
                    if let Some(m) = marker.filter(|m| *m < n) {
                        line { class: "line-chart-marker", x1: "{px(m):.2}", x2: "{px(m):.2}", y1: "0", y2: "100" }
                    }
                    if let Some((_, v)) = &reference {
                        line { class: "line-chart-reference", x1: "0", x2: "100", y1: "{py(*v):.2}", y2: "{py(*v):.2}" }
                    }
                    for (si, (dashed, runs)) in paths.iter().enumerate() {
                        for (ri, run) in runs.iter().enumerate() {
                            if let [(cx, cy)] = run.as_slice() {
                                // A lone value: a dot instead of a line.
                                circle { key: "{si}-{ri}", class: "line-chart-dot", cx: "{cx:.2}", cy: "{cy:.2}", r: "1.2" }
                            } else {
                                polyline {
                                    key: "{si}-{ri}",
                                    class: if *dashed { "line-chart-line is-dashed" } else { "line-chart-line" },
                                    points: run.iter().map(|(x, y)| format!("{x:.2},{y:.2}")).collect::<Vec<_>>().join(" "),
                                }
                            }
                        }
                    }
                }
                div { class: "line-chart-hits",
                    for i in 0..n {
                        {
                            let values: Vec<String> = series
                                .iter()
                                .map(|s| {
                                    let v = s.values.get(i).copied().unwrap_or(f64::NAN);
                                    if legend { format!("{} {}", s.label, format_value(v)) } else { format_value(v) }
                                })
                                .chain(reference.iter().map(|(name, v)| format!("{name} {}", format_value(*v))))
                                .collect();
                            let tip = format!("{x_name} {}: {}", format_value(x[i]), values.join(", "));
                            rsx! { div { key: "{i}", class: "line-chart-hit", "data-tip": "{tip.trim()}" } }
                        }
                    }
                }
            }
            div { class: "line-chart-x", aria_hidden: "true",
                span { "{first}" }
                span { "{last}" }
            }
            if legend {
                div { class: "line-chart-legend",
                    for s in series.iter() {
                        span { key: "{s.label}", class: "line-chart-key",
                            span { class: if s.dashed { "line-chart-swatch is-dashed" } else { "line-chart-swatch" } }
                            "{s.label}"
                        }
                    }
                    if let Some((name, v)) = &reference {
                        span { class: "line-chart-key",
                            span { class: "line-chart-swatch is-reference" }
                            "{name} {format_value(*v)}"
                        }
                    }
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_chart_values() {
        assert_eq!(format_value(24.0), "24");
        assert_eq!(format_value(1.2345), "1.23");
        assert_eq!(format_value(0.55512), "0.555");
        assert_eq!(format_value(0.0001), "1.0e-4");
        assert_eq!(format_value(f64::NAN), "–");
    }
}
