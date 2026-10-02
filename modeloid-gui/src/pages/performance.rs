//! Training → Performance: how a trained model did, from what Ultralytics
//! saved in its run's `runs/` folder (results.csv, plots, batches, weights).
//! The side panel opens on the Models table; the model selected last shows.

use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::jobs::JobKind;
use crate::api::train::{column_title, run_results, ImageGroup, RunImage, RunResults, TrainStatus};
use crate::components::{format_value, status_label, use_job, LineChart, LineSeries, NoWorkspace, SelectModelHint, StatTile};
use crate::state::{use_models_version, use_selected_models, use_workspace, use_workspace_checked};
use crate::util::{encode_url_path, format_bytes};

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
    let fetch_name = name.clone();
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
    let charts = chart_specs(&results);
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
                span { class: "section-card-meta", "{record.model} on {record.dataset_name} · {record.device} · started {record.started}" }
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
                        rsx! {
                            StatTile {
                                key: "{column}",
                                label: column_title(column),
                                value: format_value(at_best),
                                detail: Some(format!("best epoch · last {}", format_value(last))),
                            }
                        }
                    }
                }
            }

            section { class: "section-card", aria_labelledby: "perf-curves-title",
                div { class: "section-card-header",
                    h2 { id: "perf-curves-title", class: "section-card-title", "Per epoch" }
                    span { class: "section-card-meta",
                        "from runs/results.csv"
                        if best_epoch.is_some() { " · the line marks the best epoch" }
                    }
                }
                div { class: "chart-grid perf-chart-grid",
                    for chart in charts {
                        {
                            let series: Vec<LineSeries> = chart
                                .columns
                                .iter()
                                .map(|(label, column, dashed)| LineSeries {
                                    label: label.to_string(),
                                    values: results.column(column).unwrap_or_default(),
                                    dashed: *dashed,
                                })
                                .collect();
                            rsx! {
                                div { key: "{chart.title}", class: "perf-chart",
                                    h3 { class: "chart-caption perf-chart-title", "{chart.title}" }
                                    LineChart { x: epochs.clone(), series, label: "{chart.title} per epoch", x_name: "Epoch", marker: best }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Like Explore's gallery: tiles, and a viewer that steps through all of them.
        for group in groups {
            section { key: "{group:?}", class: "section-card", aria_label: "{group.label()}",
                h2 { class: "section-card-title", "{group.label()}" }
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
                h2 { id: "perf-weights-title", class: "section-card-title", "Weights" }
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

/// One small chart: its title and its lines as (legend label, column, dashed).
struct ChartSpec {
    title: String,
    columns: Vec<(&'static str, String, bool)>,
}

/// Losses (train solid, val dashed, same chart), then each metric, then the
/// learning rate.
fn chart_specs(results: &RunResults) -> Vec<ChartSpec> {
    let mut charts = Vec::new();
    for column in results.columns.iter().filter(|c| c.starts_with("train/")) {
        let val = format!("val/{}", &column["train/".len()..]);
        let mut columns = vec![("train", column.clone(), false)];
        if results.columns.contains(&val) {
            columns.push(("val", val, true));
        }
        charts.push(ChartSpec { title: column_title(column), columns });
    }
    // Validation losses without a training one.
    for column in results.columns.iter().filter(|c| c.starts_with("val/")) {
        if !results.columns.contains(&format!("train/{}", &column["val/".len()..])) {
            charts.push(ChartSpec { title: format!("{} (val)", column_title(column)), columns: vec![("val", column.clone(), true)] });
        }
    }
    for column in results.columns.iter().filter(|c| c.starts_with("metrics/")) {
        charts.push(ChartSpec { title: column_title(column), columns: vec![("value", column.clone(), false)] });
    }
    if let Some(lr) = results.columns.iter().find(|c| c.starts_with("lr/")) {
        charts.push(ChartSpec { title: "Learning rate".into(), columns: vec![("value", lr.clone(), false)] });
    }
    charts
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
        assert_eq!(charts[0].columns[1], ("val", "val/box_loss".to_string(), true));
    }
}
