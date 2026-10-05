//! Dataset Manager > Augmentation: augmented copies of the images of the dataset
//! selected in the panel, made with Albumentations. Choose which images get
//! copies (splits, classes, share, copies each), the augmentations in a
//! table (on/off, chance, settings), see the resulting counts, and save a
//! new dataset or overwrite the source.

use dioxus::prelude::*;

use crate::api::datasets::list_datasets;
use crate::components::TitleIcon;
use crate::api::error_message;
use crate::api::jobs::JobKind;
use crate::api::pyenv::python_env_overview;
use crate::api::transform::{
    default_augmentations, start_transform, transform_preview, AugParam, AugValue, Augmentation, ParamKind, TransformPlan,
    TransformPreview, ALBUMENTATIONS_MISSING, AUGMENTATIONS, MAX_COPIES,
};
use crate::components::table::{Column, Sort, SortHeader};
use crate::components::{
    open_python_settings, set_if_newer, use_job, Icon, IconName, InfoTip, JobProgress, NoWorkspace, OutputChoice, OutputSelect, PrimaryNote,
    SelectDatasetHint, Slider, WarningTip,
};
use crate::state::{use_datasets_version, use_primary_dataset, use_selected_datasets, use_workspace, use_workspace_checked};
use crate::util::{format_count, plural};

const SPLIT_NAMES: [&str; 4] = ["Train", "Val", "Test", "No split"];

#[component]
pub fn Transform() -> Element {
    let workspace = use_workspace();
    let checked = use_workspace_checked();
    let primary = use_primary_dataset();
    let selected = use_selected_datasets();

    rsx! {
        document::Title { "Augmentation · Modeloid" }
        div { class: "page page-wide",
            p { class: "page-eyebrow", "Dataset Manager" }
            h1 { "Augmentation" }
            p { class: "page-lead",
                "Add augmented copies of a dataset's images: flips, crops, color changes, blur, weather and more. Boxes and outlines move with the pixels."
            }
            match (workspace(), primary()) {
                (Some(_), Some(id)) => rsx! { TransformForm { key: "{id}", id, selected_count: selected.read().len() } },
                (Some(_), None) => rsx! { SelectDatasetHint {} },
                (None, _) if !checked() => rsx! {},
                (None, _) => rsx! { NoWorkspace { text: "Augmentation works on a workspace's datasets." } },
            }
        }
    }
}

#[component]
fn TransformForm(id: u32, selected_count: usize) -> Element {
    let datasets_version = use_datasets_version();
    let plan = use_signal(|| TransformPlan::new(id));
    let datasets = use_resource(move || async move {
        let _ = datasets_version();
        list_datasets().await
    });
    let name = datasets
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .and_then(|rows| rows.iter().find(|r| r.id == id).map(|r| r.name.clone()))
        .unwrap_or_else(|| format!("Dataset #{id}"));

    // The last good preview stays while the next loads.
    let mut preview = use_signal(|| None::<TransformPreview>);
    let latest = use_resource(move || async move {
        let _ = datasets_version();
        transform_preview(plan()).await
    });
    use_effect(move || {
        if let Some(Ok(p)) = &*latest.read() {
            preview.set(Some(p.clone()));
        }
    });
    let error = match &*latest.read() {
        Some(Err(e)) => Some(error_message(e)),
        _ => None,
    };

    rsx! {
        PrimaryNote { count: selected_count }
        div { class: "explore-head",
            div { class: "explore-title",
                h2 { class: "explore-name", "{name}" }
            }
        }
        if let Some(message) = error.clone() {
            p { class: "form-error", role: "alert", "{message}" }
        }
        match preview() {
            Some(p) => rsx! {
                div { class: "section-stack",
                    ImagesCard { plan, preview: p.clone() }
                    AugmentationTable { plan }
                    ResultCard { preview: p.clone(), copies: plan.read().copies }
                    RunCard { plan: plan(), can_run: p.can_run(), name }
                }
            },
            None if error.is_some() => rsx! {},
            None => rsx! { p { class: "field-hint", "Reading labels…" } },
        }
    }
}

/// Which images get copies: splits, classes, share, copies each, originals.
#[component]
fn ImagesCard(plan: Signal<TransformPlan>, preview: TransformPreview) -> Element {
    let mut plan = plan;
    let p = plan();
    let typed_seed = p.seed.map(|s| s.to_string()).unwrap_or_default();

    rsx! {
        section { class: "section-card", aria_labelledby: "transform-images-title",
            h2 { id: "transform-images-title", class: "section-card-title", TitleIcon { name: IconName::Image } "Images" }
            div { class: "field",
                span { class: "field-label",
                    "Splits "
                    InfoTip { label: "About splits", text: "Images in these splits get copies. Usually train only: val and test should show real images." }
                }
                div { class: "option-row",
                    for (slot, split) in SPLIT_NAMES.iter().enumerate().filter(|(slot, _)| preview.images_before[*slot] > 0 || p.splits[*slot]) {
                        label { key: "{slot}", class: "check",
                            input {
                                r#type: "checkbox",
                                checked: p.splits[slot],
                                onchange: move |e| plan.write().splits[slot] = e.checked(),
                            }
                            span { "{split} ({format_count(preview.images_before[slot])})" }
                        }
                    }
                }
            }
            if !preview.classes.is_empty() {
                div { class: "field",
                    span { class: "field-label",
                        "Classes "
                        InfoTip {
                            label: "About classes",
                            text: "Only images with at least one of the chosen classes get copies, e.g. to add examples of a rare class. All are highlighted when none is picked out: every image can get copies. Click a class to keep only it, then add more.",
                        }
                    }
                    div { class: "chip-list", role: "group", aria_label: "Classes",
                        for class in preview.classes.iter() {
                            {
                                let class = class.clone();
                                let on = p.classes.contains(&class);
                                // None chosen means every class: all show as chosen. Clicking one then keeps only it.
                                let included = on || p.classes.is_empty();
                                rsx! {
                                    button {
                                        key: "{class}",
                                        class: "chip",
                                        r#type: "button",
                                        aria_pressed: if included { "true" } else { "false" },
                                        onclick: {
                                            let class = class.clone();
                                            move |_| {
                                                let classes = &mut plan.write().classes;
                                                if on {
                                                    classes.retain(|c| *c != class);
                                                } else {
                                                    classes.push(class.clone());
                                                }
                                            }
                                        },
                                        "{class}"
                                    }
                                }
                            }
                        }
                        if !p.classes.is_empty() {
                            button { class: "button button-ghost button-sm", r#type: "button", onclick: move |_| plan.write().classes.clear(), "All classes" }
                        }
                    }
                }
            }
            div { class: "param-grid transform-grid",
                div { class: "field param-field",
                    span { class: "field-label",
                        "Share of images "
                        InfoTip { label: "About the share", text: "A random share of the matching images gets copies. The seed decides which." }
                    }
                    Slider { value: p.percent, label: "Share of images in percent", min: 1, max: 100, unit: "%", onchange: move |n| plan.write().percent = n }
                }
                div { class: "field param-field",
                    span { class: "field-label",
                        "Copies per image "
                        InfoTip { label: "About copies", text: "Each chosen image gets this many augmented copies, each one drawn anew." }
                    }
                    Slider { value: p.copies, label: "Copies per image", min: 1, max: MAX_COPIES, onchange: move |n| plan.write().copies = n }
                }
            }
            div { class: "option-row",
                label { class: "check",
                    input {
                        r#type: "checkbox",
                        checked: p.keep_originals,
                        onchange: move |e| plan.write().keep_originals = e.checked(),
                    }
                    span { "Keep the original images" }
                }
                InfoTip {
                    label: "About keeping originals",
                    text: "Off: the chosen images are replaced by their copies. With one copy and 100% chance, e.g. Resize or Grayscale, that changes every image instead of adding new ones.",
                }
            }
            div { class: "option-row",
                label { class: "field field-inline",
                    span { class: "field-label",
                        "Seed "
                        InfoTip { label: "About the seed", text: "Using seed {preview.seed}; empty uses the workspace seed. The same seed chooses the same images and draws the same augmentations again." }
                    }
                    input {
                        class: "input input-narrow",
                        r#type: "number",
                        min: "0",
                        placeholder: "{preview.seed}",
                        value: "{typed_seed}",
                        oninput: move |e| {
                            let text = e.value();
                            match text.trim().parse() {
                                Ok(n) => plan.write().seed = Some(n),
                                Err(_) if text.trim().is_empty() => plan.write().seed = None,
                                Err(_) => {}
                            }
                        },
                    }
                }
            }
        }
    }
}

const COLUMNS: [Column; 4] = [Column::id(), Column::text("Augmentation"), Column::text("Group"), Column::text("Chance")];

/// Every augmentation: used or not (the checkbox), its chance and its
/// settings. Rows are applied in the # order.
#[component]
fn AugmentationTable(plan: Signal<TransformPlan>) -> Element {
    let mut plan = plan;
    let sort = use_signal(Sort::by_id);
    // (row number, position in `plan.augmentations`, the augmentation).
    let mut rows: Vec<(usize, usize, &'static Augmentation)> = AUGMENTATIONS
        .iter()
        .enumerate()
        .filter_map(|(n, a)| plan.read().augmentations.iter().position(|r| r.key == a.key).map(|i| (n + 1, i, a)))
        .collect();
    let s = sort();
    rows.sort_by(|a, b| {
        let order = match s.column {
            1 => a.2.label.cmp(b.2.label),
            2 => a.2.group.cmp(b.2.group),
            3 => {
                let p = plan.read();
                p.augmentations[a.1].p.total_cmp(&p.augmentations[b.1].p)
            }
            _ => a.0.cmp(&b.0),
        };
        s.directed(order).then(a.0.cmp(&b.0))
    });
    let total = rows.len();
    let used = plan.read().augmentations.iter().filter(|r| r.enabled).count();

    rsx! {
        details { class: "section-card param-group tune-space", open: true,
            summary { class: "param-group-summary",
                h2 { class: "section-card-title",
                    TitleIcon { name: IconName::Wand }
                    "Augmentations "
                    InfoTip {
                        label: "About the augmentations",
                        text: "Each copy goes through the checked rows from top to bottom (# order); each applies with its chance. Settings with two values draw a random value between them every time.",
                    }
                }
                span { class: "section-card-meta", "{used} of {plural(total, \"augmentation\")} used" }
            }
            div { class: "panel-selection tune-space-bar", role: "group", aria_label: "Augmentation selection",
                button { class: "button button-ghost button-sm", r#type: "button",
                    onclick: move |_| plan.write().augmentations.iter_mut().for_each(|r| r.enabled = true),
                    "Select all"
                }
                button { class: "button button-ghost button-sm", r#type: "button",
                    onclick: move |_| plan.write().augmentations.iter_mut().for_each(|r| r.enabled = false),
                    "None"
                }
                span { class: "section-card-meta", "{used} selected" }
                button {
                    class: "icon-button icon-button-sm",
                    r#type: "button",
                    aria_label: "Reset the augmentations",
                    title: "Reset: the default augmentations and settings",
                    onclick: move |_| plan.write().augmentations = default_augmentations(),
                    Icon { name: IconName::Refresh, size: 14 }
                }
            }
            div { class: "table-scroll tune-space-scroll",
                table { class: "data-table tune-space-table transform-table",
                    thead {
                        tr {
                            th { class: "check-col", scope: "col", span { class: "visually-hidden", "Used" } }
                            for (i, def) in COLUMNS.iter().enumerate() {
                                SortHeader { key: "{i}", column: i, def: *def, sort }
                            }
                        }
                    }
                    // One body per augmentation: its row and its settings line read as one band.
                    for (number, i, a) in rows.iter().copied() {
                        {
                            let row = plan.read().augmentations[i].clone();
                            let tip = if a.spatial {
                                format!("{} Boxes and outlines move with it. Albumentations: {}.", a.tip, a.key)
                            } else {
                                format!("{} Labels stay as they are. Albumentations: {}.", a.tip, a.key)
                            };
                            let percent = (row.p * 100.0).round() as u32;
                            let problems = row.problems();
                            let has_settings = !a.params.is_empty() || !problems.is_empty();
                            rsx! {
                                tbody { key: "{row.key}", class: if row.enabled { "transform-aug is-on" } else { "transform-aug" },
                                    tr { class: if row.enabled { "is-checked" } else { "" },
                                        td { class: "check-col",
                                            input {
                                                r#type: "checkbox",
                                                aria_label: "Use {a.label}",
                                                checked: row.enabled,
                                                onchange: move |e| plan.write().augmentations[i].enabled = e.checked(),
                                            }
                                        }
                                        td { class: "num muted", "{number}" }
                                        th { class: "name", scope: "row",
                                            span { class: "tune-param-name",
                                                "{a.label} "
                                                InfoTip { label: "About {a.label}", text: "{tip}" }
                                            }
                                        }
                                        td { class: "muted", "{a.group}" }
                                        td {
                                            Slider {
                                                value: percent,
                                                label: "Chance of {a.label} in percent",
                                                unit: "%",
                                                disabled: !row.enabled,
                                                onchange: move |n: u32| plan.write().augmentations[i].p = f64::from(n) / 100.0,
                                            }
                                        }
                                    }
                                    // Settings on a line of their own under the row, so the table stays narrow.
                                    if has_settings {
                                        tr { class: if row.enabled { "transform-settings-row is-checked" } else { "transform-settings-row" },
                                            td { colspan: "2" }
                                            td { colspan: "3",
                                                div { class: "transform-settings", role: "group", aria_label: "{a.label} settings",
                                                    for param in a.params.iter() {
                                                        ParamEditor {
                                                            key: "{param.key}",
                                                            param: *param,
                                                            value: row.value(param),
                                                            disabled: !row.enabled,
                                                            on_change: move |(key, value): (String, AugValue)| {
                                                                plan.write().augmentations[i].params.insert(key, value);
                                                            },
                                                        }
                                                    }
                                                }
                                                for problem in problems.iter() {
                                                    span { key: "{problem}", class: "field-error tune-row-problem", "{problem}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One setting: two numbers (a random value between them), a number, or a choice.
#[component]
fn ParamEditor(param: AugParam, value: AugValue, disabled: bool, on_change: EventHandler<(String, AugValue)>) -> Element {
    let key = param.key.to_string();
    let step = if param.integer { "1" } else { "any" };
    let body = match (param.kind, value) {
        (ParamKind::Pair, AugValue::Pair([low, high])) => {
            let key2 = key.clone();
            rsx! {
                span { class: "tune-range",
                    input {
                        class: "input input-narrow",
                        r#type: "number",
                        step,
                        min: "{param.min}",
                        max: "{param.max}",
                        aria_label: "{param.label}, lowest",
                        disabled,
                        value: "{low}",
                        onchange: move |e| {
                            if let Ok(v) = e.value().trim().parse::<f64>() {
                                on_change.call((key.clone(), AugValue::Pair([v, high])));
                            }
                        },
                    }
                    span { class: "muted", "to" }
                    input {
                        class: "input input-narrow",
                        r#type: "number",
                        step,
                        min: "{param.min}",
                        max: "{param.max}",
                        aria_label: "{param.label}, highest",
                        disabled,
                        value: "{high}",
                        onchange: move |e| {
                            if let Ok(v) = e.value().trim().parse::<f64>() {
                                on_change.call((key2.clone(), AugValue::Pair([low, v])));
                            }
                        },
                    }
                }
            }
        }
        (ParamKind::Number, AugValue::Number(n)) => rsx! {
            input {
                class: "input input-narrow",
                r#type: "number",
                step,
                min: "{param.min}",
                max: "{param.max}",
                aria_label: "{param.label}",
                disabled,
                value: "{n}",
                onchange: move |e| {
                    if let Ok(v) = e.value().trim().parse::<f64>() {
                        on_change.call((key.clone(), AugValue::Number(v)));
                    }
                },
            }
        },
        (ParamKind::Choice(options), AugValue::Text(current)) => rsx! {
            select {
                class: "input select input-narrow",
                aria_label: "{param.label}",
                disabled,
                value: "{current}",
                onchange: move |e| on_change.call((key.clone(), AugValue::Text(e.value()))),
                for option in options.iter() {
                    option { key: "{option}", value: "{option}", selected: *option == current, "{option}" }
                }
            }
        },
        _ => rsx! {},
    };
    rsx! {
        span { class: "transform-param",
            span { class: "transform-param-label", "{param.label}" }
            {body}
        }
    }
}

/// Images per split before and after, and what is added.
#[component]
fn ResultCard(preview: TransformPreview, copies: u32) -> Element {
    let p = &preview;
    let slots: Vec<usize> = (0..4).filter(|&s| p.images_before[s] > 0 || p.images_after[s] > 0).collect();
    let mut notes = Vec::new();
    if p.can_run() {
        notes.push(format!(
            "{} of {} matching images get {} each: {} new images.",
            format_count(p.chosen),
            format_count(p.matching),
            if copies == 1 { "1 copy".to_string() } else { format!("{copies} copies") },
            format_count(p.added),
        ));
    }
    if p.has_polygons {
        notes.push("Outlines are transformed with the image; their boxes are fitted to them again.".to_string());
    }

    rsx! {
        section { class: "section-card", aria_labelledby: "transform-result-title",
            div { class: "section-card-header",
                h2 { id: "transform-result-title", class: "section-card-title", TitleIcon { name: IconName::Chart } "Result" }
                span { class: "section-card-aside",
                    span { class: "section-card-meta", "{format_count(p.total_before())} → {format_count(p.total_after())} images" }
                    WarningTip { errors: p.problems.clone() }
                }
            }
            div { class: "table-scroll",
                table { class: "data-table result-table",
                    thead {
                        tr {
                            th { scope: "col", "Images" }
                            th { class: "num", scope: "col", "Before" }
                            th { class: "num", scope: "col", "After" }
                        }
                    }
                    tbody {
                        tr { class: "total-row",
                            th { class: "name", scope: "row", "All" }
                            td { class: "num muted", "{format_count(p.total_before())}" }
                            td { class: "num", "{format_count(p.total_after())}" }
                        }
                        for slot in slots {
                            tr { key: "{slot}",
                                th { class: "name", scope: "row", "{SPLIT_NAMES[slot]}" }
                                td { class: "num muted", "{format_count(p.images_before[slot])}" }
                                td { class: "num", "{format_count(p.images_after[slot])}" }
                            }
                        }
                    }
                }
            }
            for note in notes.iter() {
                p { key: "{note}", class: "field-hint", "{note}" }
            }
        }
    }
}

/// Output choice and the button that starts the job.
#[component]
fn RunCard(plan: TransformPlan, can_run: bool, name: String) -> Element {
    let output = use_signal(OutputChoice::default);
    let mut submitted = use_signal(|| false);
    let mut form_error = use_signal(|| None::<String>);
    let (job, _) = use_job(JobKind::Transform);
    let running = job.read().as_ref().is_some_and(|j| !j.finished);
    let finished_version = use_memo(move || job.read().as_ref().filter(|j| j.finished).map(|j| j.version));
    let python = use_resource(move || async move {
        let _ = finished_version();
        python_env_overview().await.ok().map(|ov| ov.status.info().is_some_and(|i| !i.albumentations.is_empty()))
    });
    let python_missing = matches!(&*python.read(), Some(Some(false)) | Some(None));
    let suggested = format!("{name}-aug");

    let suggested_for_run = suggested.clone();
    let submit = move |e: FormEvent| {
        let plan = plan.clone();
        let suggested = suggested_for_run.clone();
        async move {
            e.prevent_default();
            submitted.set(true);
            form_error.set(None);
            let Ok(target) = output.read().resolve(&suggested, true) else { return };
            match start_transform(plan, target).await {
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
            OutputSelect {
                choice: output,
                suggested_name: suggested,
                source_name: Some(name),
                show_errors: submitted(),
                disabled: running,
                id: "transform-output",
            }
            if python_missing {
                div { class: "notice notice-warning python-notice", role: "status",
                    span { "{ALBUMENTATIONS_MISSING}" }
                    button { class: "button button-sm", r#type: "button", onclick: move |_| open_python_settings(), "Open Python settings" }
                }
            }
            if let Some(message) = form_error() {
                p { class: "form-error", role: "alert", "{message}" }
            }
            div { class: "form-actions",
                button { class: "button", r#type: "submit", disabled: !can_run || running || python_missing,
                    if running { "Working…" } else { "Augment" }
                }
                if !can_run {
                    span { class: "form-actions-note", "Resolve the problems in Result first." }
                }
            }
        }
        if let Some(current) = job() {
            JobProgress { job: current, title: "Augmenting", noun: "dataset", verb: "written" }
        }
    }
}
