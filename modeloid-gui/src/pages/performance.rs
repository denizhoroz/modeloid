//! Training → Performance: how a trained model did, from what Ultralytics
//! saved in its run's `runs/` folder (results.csv, plots, batches, weights)
//! and what the run measured at its end (final scores, speed, size, memory).
//! The side panel opens on the Models table; the model selected last shows.

use dioxus::prelude::*;

use crate::api::error_message;
use crate::components::{IconName, TitleIcon};
use crate::api::jobs::JobKind;
use crate::api::train::{column_title, run_results, ImageGroup, RunImage, RunResults, TrainRecord, TrainStatus};
use crate::components::{format_value, status_label, use_job, LineChart, LineSeries, NoWorkspace, SelectModelHint, StatTile};
use crate::state::{use_models_version, use_selected_models, use_workspace, use_workspace_checked};
use crate::util::{encode_url_path, format_bytes, format_count, short_time};
use modeloid_types::parameters::Task;

#[component]
pub fn Performance() -> Element {
    let workspace = use_workspace();
    let checked = use_workspace_checked();
    let selected = use_selected_models();
    let model = selected.read().last().cloned();

    rsx! {
        document::Title { "Performance · Modeloid" }
        div { class: "page page-wide",
            p { class: "page-eyebrow", "Training" }
            h1 { "Performance" }
            p { class: "page-lead",
                "How a trained model did: its metrics and losses per epoch, and the plots and example batches saved with it."
            }
            match (workspace.read().is_some(), model) {
                (true, Some(name)) => rsx! { Results { key: "{name}", name } },
                (true, None) => rsx! { SelectModelHint {} },
                (false, _) if checked() => rsx! { NoWorkspace { text: "Trained models are kept in a workspace." } },
                _ => rsx! {},
            }
        }
    }
}

/// Run `name`'s results; read again when the models change and after each
/// epoch while it trains.
#[component]
fn Results(name: String) -> Element {
    let models_version = use_models_version();
    let (job, _) = use_job(JobKind::Train);
    let run_name = name.clone();
    // Changes as this model's job moves on (an epoch ends, the run ends).
    let job_step = use_memo(move || {
        job.read().as_ref().filter(|j| j.items.first().is_some_and(|i| i.label == run_name)).map(|j| (j.version, j.finished))
    });
    let fetch_name = name;
    let results = use_resource(move || {
        let name = fetch_name.clone();
        async move {
            let _ = (models_version(), job_step());
            run_results(name).await
        }
    });
    // The last answer stays shown while the next one loads.
    let mut shown = use_signal(|| None::<RunResults>);
    use_effect(move || {
        if let Some(Ok(r)) = &*results.read() {
            shown.set(Some(r.clone()));
        }
    });
    let error = match &*results.read() {
        Some(Err(e)) => Some(error_message(e)),
        _ => None,
    };

    rsx! {
        if let Some(message) = error {
            p { class: "form-error", role: "alert", "{message}" }
        }
        match shown() {
            Some(r) => rsx! { Report { results: r } },
            None => rsx! { p { class: "field-hint", aria_live: "polite", "Reading the run's results…" } },
        }
    }
}

#[component]
fn Report(results: RunResults) -> Element {
    // The picture open in the viewer, by its place in `results.images`.
    let mut open = use_signal(|| None::<usize>);
    let record = &results.record;
    let status = status_label(record.status);
    let epochs = results.column("epoch").unwrap_or_else(|| (1..=results.rows.len()).map(|e| e as f64).collect());
    let best = results.best_row();
    let best_epoch = best.and_then(|b| epochs.get(b).copied());
    let metrics: Vec<&String> = results.columns.iter().filter(|c| c.starts_with("metrics/")).collect();
    let summary = record.summary.clone().unwrap_or_default();
    let charts = chart_specs(&results);
    let has_final = !summary.final_metrics.is_empty();
    let facts = run_facts(&results);
    let groups: Vec<ImageGroup> = {
        let mut g: Vec<ImageGroup> = results.images.iter().map(|i| i.group).collect();
        g.dedup();
        g
    };
    let base = format!("/files/run/{}", encode_url_path(&record.name, false));
    let nothing_yet = results.rows.is_empty() && results.images.is_empty();

    rsx! {
        div { class: "explore-head",
            div { class: "explore-title",
                h2 { class: "explore-name", "{record.name}" }
                span { class: "run-status is-{status}", "{status}" }
                span { class: "section-card-meta", "{record.model} on {record.dataset_name}" }
            }
        }
        if record.status == TrainStatus::Running {
            div { class: "notice", role: "status",
                p { "Still training: the results below grow after each epoch." }
            }
        }
        if let Some(error) = &record.error {
            div { class: "notice notice-error", role: "alert", p { "{error}" } }
        }
        if !results.warnings.is_empty() {
            div { class: "notice notice-warning", role: "status",
                for w in results.warnings.iter() {
                    p { key: "{w}", "{w}" }
                }
            }
        }
        if nothing_yet {
            div { class: "empty-state",
                p { "Nothing saved yet. Results appear here after the first epoch." }
            }
        }

        if !results.rows.is_empty() {
            div { class: "stat-tiles",
                StatTile {
                    label: "Epochs",
                    value: format!("{}/{}", results.rows.len(), record.epochs.max(results.rows.len() as u32)),
                    detail: best_epoch.map(|e| format!("best: epoch {}", format_value(e))),
                }
                for column in metrics.iter() {
                    {
                        let values = results.column(column).unwrap_or_default();
                        let at_best = best.and_then(|b| values.get(b).copied()).unwrap_or(f64::NAN);
                        let last = values.last().copied().unwrap_or(f64::NAN);
                        // The official score is the final validation of best.pt, when the run got that far.
                        let (value, detail) = match summary.final_metrics.get(column.as_str()) {
                            Some(v) => (*v, format!("final, best.pt · last epoch {}", format_value(last))),
                            None => (at_best, format!("best epoch · last {}", format_value(last))),
                        };
                        rsx! {
                            StatTile {
                                key: "{column}",
                                label: column_title(column),
                                value: format_value(value),
                                detail: Some(detail),
                            }
                        }
                    }
                }
                if let Some(ms) = summary.inference_ms() {
                    StatTile {
                        label: "Inference",
                        value: format!("{ms:.1} ms"),
                        detail: Some(match summary.speed.get("inference") {
                            Some(model) => format!("per image · model {model:.1} ms"),
                            None => "per image".to_string(),
                        }),
                    }
                }
                if let Some(parameters) = summary.parameters {
                    StatTile {
                        label: "Parameters",
                        value: format_millions(parameters),
                        detail: Some(
                            [summary.layers.map(|l| format!("{} layers", format_count(l))), summary.gflops.map(|g| format!("{g:.1} GFLOPs"))]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(" · "),
                        ),
                    }
                }
                match (summary.peak_gpu_bytes, summary.peak_ram_bytes) {
                    (Some(gpu), ram) => rsx! {
                        StatTile { label: "Peak GPU memory", value: format_bytes(gpu), detail: ram.map(|r| format!("RAM {}", format_bytes(r))) }
                    },
                    (None, Some(ram)) => rsx! {
                        StatTile { label: "Peak RAM", value: format_bytes(ram), detail: Some("training process".to_string()) }
                    },
                    (None, None) => rsx! {},
                }
            }
        }

        section { class: "section-card", aria_labelledby: "perf-run-title",
            h2 { id: "perf-run-title", class: "section-card-title", TitleIcon { name: IconName::Activity } "Run" }
            dl { class: "perf-facts",
                for (label, value) in facts {
                    div { key: "{label}", dt { "{label}" } dd { "{value}" } }
                }
            }
        }

        if !results.rows.is_empty() {

            section { class: "section-card", aria_labelledby: "perf-curves-title",
                div { class: "section-card-header",
                    h2 { id: "perf-curves-title", class: "section-card-title", TitleIcon { name: IconName::TrendingUp } "Per epoch" }
                    span { class: "section-card-meta",
                        "from runs/results.csv"
                        if best_epoch.is_some() { " · the line marks the best epoch" }
                        if has_final { " · dotted: final score of best.pt" }
                    }
                }
                div { class: "chart-grid perf-chart-grid",
                    for chart in charts {
                        div { key: "{chart.title}", class: "perf-chart",
                            h3 { class: "chart-caption perf-chart-title", "{chart.title}" }
                            LineChart {
                                x: epochs.clone(),
                                series: chart.series,
                                label: "{chart.title} per epoch",
                                x_name: "Epoch",
                                marker: best,
                                reference: chart.reference,
                            }
                        }
                    }
                }
            }
        }

        // Like Explore's gallery: tiles, and a viewer that steps through all of them.
        for group in groups {
            section { key: "{group:?}", class: "section-card", aria_label: "{group.label()}",
                h2 { class: "section-card-title", TitleIcon { name: IconName::TrendingUp } "{group.label()}" }
                ul { class: "gallery-grid perf-gallery",
                    for (i, image) in results.images.iter().enumerate().filter(|(_, i)| i.group == group) {
                        li { key: "{image.path}",
                            button {
                                class: "gallery-tile",
                                r#type: "button",
                                title: "{image.title}",
                                onclick: move |_| open.set(Some(i)),
                                div { class: "gallery-thumb perf-thumb",
                                    img { src: image_url(&base, image), alt: "{image.title}", loading: "lazy", decoding: "async" }
                                }
                                span { class: "gallery-caption",
                                    span { class: "gallery-name", "{image.title}" }
                                    span { class: "gallery-meta", "{image.path}" }
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(i) = open() {
            if let Some(image) = results.images.get(i).cloned() {
                RunImageDetail {
                    src: image_url(&base, &image),
                    image,
                    position: format!("{} of {}", i + 1, results.images.len()),
                    on_close: move |_| open.set(None),
                    on_step: {
                        let count = results.images.len();
                        move |step: isize| {
                            let next = i as isize + step;
                            if next >= 0 && (next as usize) < count {
                                open.set(Some(next as usize));
                            }
                        }
                    },
                }
            }
        }

        if !results.weights.is_empty() {
            section { class: "section-card", aria_labelledby: "perf-weights-title",
                h2 { id: "perf-weights-title", class: "section-card-title", TitleIcon { name: IconName::Package } "Weights" }
                ul { class: "perf-weights",
                    for w in results.weights.iter() {
                        li { key: "{w.path}",
                            code { "training/trains/{record.name}/runs/{w.path}" }
                            span { class: "section-card-meta", "{format_bytes(w.size)}" }
                        }
                    }
                }
            }
        }
    }
}

fn image_url(base: &str, image: &RunImage) -> String {
    format!("{base}/{}", encode_url_path(&image.path, true))
}

/// One picture as large as the window allows, like Explore's image detail.
/// Esc closes, ←/→ step through the run's pictures.
#[component]
fn RunImageDetail(image: RunImage, src: String, position: String, on_close: EventHandler<()>, on_step: EventHandler<isize>) -> Element {
    rsx! {
        div { class: "scrim scrim-modal", onclick: move |_| on_close.call(()) }
        div {
            class: "image-detail",
            role: "dialog",
            aria_modal: "true",
            aria_label: "{image.title}",
            tabindex: "-1",
            onmounted: move |e| async move {
                let _ = e.set_focus(true).await;
            },
            onkeydown: move |e| match e.key() {
                Key::Escape => on_close.call(()),
                Key::ArrowLeft => on_step.call(-1),
                Key::ArrowRight => on_step.call(1),
                _ => {}
            },
            div { class: "image-detail-head",
                div { class: "image-detail-title",
                    h2 { class: "section-card-title", title: "{image.title}", "{image.title}" }
                    span { class: "section-card-meta", "{position} · {image.group.label()} · runs/{image.path}" }
                }
                a { class: "button button-ghost button-sm", href: "{src}", target: "_blank", rel: "noopener", "Open full size" }
                button { class: "button button-secondary button-sm", r#type: "button", aria_label: "Previous image", onclick: move |_| on_step.call(-1), "←" }
                button { class: "button button-secondary button-sm", r#type: "button", aria_label: "Next image", onclick: move |_| on_step.call(1), "→" }
                button { class: "button button-sm", r#type: "button", onclick: move |_| on_close.call(()), "Close" }
            }
            div { class: "image-detail-body",
                img { class: "perf-detail-image", src: "{src}", alt: "{image.title}" }
            }
        }
    }
}

/// One small chart: its title, its lines and an optional level line (the
/// final score).
struct ChartSpec {
    title: String,
    series: Vec<LineSeries>,
    reference: Option<(String, f64)>,
}

/// Losses (train solid, val dashed, same chart), then each metric (with the
/// final score of best.pt), then the time per epoch and the learning rate.
fn chart_specs(results: &RunResults) -> Vec<ChartSpec> {
    let line = |label: &str, values: Vec<f64>, dashed: bool| LineSeries { label: label.to_string(), values, dashed };
    let column = |name: &str| results.column(name).unwrap_or_default();
    let chart = |title: String, series: Vec<LineSeries>| ChartSpec { title, series, reference: None };
    let finals = results.record.summary.as_ref().map(|s| &s.final_metrics);

    let mut charts = Vec::new();
    for train in results.columns.iter().filter(|c| c.starts_with("train/")) {
        let val = format!("val/{}", &train["train/".len()..]);
        let mut series = vec![line("train", column(train), false)];
        if results.columns.contains(&val) {
            series.push(line("val", column(&val), true));
        }
        charts.push(chart(column_title(train), series));
    }
    // Validation losses without a training one.
    for val in results.columns.iter().filter(|c| c.starts_with("val/")) {
        if !results.columns.contains(&format!("train/{}", &val["val/".len()..])) {
            charts.push(chart(format!("{} (val)", column_title(val)), vec![line("val", column(val), true)]));
        }
    }
    for metric in results.columns.iter().filter(|c| c.starts_with("metrics/")) {
        let reference = finals.and_then(|f| f.get(metric.as_str())).map(|v| ("final (best.pt)".to_string(), *v));
        charts.push(ChartSpec { title: column_title(metric), series: vec![line("value", column(metric), false)], reference });
    }
    if let Some(seconds) = epoch_seconds(results) {
        charts.push(chart("Seconds per epoch".into(), vec![line("value", seconds, false)]));
    }
    if let Some(lr) = results.columns.iter().find(|c| c.starts_with("lr/")) {
        charts.push(chart("Learning rate".into(), vec![line("value", column(lr), false)]));
    }
    charts
}

/// How long each epoch took: Ultralytics' `time` column counts seconds since
/// training started.
fn epoch_seconds(results: &RunResults) -> Option<Vec<f64>> {
    let total = results.column("time").filter(|t| t.iter().any(|v| v.is_finite()))?;
    let mut before = 0.0;
    Some(
        total
            .iter()
            .map(|t| {
                let took = t - before;
                if t.is_finite() {
                    before = *t;
                }
                took
            })
            .collect(),
    )
}

/// The run's facts for the Run card: what ran and how long it took.
fn run_facts(results: &RunResults) -> Vec<(&'static str, String)> {
    let record: &TrainRecord = &results.record;
    let task = Task::ALL.iter().find(|t| t.id() == record.task).map_or_else(|| record.task.clone(), |t| t.label().to_string());
    let mut facts = vec![
        ("Task", task),
        ("Parameter set", record.parameters.clone()),
        ("Device", record.device_name.clone().unwrap_or_else(|| record.device.clone())),
        ("Started", short_time(&record.started)),
    ];
    match &record.finished {
        Some(finished) => {
            facts.push(("Finished", short_time(finished)));
            if let (Some(start), Some(end)) = (unix_seconds(&record.started), unix_seconds(finished)) {
                facts.push(("Total duration", format_duration((end - start) as f64)));
            }
        }
        None if record.status == TrainStatus::Running => facts.push(("Finished", "Still training".into())),
        None => {}
    }
    // Epochs only: the whole run also loads the data and validates best.pt at the end.
    if let Some(seconds) = epoch_seconds(results) {
        if let Some(total) = results.column("time").and_then(|t| t.into_iter().rev().find(|v| v.is_finite())) {
            facts.push(("Training time", format_duration(total)));
        }
        let done: Vec<f64> = seconds.into_iter().filter(|s| s.is_finite()).collect();
        if !done.is_empty() {
            facts.push(("Time per epoch", format!("{:.1} s on average", done.iter().sum::<f64>() / done.len() as f64)));
        }
    }
    facts
}

/// Seconds since 1970 of a UTC time as runs save it ("2026-10-07T10:00:54Z").
fn unix_seconds(time: &str) -> Option<i64> {
    let (date, clock) = time.trim_end_matches('Z').split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut c = clock.get(..8)?.split(':').map(|p| p.parse::<i64>().ok());
    let (h, min, s) = (c.next()??, c.next()??, c.next()??);
    // Days since 1970 of a calendar date (H. Hinnant's days_from_civil).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + h * 3600 + min * 60 + s)
}

/// "45 s", "12 min 5 s", "1 h 4 min".
fn format_duration(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min {} s", s / 60, s % 60),
        _ => format!("{} h {} min", s / 3600, s % 3600 / 60),
    }
}

/// "2.51M", "870K", "950".
fn format_millions(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.0}K", n as f64 / 1e3),
        _ => format!("{:.2}M", n as f64 / 1e6),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_train_and_val_losses() {
        let results = RunResults {
            columns: ["epoch", "train/box_loss", "train/cls_loss", "metrics/mAP50(B)", "val/box_loss", "val/cls_loss", "lr/pg0", "lr/pg1"]
                .map(String::from)
                .to_vec(),
            ..Default::default()
        };
        let charts = chart_specs(&results);
        let titles: Vec<&str> = charts.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Box loss", "Class loss", "mAP50 (boxes)", "Learning rate"]);
        assert_eq!((charts[0].series[1].label.as_str(), charts[0].series[1].dashed), ("val", true));
    }

    #[test]
    fn times_epochs_and_runs() {
        let results = RunResults {
            columns: vec!["epoch".into(), "time".into()],
            rows: vec![vec![1.0, 100.0], vec![2.0, 180.0], vec![3.0, 270.0]],
            ..Default::default()
        };
        assert_eq!(epoch_seconds(&results), Some(vec![100.0, 80.0, 90.0]));
        assert_eq!(unix_seconds("2026-10-07T10:43:50Z").unwrap() - unix_seconds("2026-10-07T10:00:54Z").unwrap(), 2576);
        assert_eq!(unix_seconds("1970-01-02T00:00:00Z"), Some(86_400));
        assert_eq!(format_duration(2576.0), "42 min 56 s");
        assert_eq!(format_duration(3720.0), "1 h 2 min");
        assert_eq!(format_millions(2_505_750), "2.51M");
    }
}
