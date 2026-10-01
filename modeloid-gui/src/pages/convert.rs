//! Dataset Manager > Convert: write the selected datasets in another label
//! format (YOLO, COCO, Pascal VOC), with boxes only, or as classification
//! crops (one image per object, in a folder per class). Several selected
//! datasets are converted with one plan, each to its own result.

use dioxus::prelude::*;

use crate::api::convert::{
    convert_batch_preview, convert_preview, start_convert, start_convert_batch, BatchOutput, ConvertPlan, ConvertPreview, TargetTask, DEFAULT_MIN_CROP,
    MAX_PADDING,
};
use crate::api::datasets::{list_datasets, validate_dataset_name, DatasetRow};
use crate::api::error_message;
use crate::api::jobs::JobKind;
use crate::components::{
    set_if_newer, use_job, Bar, BarList, JobProgress, NoWorkspace, NumberInput, OutputChoice, OutputSelect, SelectDatasetHint, StatTile, WarningTip,
};
use crate::state::{use_datasets_version, use_primary_dataset, use_selected_datasets, use_workspace, use_workspace_checked};
use crate::util::{format_count, plural};

/// Label formats that can be written: plan value, name, and what lands on disk.
const FORMATS: [(&str, &str, &str); 3] = [
    ("yolo", "YOLO", "Each split gets an images and a labels folder; data.yaml at the top is ready for training."),
    ("coco", "COCO JSON", "Each split folder gets its images and one _annotations.coco.json."),
    ("voc", "Pascal VOC", "Each split folder gets its images with one .xml beside each."),
];

#[component]
pub fn Convert() -> Element {
    let workspace = use_workspace();
    let checked = use_workspace_checked();
    let primary = use_primary_dataset();
    let selected = use_selected_datasets();

    rsx! {
        document::Title { "Convert · Modeloid" }
        div { class: "page page-wide",
            p { class: "page-eyebrow", "Dataset Manager" }
            h1 { "Convert" }
            p { class: "page-lead",
                "Write a dataset in another label format, turn outlines into boxes, or cut its objects out into a classification dataset."
            }
            match (workspace(), primary()) {
                (Some(_), Some(_)) if selected.read().len() > 1 => {
                    let ids = selected();
                    let key = ids.iter().map(u32::to_string).collect::<Vec<_>>().join("-");
                    rsx! { BatchConverter { key: "{key}", ids } }
                }
                (Some(_), Some(id)) => rsx! { Converter { key: "{id}", id } },
                (Some(_), None) => rsx! { SelectDatasetHint {} },
                (None, _) if !checked() => rsx! {},
                (None, _) => rsx! { NoWorkspace { text: "Convert works on a workspace's datasets." } },
            }
        }
    }
}

#[component]
fn Converter(id: u32) -> Element {
    let datasets_version = use_datasets_version();
    let workspace = use_workspace();
    // The format starts empty and is set once the dataset's own format is known.
    let mut plan = use_signal(|| ConvertPlan {
        input: id,
        task: TargetTask::Same,
        format: String::new(),
        padding_percent: 0,
        min_size: DEFAULT_MIN_CROP,
    });
    let mut preview = use_signal(|| None::<ConvertPreview>);
    let latest = use_resource(move || async move {
        let _ = datasets_version();
        convert_preview(plan()).await
    });
    use_effect(move || {
        if let Some(Ok(p)) = &*latest.read() {
            if plan.peek().format.is_empty() && !p.source_format.is_empty() {
                // The next preview is the first shown.
                let default = workspace.peek().as_ref().map(|w| w.default_format.clone()).unwrap_or_default();
                plan.write().format = start_format(&default, &p.source_format);
            } else {
                preview.set(Some(p.clone()));
            }
        }
    });
    let load_error = match &*latest.read() {
        Some(Err(e)) => Some(error_message(e)),
        _ => None,
    };

    let datasets = use_resource(move || async move {
        let _ = datasets_version();
        list_datasets().await
    });
    let row = datasets.read().as_ref().and_then(|r| r.as_ref().ok()).and_then(|rows| rows.iter().find(|r| r.id == id).cloned());
    let name = row.as_ref().map_or(format!("Dataset #{id}"), |r| r.name.clone());
    let format = row.map(|r| r.format).unwrap_or_default();

    rsx! {
        div { class: "explore-head",
            div { class: "explore-title",
                h2 { class: "explore-name", "{name}" }
                if !format.is_empty() {
                    span { class: "section-card-meta", "{format} labels" }
                }
            }
        }
        if let Some(message) = &load_error {
            p { class: "form-error", role: "alert", "{message}" }
        }
        match preview() {
            // Not YOLO, COCO or Pascal VOC: nothing to choose.
            Some(p) if p.source_format.is_empty() => rsx! {
                div { class: "notice notice-warning", role: "status",
                    for problem in p.problems.iter() {
                        p { key: "{problem}", "{problem}" }
                    }
                }
            },
            Some(p) => rsx! {
                div { class: "section-stack",
                    TargetCard { plan, preview: p.clone() }
                    ResultCard { plan: plan(), preview: p.clone() }
                    RunCard { plan: plan(), preview: p, name }
                }
            },
            None if load_error.is_some() => rsx! {},
            None => rsx! { p { class: "field-hint", "Reading labels…" } },
        }
    }
}

/// What to convert to: the task, then the label format or the crop options.
#[component]
fn TargetCard(plan: Signal<ConvertPlan>, preview: ConvertPreview) -> Element {
    let mut plan = plan;
    let current = plan();
    let outlines = if preview.outlines > 0 {
        format!("{} outlines become their bounding boxes.", format_count(preview.outlines))
    } else {
        "This dataset has no outlines; its labels are boxes already.".to_string()
    };
    let tasks = [
        (TargetTask::Same, "Same labels, another format", "Boxes stay boxes and outlines stay outlines, where the format has them.".to_string()),
        (TargetTask::Detection, "Detection (boxes only)", outlines),
        (TargetTask::Classification, "Classification (one image per object)", "Every object is cut out into a folder named after its class.".to_string()),
    ];

    rsx! {
        section { class: "section-card", aria_labelledby: "convert-target-title",
            h2 { id: "convert-target-title", class: "section-card-title", "Convert to" }
            div { class: "choice-list",
                for (task, label, detail) in tasks {
                    label { key: "{label}", class: if current.task == task { "choice is-selected" } else { "choice" },
                        input {
                            r#type: "radio",
                            name: "convert-task",
                            checked: current.task == task,
                            onchange: move |_| plan.write().task = task,
                        }
                        span { class: "choice-body",
                            span { class: "choice-label", "{label}" }
                            span { class: "choice-detail", "{detail}" }
                        }
                    }
                }
            }
            if current.task == TargetTask::Classification {
                div { class: "option-row",
                    span { class: "field-label", "Padding" }
                    NumberInput { value: current.padding_percent, label: "Padding in percent", max: MAX_PADDING, onchange: move |n| plan.write().padding_percent = n }
                    span { class: "field-hint", "% of the box's width and height, added on each side for context" }
                }
                div { class: "option-row",
                    span { class: "field-label", "Smallest crop" }
                    NumberInput { value: current.min_size, label: "Smallest crop in pixels", max: 10_000, onchange: move |n| plan.write().min_size = n }
                    span { class: "field-hint", "pixels; objects smaller than this in width or height are skipped" }
                }
            } else {
                div { class: "field field-inline",
                    label { class: "field-label", r#for: "convert-format", "Label format" }
                    select {
                        id: "convert-format",
                        class: "input select",
                        value: "{current.format}",
                        onchange: move |e| plan.write().format = e.value(),
                        for (value, label, _) in FORMATS {
                            option { key: "{value}", value, selected: current.format == value,
                                if preview.source_format == value { "{label} (current)" } else { "{label}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// What the conversion writes.
#[component]
fn ResultCard(plan: ConvertPlan, preview: ConvertPreview) -> Element {
    let crops = plan.task == TargetTask::Classification;
    let layout = if crops {
        "Each split gets one folder per class, holding that class's crops."
    } else {
        FORMATS.iter().find(|f| f.0 == plan.format).map_or("", |f| f.2)
    };
    let kept = preview.outlines - preview.outlines_lost;
    let bars: Vec<Bar> =
        preview.classes.iter().map(|c| Bar { label: c.name.clone(), value: c.labels, detail: Some("crops".to_string()) }).collect();

    rsx! {
        section { class: "section-card", aria_labelledby: "convert-result-title",
            div { class: "section-card-header",
                h2 { id: "convert-result-title", class: "section-card-title", "Result" }
                WarningTip { errors: preview.problems.clone(), warnings: preview.notes.clone() }
            }
            div { class: "stat-tiles",
                if crops {
                    StatTile { label: "Images", value: format_count(preview.labels), detail: "one per object".to_string() }
                    StatTile { label: "Classes", value: format_count(preview.classes.len() as u32), detail: "folders".to_string() }
                } else {
                    StatTile { label: "Images", value: format_count(preview.images) }
                    StatTile { label: "Labels", value: format_count(preview.labels) }
                    StatTile { label: "Outlines kept", value: format_count(kept), detail: format!("{} become boxes", format_count(preview.outlines_lost)) }
                }
            }
            if crops && !bars.is_empty() {
                BarList { bars, label: "Crops per class" }
                p { class: "field-hint", "Counts are before the size limit; the run reports how many were too small." }
            }
            p { class: "field-hint", "{layout}" }
        }
    }
}

#[component]
fn RunCard(plan: ConvertPlan, preview: ConvertPreview, name: String) -> Element {
    let output = use_signal(OutputChoice::default);
    let mut submitted = use_signal(|| false);
    let mut form_error = use_signal(|| None::<String>);
    let (job, _) = use_job(JobKind::Convert);
    let running = job.read().as_ref().is_some_and(|j| !j.finished);

    let suggested = format!("{name}{}", result_suffix(&plan, &preview.source_format));
    let can_run = preview.can_run() && !running;

    let suggested_for_run = suggested.clone();
    let submit = move |e: FormEvent| {
        let plan = plan.clone();
        let suggested = suggested_for_run.clone();
        async move {
            e.prevent_default();
            submitted.set(true);
            form_error.set(None);
            let Ok(target) = output.read().resolve(&suggested, true) else { return };
            match start_convert(plan, target).await {
                Ok(started) => {
                    set_if_newer(job, started);
                    submitted.set(false);
                }
                Err(e) => form_error.set(Some(error_message(&e))),
            }
        }
    };

    rsx! {
        form { class: "section-stack", novalidate: true, onsubmit: submit,
            OutputSelect { choice: output, suggested_name: suggested, source_name: Some(name), show_errors: submitted(), disabled: running }
            if let Some(message) = form_error() {
                p { class: "form-error", role: "alert", "{message}" }
            }
            div { class: "form-actions",
                button { class: "button", r#type: "submit", disabled: !can_run,
                    if running { "Working…" } else { "Convert" }
                }
                if !preview.problems.is_empty() {
                    span { class: "form-actions-note", "Resolve the problems in Result first." }
                } else if let Some(reason) = &preview.unchanged {
                    span { class: "form-actions-note", "{reason}" }
                }
            }
        }
        if let Some(current) = job() {
            JobProgress { job: current, title: "Converting", noun: "dataset", verb: "converted" }
        }
    }
}

/// Format a conversion starts on: the workspace's default, or, when the
/// dataset is in it already or there is none, another one.
fn start_format(default: &str, source: &str) -> String {
    let other = if source == "yolo" { "coco" } else { "yolo" };
    if default.is_empty() || default == source { other.to_string() } else { default.to_string() }
}

/// Name suffix of a conversion's results, e.g. "-yolo".
fn result_suffix(plan: &ConvertPlan, source_format: &str) -> String {
    match plan.task {
        TargetTask::Classification => "-cls".into(),
        _ if plan.format != source_format => format!("-{}", plan.format),
        _ => "-det".into(),
    }
}

/// Several selected datasets, converted with one plan, each to its own result.
#[component]
fn BatchConverter(ids: Vec<u32>) -> Element {
    let datasets_version = use_datasets_version();
    let workspace = use_workspace();
    let default = workspace.peek().as_ref().map(|w| w.default_format.clone()).unwrap_or_default();
    let plan = use_signal(|| ConvertPlan {
        input: ids[0],
        task: TargetTask::Same,
        format: if default.is_empty() { "yolo".to_string() } else { default },
        padding_percent: 0,
        min_size: DEFAULT_MIN_CROP,
    });
    // The last previews stay shown while the next load.
    let mut previews = use_signal(|| None::<Vec<ConvertPreview>>);
    let ids_for_preview = ids.clone();
    let latest = use_resource(move || {
        let ids = ids_for_preview.clone();
        async move {
            let _ = datasets_version();
            convert_batch_preview(plan(), ids).await
        }
    });
    use_effect(move || {
        if let Some(Ok(p)) = &*latest.read() {
            previews.set(Some(p.clone()));
        }
    });
    let load_error = match &*latest.read() {
        Some(Err(e)) => Some(error_message(e)),
        _ => None,
    };

    let datasets = use_resource(move || async move {
        let _ = datasets_version();
        list_datasets().await
    });
    let all_rows: Vec<DatasetRow> = datasets.read().as_ref().and_then(|r| r.as_ref().ok()).cloned().unwrap_or_default();
    let names: Vec<String> = ids.iter().map(|id| all_rows.iter().find(|r| r.id == *id).map_or(format!("Dataset #{id}"), |r| r.name.clone())).collect();
    let taken: Vec<String> = all_rows.iter().map(|r| r.name.clone()).collect();

    rsx! {
        div { class: "explore-head",
            div { class: "explore-title",
                h2 { class: "explore-name", "{ids.len()} datasets" }
                span { class: "section-card-meta", "each converted to its own result" }
            }
        }
        if let Some(message) = &load_error {
            p { class: "form-error", role: "alert", "{message}" }
        }
        match previews() {
            Some(list) => {
                // What the task and format choices need to know about the inputs together.
                let first = list.first().map(|p| p.source_format.clone()).unwrap_or_default();
                let common = if list.iter().all(|p| p.source_format == first) { first } else { String::new() };
                let combined = ConvertPreview { source_format: common, outlines: list.iter().map(|p| p.outlines).sum(), ..Default::default() };
                rsx! {
                    div { class: "section-stack",
                        TargetCard { plan, preview: combined }
                        BatchResultCard { names: names.clone(), previews: list.clone() }
                        BatchRunCard { plan: plan(), ids: ids.clone(), names, previews: list, taken }
                    }
                }
            }
            None if load_error.is_some() => rsx! {},
            None => rsx! { p { class: "field-hint", "Reading labels…" } },
        }
    }
}

/// What happens to each dataset of a batch.
#[component]
fn BatchResultCard(names: Vec<String>, previews: Vec<ConvertPreview>) -> Element {
    let converted = previews.iter().filter(|p| p.can_run()).count();
    let skipped = previews.iter().filter(|p| p.problems.is_empty() && p.unchanged.is_some()).count();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for (name, preview) in names.iter().zip(&previews) {
        errors.extend(preview.problems.iter().map(|p| format!("{name}: {p}")));
        warnings.extend(preview.notes.iter().map(|n| format!("{name}: {n}")));
    }

    rsx! {
        section { class: "section-card", aria_labelledby: "convert-batch-title",
            div { class: "section-card-header",
                h2 { id: "convert-batch-title", class: "section-card-title", "Result" }
                span { class: "section-card-aside",
                    span { class: "section-card-meta",
                        "{plural(converted, \"dataset\")} to convert"
                        if skipped > 0 { " · {skipped} skipped" }
                    }
                    WarningTip { errors, warnings }
                }
            }
            div { class: "table-scroll",
                table { class: "data-table result-table",
                    thead {
                        tr {
                            th { scope: "col", "Dataset" }
                            th { scope: "col", "From" }
                            th { class: "num", scope: "col", "Images" }
                            th { class: "num", scope: "col", "Labels" }
                            th { class: "num", scope: "col", "Outlines to boxes" }
                            th { scope: "col", "Result" }
                        }
                    }
                    tbody {
                        for (name, preview) in names.iter().zip(&previews) {
                            tr { key: "{name}",
                                th { class: "name", scope: "row", title: "{name}", "{name}" }
                                td { class: "muted", if preview.source_format.is_empty() { "–" } else { "{preview.source_format}" } }
                                td { class: "num", "{format_count(preview.images)}" }
                                td { class: "num", "{format_count(preview.labels)}" }
                                td { class: "num muted", "{format_count(preview.outlines_lost)}" }
                                match (preview.problems.first(), &preview.unchanged) {
                                    (Some(problem), _) => rsx! { td { class: "field-error", title: "{problem}", "Problem" } },
                                    (None, Some(reason)) => rsx! { td { class: "muted", title: "{reason}", "Skipped, nothing changes" } },
                                    (None, None) => rsx! { td { "Converted" } },
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Where a batch's results go, and the button that starts it.
#[component]
fn BatchRunCard(plan: ConvertPlan, ids: Vec<u32>, names: Vec<String>, previews: Vec<ConvertPreview>, taken: Vec<String>) -> Element {
    let mut overwrite = use_signal(|| false);
    let mut confirmed = use_signal(|| false);
    let mut suffix = use_signal(|| None::<String>);
    let mut form_error = use_signal(|| None::<String>);
    let (job, _) = use_job(JobKind::Convert);
    let running = job.read().as_ref().is_some_and(|j| !j.finished);

    let common_source = previews.first().map(|p| p.source_format.clone()).unwrap_or_default();
    let suggested = result_suffix(&plan, &common_source);
    let suffix_now = suffix().unwrap_or(suggested);
    let converted = previews.iter().filter(|p| p.can_run()).count();
    // New names of the datasets that would be written.
    let name_error = if overwrite() {
        None
    } else {
        names.iter().zip(&previews).filter(|(_, p)| p.can_run()).find_map(|(name, _)| {
            let new = format!("{name}{suffix_now}");
            match validate_dataset_name(&new) {
                Err(e) => Some(format!("{new}: {e}")),
                Ok(()) if taken.contains(&new) => Some(format!("A dataset named {new} exists already.")),
                Ok(()) => None,
            }
        })
    };
    let example = names.iter().zip(&previews).find(|(_, p)| p.can_run()).map(|(n, _)| format!("{n}{suffix_now}"));
    let can_run = converted > 0 && !running && name_error.is_none() && (!overwrite() || confirmed());

    let suffix_for_run = suffix_now.clone();
    let submit = move |e: FormEvent| {
        let (plan, ids, suffix) = (plan.clone(), ids.clone(), suffix_for_run.clone());
        async move {
            e.prevent_default();
            form_error.set(None);
            let output = if overwrite() { BatchOutput::Overwrite } else { BatchOutput::New { suffix } };
            match start_convert_batch(plan, ids, output).await {
                Ok(started) => {
                    set_if_newer(job, started);
                    confirmed.set(false);
                }
                Err(e) => form_error.set(Some(error_message(&e))),
            }
        }
    };

    rsx! {
        form { class: "section-stack", novalidate: true, onsubmit: submit,
            fieldset { class: "section-card", disabled: running,
                legend { class: "section-card-title", "Output" }
                div { class: "choice-list",
                    label { class: if !overwrite() { "choice is-selected" } else { "choice" },
                        input { r#type: "radio", name: "convert-batch-output", checked: !overwrite(), onchange: move |_| overwrite.set(false) }
                        span { class: "choice-body",
                            span { class: "choice-label", "Save as new datasets" }
                            span { class: "choice-detail", "The sources stay as they are." }
                        }
                    }
                    label { class: if overwrite() { "choice is-selected" } else { "choice" },
                        input { r#type: "radio", name: "convert-batch-output", checked: overwrite(), onchange: move |_| overwrite.set(true) }
                        span { class: "choice-body",
                            span { class: "choice-label", "Overwrite sources" }
                            span { class: "choice-detail", "Each dataset is replaced by its converted version." }
                        }
                    }
                }
                if overwrite() {
                    label { class: "check",
                        input { r#type: "checkbox", checked: confirmed(), onchange: move |e| confirmed.set(e.checked()) }
                        span { "I understand the {plural(converted, \"source dataset\")} will be replaced. This can't be undone." }
                    }
                } else {
                    div { class: "field",
                        label { class: "field-label", r#for: "convert-batch-suffix", "Name suffix" }
                        input {
                            id: "convert-batch-suffix",
                            class: "input input-suffix",
                            r#type: "text",
                            spellcheck: false,
                            autocomplete: "off",
                            value: "{suffix_now}",
                            aria_invalid: if name_error.is_some() { "true" },
                            aria_describedby: "convert-batch-suffix-hint",
                            oninput: move |e| suffix.set(Some(e.value())),
                        }
                        p { id: "convert-batch-suffix-hint", class: "field-hint",
                            "Added to each dataset's name"
                            if let Some(example) = &example { ", e.g. {example}" }
                            "."
                        }
                    }
                    if let Some(message) = &name_error {
                        p { class: "field-error", role: "alert", "{message}" }
                    }
                }
            }
            if let Some(message) = form_error() {
                p { class: "form-error", role: "alert", "{message}" }
            }
            div { class: "form-actions",
                button { class: "button", r#type: "submit", disabled: !can_run,
                    if running { "Working…" } else { "Convert {plural(converted, \"dataset\")}" }
                }
                if converted == 0 {
                    span { class: "form-actions-note", "Nothing to convert with these settings." }
                }
            }
        }
        if let Some(current) = job() {
            JobProgress { job: current, title: "Converting", noun: "dataset", verb: "converted" }
        }
    }
}
