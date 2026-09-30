//! Dataset Manager > Convert: write the selected dataset in another label
//! format (YOLO, COCO, Pascal VOC), with boxes only, or as classification
//! crops (one image per object, in a folder per class).

use dioxus::prelude::*;

use crate::api::convert::{convert_preview, start_convert, ConvertPlan, ConvertPreview, TargetTask, DEFAULT_MIN_CROP, MAX_PADDING};
use crate::api::datasets::list_datasets;
use crate::api::error_message;
use crate::api::jobs::JobKind;
use crate::components::{NoWorkspace, 
    set_if_newer, use_job, Bar, BarList, JobProgress, NumberInput, OutputChoice, OutputSelect, PrimaryNote, SelectDatasetHint, StatTile,
};
use crate::state::{use_datasets_version, use_primary_dataset, use_selected_datasets, use_workspace, use_workspace_checked};
use crate::util::format_count;

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
                (Some(_), Some(id)) => rsx! { Converter { key: "{id}", id, selected_count: selected.read().len() } },
                (Some(_), None) => rsx! { SelectDatasetHint {} },
                (None, _) if !checked() => rsx! {},
                (None, _) => rsx! { NoWorkspace { text: "Convert works on a workspace's datasets." } },
            }
        }
    }
}

#[component]
fn Converter(id: u32, selected_count: usize) -> Element {
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
                // Start on the workspace's default format, or, when the dataset is in it
                // already or there is none, on another one. The next preview is the first shown.
                let default = workspace.peek().as_ref().map(|w| w.default_format.clone()).unwrap_or_default();
                let other = if p.source_format == "yolo" { "coco" } else { "yolo" };
                plan.write().format = if default.is_empty() || default == p.source_format { other.to_string() } else { default };
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
        PrimaryNote { count: selected_count }
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
            h2 { id: "convert-result-title", class: "section-card-title", "Result" }
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
            if !preview.problems.is_empty() || !preview.notes.is_empty() {
                ul { class: "warning-list",
                    for problem in preview.problems.iter() {
                        li { key: "{problem}", class: "is-error", "{problem}" }
                    }
                    for note in preview.notes.iter() {
                        li { key: "{note}", "{note}" }
                    }
                }
            }
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

    let suffix = match plan.task {
        TargetTask::Classification => "cls",
        _ if plan.format != preview.source_format => plan.format.as_str(),
        _ => "det",
    };
    let suggested = format!("{name}-{suffix}");
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
                if !preview.can_run() {
                    span { class: "form-actions-note", "Resolve the problems in Result first." }
                }
            }
        }
        if let Some(current) = job() {
            JobProgress { job: current, title: "Converting", noun: "dataset", verb: "converted" }
        }
    }
}
