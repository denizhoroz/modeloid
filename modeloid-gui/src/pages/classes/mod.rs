//! Dataset Manager > Classes & Merge: map classes onto an ordered target list.
//! One dataset: rename, merge, drop and reorder its classes. Several: merge them.

mod edit;

use dioxus::prelude::*;
use modeloid_types::classes::normalize_class;

use crate::api::classes::{
    class_lists, class_merge_preview, class_merge_suggestion, delete_class_list, save_class_list, start_class_merge,
    ClassList, MergePlan, MergePreview, SplitMode,
};
use crate::api::datasets::{list_datasets, DatasetRow};
use crate::api::error_message;
use crate::api::jobs::JobKind;
use crate::components::{InfoTip, NoWorkspace, set_if_newer, use_job, Bar, BarList, JobProgress, OutputChoice, OutputSelect, SelectDatasetHint, WarningTip};
use crate::state::{use_batch_selected, use_datasets_version, use_selected_datasets, use_workspace, use_workspace_checked};
use crate::util::{format_count, plural};

use edit::Choice;

#[component]
pub fn ClassesMerge() -> Element {
    let workspace = use_workspace();
    let checked = use_workspace_checked();

    rsx! {
        document::Title { "Classes & Merge · Modeloid" }
        div { class: "page page-wide",
            p { class: "page-eyebrow", "Dataset Manager" }
            h1 { "Classes & Merge" }
            p { class: "page-lead",
                "Rename, merge, drop and reorder classes. Select several datasets in the panel to merge them; their classes are matched by name."
            }
            match workspace() {
                Some(_) => rsx! { Editor {} },
                None if !checked() => rsx! {},
                None => rsx! { NoWorkspace { text: "Classes & Merge works on a workspace's datasets." } },
            }
        }
    }
}

#[component]
fn Editor() -> Element {
    // Inputs are the datasets selected in the panel, in selection order.
    let inputs = use_selected_datasets();
    let batch = use_batch_selected();
    let datasets_version = use_datasets_version();
    let mut plan = use_signal(|| None::<MergePlan>);
    let mut preview = use_signal(|| None::<MergePreview>);
    let mut load_error = use_signal(|| None::<String>);

    // New inputs: fresh suggestion. Datasets added or removed with the
    // checkboxes keep earlier edits that still apply; a row click starts over.
    // The same inputs changing on disk (e.g. this tool overwrote them, so a
    // renamed class has its new name now) re-read their classes and keep the
    // edits that still apply.
    let mut last_inputs = use_signal(Vec::<u32>::new);
    use_effect(move || {
        let ids = inputs();
        let _ = datasets_version();
        let same_inputs = *last_inputs.peek() == ids;
        last_inputs.set(ids.clone());
        let keep_edits = same_inputs || *batch.peek();
        spawn(async move {
            if ids.is_empty() {
                plan.set(None);
                preview.set(None);
                return;
            }
            match class_merge_suggestion(ids).await {
                Ok(suggested) => {
                    let merged = match plan.peek().as_ref() {
                        Some(old) if keep_edits => edit::carry_over(old, suggested),
                        _ => suggested,
                    };
                    plan.set(Some(merged));
                    load_error.set(None);
                }
                Err(e) => load_error.set(Some(error_message(&e))),
            }
        });
    });

    // Preview of the current plan; the last one stays shown while the next loads.
    let preview_now = use_resource(move || async move {
        let _ = datasets_version();
        match plan() {
            Some(p) => Some(class_merge_preview(p).await),
            None => None,
        }
    });
    use_effect(move || {
        if let Some(Some(Ok(p))) = &*preview_now.read() {
            preview.set(Some(p.clone()));
        }
    });

    let datasets = use_resource(move || async move {
        let _ = datasets_version();
        list_datasets().await
    });
    let rows: Vec<DatasetRow> = datasets.read().as_ref().and_then(|r| r.as_ref().ok()).cloned().unwrap_or_default();
    let names: Vec<String> = inputs().iter().map(|id| rows.iter().find(|r| r.id == *id).map_or(format!("#{id}"), |r| r.name.clone())).collect();
    let taken: Vec<String> = rows.iter().map(|r| r.name.clone()).collect();

    rsx! {
        div { class: "section-stack",
            if !names.is_empty() {
                p { class: "field-hint inputs-line",
                    if names.len() == 1 { "Editing " } else { "Merging, in this order: " }
                    strong { {names.join(" + ")} }
                    ". Change the selection in the datasets panel."
                }
            }
            if let Some(message) = load_error() {
                p { class: "form-error", role: "alert", "{message}" }
            }
            match (plan(), preview()) {
                _ if inputs.read().is_empty() => rsx! {
                    SelectDatasetHint {}
                    // The last run stays visible, e.g. after it deleted its inputs.
                    LastRun {}
                },
                (Some(_), Some(p)) => rsx! {
                    TargetsCard { plan, sources: p.sources.iter().map(|s| s.name.clone()).collect::<Vec<_>>() }
                    MappingCard { plan, preview: p.clone() }
                    OptionsCard { plan, multiple: inputs.read().len() > 1 }
                    PreviewCard { plan: plan().unwrap(), preview: p.clone() }
                    RunCard { plan: plan().unwrap(), preview: p, names, taken }
                },
                _ => rsx! { p { class: "field-hint", "Reading class statistics…" } },
            }
        }
    }
}

/// The ordered output classes, plus saved class lists.
#[component]
fn TargetsCard(plan: Signal<Option<MergePlan>>, sources: Vec<String>) -> Element {
    let mut plan = plan;
    let mut new_name = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    // Bumped to reset inputs that were edited into an invalid name.
    let mut nonce = use_signal(|| 0u32);
    let mut lists_version = use_signal(|| 0u32);
    let lists = use_resource(move || async move {
        let _ = lists_version();
        class_lists().await
    });
    let mut chosen_list = use_signal(String::new);
    let mut list_name = use_signal(String::new);

    let targets = plan.read().as_ref().map(|p| p.targets.clone()).unwrap_or_default();
    let count = targets.len();
    let saved: Vec<ClassList> = lists.read().as_ref().and_then(|r| r.as_ref().ok()).cloned().unwrap_or_default();
    let saved_for_load = saved.clone();

    let mut edit = move |f: &mut dyn FnMut(&mut MergePlan) -> Result<(), String>| {
        let result = plan.write().as_mut().map_or(Ok(()), |p| f(p));
        match result {
            Ok(()) => error.set(None),
            Err(message) => {
                error.set(Some(message));
                *nonce.write() += 1;
            }
        }
    };

    let save_list = move |_| async move {
        let classes = plan.read().as_ref().map(|p| p.targets.clone()).unwrap_or_default();
        match save_class_list(ClassList { name: list_name().trim().to_string(), classes }).await {
            Ok(()) => {
                error.set(None);
                list_name.set(String::new());
                *lists_version.write() += 1;
            }
            Err(e) => error.set(Some(error_message(&e))),
        }
    };
    let delete_list = move |_| async move {
        if chosen_list().is_empty() {
            return;
        }
        match delete_class_list(chosen_list()).await {
            Ok(()) => {
                chosen_list.set(String::new());
                *lists_version.write() += 1;
            }
            Err(e) => error.set(Some(error_message(&e))),
        }
    };

    rsx! {
        section { class: "section-card", aria_labelledby: "targets-title",
            div { class: "section-card-header",
                h2 { id: "targets-title", class: "section-card-title", "Target classes" }
                span { class: "section-card-meta", "{count} classes · order sets the class ids" }
            }
            ol { class: "target-list",
                for (i, target) in targets.iter().enumerate() {
                    li { key: "{i}-{target}-{nonce}", class: "target-row",
                        span { class: "target-id", "{i}" }
                        input {
                            class: "input",
                            r#type: "text",
                            value: "{target}",
                            aria_label: "Name of class {i}",
                            onchange: {
                                let from = target.clone();
                                move |e: FormEvent| {
                                    let to = e.value();
                                    let from = from.clone();
                                    edit(&mut |p| edit::rename_target(p, &from, &to));
                                }
                            },
                        }
                        button { class: "icon-button icon-button-sm", r#type: "button", aria_label: "Move up", disabled: i == 0,
                            onclick: move |_| edit(&mut |p| { edit::move_target(p, i, true); Ok(()) }), "↑" }
                        button { class: "icon-button icon-button-sm", r#type: "button", aria_label: "Move down", disabled: i + 1 == count,
                            onclick: move |_| edit(&mut |p| { edit::move_target(p, i, false); Ok(()) }), "↓" }
                        button {
                            class: "icon-button icon-button-sm",
                            r#type: "button",
                            aria_label: "Remove class",
                            title: "Remove; classes mapped to it become unmapped",
                            onclick: {
                                let name = target.clone();
                                move |_| {
                                    let name = name.clone();
                                    edit(&mut |p| { p.remove_target(&name); Ok(()) });
                                }
                            },
                            "×"
                        }
                    }
                }
            }
            div { class: "input-row",
                input {
                    class: "input",
                    r#type: "text",
                    placeholder: "New class name",
                    value: "{new_name}",
                    oninput: move |e| new_name.set(e.value()),
                    onkeydown: move |e| {
                        if e.key() == Key::Enter {
                            e.prevent_default();
                            let name = new_name();
                            edit(&mut |p| edit::add_target(p, &name));
                            if error().is_none() {
                                new_name.set(String::new());
                            }
                        }
                    },
                }
                button {
                    class: "button button-secondary",
                    r#type: "button",
                    onclick: move |_| {
                        let name = new_name();
                        edit(&mut |p| edit::add_target(p, &name));
                        if error().is_none() {
                            new_name.set(String::new());
                        }
                    },
                    "Add class"
                }
            }

            div { class: "preset-row",
                span { class: "field-label", "Saved lists" }
                select {
                    class: "input select",
                    aria_label: "Saved class list",
                    value: "{chosen_list}",
                    onchange: move |e| chosen_list.set(e.value()),
                    option { value: "", "Choose a list…" }
                    for list in saved.iter() {
                        option { key: "{list.name}", value: "{list.name}", "{list.name} ({list.classes.len()})" }
                    }
                }
                button {
                    class: "button button-secondary button-sm",
                    r#type: "button",
                    disabled: chosen_list().is_empty(),
                    title: "Use this list as the target classes; matching names are mapped automatically",
                    onclick: move |_| {
                        if let Some(list) = saved_for_load.iter().find(|l| l.name == chosen_list()).cloned() {
                            let sources = sources.clone();
                            edit(&mut |p| { edit::apply_class_list(p, &list, &sources); Ok(()) });
                        }
                    },
                    "Use list"
                }
                button { class: "button button-ghost button-sm", r#type: "button", disabled: chosen_list().is_empty(), onclick: delete_list, "Delete" }
                input {
                    class: "input",
                    r#type: "text",
                    placeholder: "Save these classes as…",
                    value: "{list_name}",
                    oninput: move |e| list_name.set(e.value()),
                }
                button { class: "button button-secondary button-sm", r#type: "button", disabled: list_name().trim().is_empty() || count == 0, onclick: save_list, "Save list" }
            }
            if let Some(message) = error() {
                p { class: "field-error", role: "alert", "{message}" }
            }
        }
    }
}

/// Options for a mapping `select`.
#[component]
fn ChoiceOptions(targets: Vec<String>, current: Choice, allow_unmapped: bool, #[props(default)] same_label: Option<String>) -> Element {
    rsx! {
        if let Some(label) = same_label {
            option { value: "same", selected: false, "{label}" }
        }
        if allow_unmapped {
            option { value: "", selected: current == Choice::Unmapped, "— Choose —" }
        }
        optgroup { label: "Map to",
            for t in targets.iter() {
                option { key: "{t}", value: "t:{t}", selected: current == Choice::Target(t.clone()), "{t}" }
            }
            option { value: "new", "New class with this name" }
        }
        optgroup { label: "Drop",
            option { value: "drop:boxes", selected: current == Choice::DropBoxes, "Drop boxes (objects stay unlabeled)" }
            option { value: "drop:images", selected: current == Choice::DropImages, "Drop images containing it" }
        }
    }
}

/// Source classes and what happens to each.
#[component]
fn MappingCard(plan: Signal<Option<MergePlan>>, preview: MergePreview) -> Element {
    let mut plan = plan;
    let mut checked = use_signal(Vec::<String>::new);
    let mut bulk = use_signal(String::new);
    let mut expanded = use_signal(|| None::<String>);
    // Labels per input dataset as columns, instead of one total.
    let mut per_input = use_signal(|| false);

    let Some(current) = plan() else { return rsx! {} };
    let inputs = current.inputs.clone();
    let multiple = inputs.len() > 1;
    let targets = current.targets.clone();

    let apply_bulk = move |_| {
        let choice = Choice::parse(&bulk());
        if bulk().is_empty() {
            return;
        }
        let sources = checked();
        if let Some(p) = plan.write().as_mut() {
            for source in &sources {
                edit::set_choice(p, source, choice.clone());
            }
        }
        checked.set(Vec::new());
    };

    rsx! {
        section { class: "section-card", aria_labelledby: "mapping-title",
            div { class: "section-card-header",
                h2 { id: "mapping-title", class: "section-card-title", "Mapping" }
                span { class: "section-card-meta", "{preview.sources.len()} source classes" }
                if multiple {
                    button {
                        class: "button button-ghost button-sm",
                        r#type: "button",
                        aria_expanded: if per_input() { "true" } else { "false" },
                        onclick: move |_| per_input.toggle(),
                        if per_input() { "Hide labels per dataset" } else { "Show labels per dataset" }
                    }
                }
            }
            if !checked.read().is_empty() {
                div { class: "bulk-bar", role: "group", aria_label: "Assign selected classes",
                    span { "{checked.read().len()} selected →" }
                    select {
                        class: "input select",
                        aria_label: "Assign selected to",
                        value: "{bulk}",
                        onchange: move |e| bulk.set(e.value()),
                        ChoiceOptions { targets: targets.clone(), current: Choice::Unmapped, allow_unmapped: true }
                    }
                    button { class: "button button-sm", r#type: "button", disabled: bulk().is_empty(), onclick: apply_bulk, "Assign" }
                    button { class: "button button-ghost button-sm", r#type: "button", onclick: move |_| checked.set(Vec::new()), "Clear" }
                }
            }
            div { class: "table-scroll",
                table { class: "data-table mapping-table",
                    thead {
                        tr {
                            th { scope: "col", span { class: "visually-hidden", "Select" } }
                            th { scope: "col", "Source class" }
                            if per_input() {
                                for name in preview.input_names.iter() {
                                    th { key: "{name}", class: "num", scope: "col", title: "Labels in {name}", "{name}" }
                                }
                            } else {
                                th { class: "num", scope: "col", title: "Labels in all input datasets", "Labels" }
                            }
                            th { scope: "col", "Becomes" }
                            if multiple {
                                th { scope: "col", span { class: "visually-hidden", "Per dataset" } }
                            }
                        }
                    }
                    tbody {
                        for source in preview.sources.iter() {
                            {
                                let name = source.name.clone();
                                // The rule for the name (overrides show in the expanded row).
                                let rule = current.rules.iter().find(|r| r.source == name).map(|r| &r.action);
                                let choice = Choice::of(rule);
                                let matched = matches!(&choice, Choice::Target(t) if *t != name && normalize_class(t) == normalize_class(&name));
                                let is_checked = checked.read().contains(&name);
                                let open = expanded() == Some(name.clone());
                                let overridden = current.overrides.iter().any(|o| o.source == name);
                                let row_class = if choice == Choice::Unmapped { "is-unmapped" } else { "" };
                                rsx! {
                                    tr { key: "{name}", class: "{row_class}",
                                        td {
                                            input {
                                                r#type: "checkbox",
                                                aria_label: "Select {name}",
                                                checked: is_checked,
                                                onchange: {
                                                    let name = name.clone();
                                                    move |e: FormEvent| {
                                                        let name = name.clone();
                                                        if e.checked() { checked.write().push(name) } else { checked.write().retain(|n| *n != name) }
                                                    }
                                                },
                                            }
                                        }
                                        th { class: "name", scope: "row", title: "{name}", "{name}"
                                            if matched {
                                                span { class: "tag", title: "Joined by a similar name", "matched" }
                                            }
                                        }
                                        if per_input() {
                                            for (i, labels) in source.labels.iter().enumerate() {
                                                td { key: "{i}", class: "num muted", if *labels > 0 { "{format_count(*labels)}" } else { "–" } }
                                            }
                                        } else {
                                            td { class: "num muted", "{format_count(source.labels.iter().map(|&n| u64::from(n)).sum::<u64>())}" }
                                        }
                                        td {
                                            select {
                                                class: "input select",
                                                aria_label: "What {name} becomes",
                                                value: "{choice.value()}",
                                                onchange: {
                                                    let name = name.clone();
                                                    move |e: FormEvent| {
                                                        let name = name.clone();
                                                        if let Some(p) = plan.write().as_mut() {
                                                            edit::set_choice(p, &name, Choice::parse(&e.value()));
                                                        }
                                                    }
                                                },
                                                ChoiceOptions { targets: targets.clone(), current: choice.clone(), allow_unmapped: choice == Choice::Unmapped }
                                            }
                                        }
                                        if multiple {
                                            td {
                                                button {
                                                    class: if overridden { "button button-ghost button-sm is-active" } else { "button button-ghost button-sm" },
                                                    r#type: "button",
                                                    aria_expanded: if open { "true" } else { "false" },
                                                    title: "Different rule per dataset",
                                                    onclick: {
                                                        let name = name.clone();
                                                        move |_| expanded.set(if expanded() == Some(name.clone()) { None } else { Some(name.clone()) })
                                                    },
                                                    if overridden { "Per dataset ●" } else { "Per dataset" }
                                                }
                                            }
                                        }
                                    }
                                    if open {
                                        for (slot, dataset) in inputs.iter().enumerate().filter(|(slot, _)| source.labels[*slot] > 0) {
                                            {
                                                let dataset = *dataset;
                                                let own = current.overrides.iter().find(|o| o.dataset == dataset && o.source == name).map(|o| &o.action);
                                                let own_choice = own.map(|a| Choice::of(Some(a)));
                                                let label = preview.input_names.get(slot).cloned().unwrap_or_default();
                                                rsx! {
                                                    tr { key: "{name}-{dataset}", class: "override-row",
                                                        td {}
                                                        th { scope: "row", class: "muted", "in {label}" }
                                                        td { colspan: if per_input() { preview.input_names.len().to_string() } else { "1".to_string() } }
                                                        td {
                                                            select {
                                                                class: "input select",
                                                                aria_label: "What {name} becomes in {label}",
                                                                value: own_choice.as_ref().map_or("same".to_string(), |c| c.value()),
                                                                onchange: {
                                                                    let name = name.clone();
                                                                    move |e: FormEvent| {
                                                                        let value = e.value();
                                                                        let choice = if value == "same" { None } else { Some(Choice::parse(&value)) };
                                                                        if let Some(p) = plan.write().as_mut() {
                                                                            edit::set_override_choice(p, dataset, &name, choice);
                                                                        }
                                                                    }
                                                                },
                                                                ChoiceOptions {
                                                                    targets: targets.clone(),
                                                                    current: own_choice.clone().unwrap_or(Choice::Unmapped),
                                                                    allow_unmapped: false,
                                                                    same_label: "Same as all datasets".to_string(),
                                                                }
                                                            }
                                                        }
                                                        td {}
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
}

#[component]
fn OptionsCard(plan: Signal<Option<MergePlan>>, multiple: bool) -> Element {
    let mut plan = plan;
    let Some(current) = plan() else { return rsx! {} };
    let keep = current.split == SplitMode::Keep;

    rsx! {
        section { class: "section-card", aria_labelledby: "options-title",
            h2 { id: "options-title", class: "section-card-title", "Options" }
            div { class: "field field-inline",
                label { class: "field-label", r#for: "classes-format", "Output format" }
                select {
                    id: "classes-format",
                    class: "input select",
                    value: "{current.format}",
                    onchange: move |e| {
                        if let Some(p) = plan.write().as_mut() {
                            p.format = e.value();
                        }
                    },
                    for (value, label) in [("yolo", "YOLO"), ("coco", "COCO JSON"), ("voc", "Pascal VOC")] {
                        option { key: "{value}", value, selected: current.format == value, "{label}" }
                    }
                }
            }
            div { class: "choice-list",
                for (is_keep, label, detail) in [
                    (false, "Put everything into train", "Re-split afterwards with Split & Balance."),
                    (true, "Keep each dataset's splits", if multiple { "Images stay in train / valid / test as they were." } else { "Images stay in train / valid / test." }),
                ] {
                    label { key: "{label}", class: if keep == is_keep { "choice is-selected" } else { "choice" },
                        input {
                            r#type: "radio",
                            name: "split-mode",
                            checked: keep == is_keep,
                            onchange: move |_| {
                                if let Some(p) = plan.write().as_mut() {
                                    p.split = if is_keep { SplitMode::Keep } else { SplitMode::AllTrain };
                                }
                            },
                        }
                        span { class: "choice-body",
                            span { class: "choice-label",
                                "{label} "
                                InfoTip { label: "About {label}", text: "{detail}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn PreviewCard(plan: MergePlan, preview: MergePreview) -> Element {
    let bars: Vec<Bar> = preview
        .targets
        .iter()
        .map(|t| {
            let per: Vec<String> = t.labels.iter().zip(&preview.input_names).map(|(n, name)| format!("{name} {}", format_count(*n))).collect();
            Bar { label: t.target.clone(), value: t.labels.iter().sum(), detail: Some(format!("labels ({})", per.join(", "))) }
        })
        .collect();
    let name_of = |id: u32| {
        plan.inputs.iter().position(|&i| i == id).and_then(|p| preview.input_names.get(p)).cloned().unwrap_or_else(|| format!("#{id}"))
    };
    let missing: Vec<String> = preview
        .missing_labels
        .iter()
        .map(|m| {
            let names: Vec<String> = m.missing_in.iter().map(|&id| name_of(id)).collect();
            format!(
                "\"{}\" isn't labelled in {}: {} images there may contain unlabeled {}.",
                m.target,
                names.join(", "),
                format_count(m.images_at_risk),
                m.target
            )
        })
        .collect();
    let unmapped: Vec<String> = preview.unmapped.iter().map(|(id, class)| format!("\"{class}\" in {}", name_of(*id))).collect();
    let mut errors = preview.problems.clone();
    if !unmapped.is_empty() {
        errors.push(format!("Choose what happens to: {}", unmapped.join(", ")));
    }
    let warnings: Vec<String> = preview.empty_targets.iter().map(|t| format!("\"{t}\" gets no labels.")).chain(missing).collect();

    rsx! {
        section { class: "section-card", aria_labelledby: "preview-title",
            div { class: "section-card-header",
                h2 { id: "preview-title", class: "section-card-title", "Result" }
                span { class: "section-card-aside",
                    span { class: "section-card-meta", "labels per target class" }
                    WarningTip { errors, warnings }
                }
            }
            BarList { bars, label: "Labels per target class" }
            if preview.dropped_labels > 0 || preview.dropped_images > 0 {
                p { class: "field-hint",
                    if preview.dropped_labels > 0 { "{format_count(preview.dropped_labels)} labels dropped. " }
                    if preview.dropped_images > 0 { "Up to {format_count(preview.dropped_images)} images dropped." }
                }
            }
        }
    }
}

#[component]
fn RunCard(plan: MergePlan, preview: MergePreview, names: Vec<String>, taken: Vec<String>) -> Element {
    let output = use_signal(OutputChoice::default);
    let mut delete_inputs = use_signal(|| false);
    let mut submitted = use_signal(|| false);
    let mut form_error = use_signal(|| None::<String>);
    let (job, _) = use_job(JobKind::ClassesMerge);
    let running = job.read().as_ref().is_some_and(|j| !j.finished);

    let single = names.len() == 1;
    let suggested = if single { format!("{}-classes", names[0]) } else { free_name("new-dataset", &taken) };
    let source_name = single.then(|| names[0].clone());
    let can_run = preview.can_run() && !running;
    let progress_title = if single { "Applying" } else { "Merging" };
    let sources_text = plural(names.len(), "source dataset");

    let suggested_for_run = suggested.clone();
    let submit = move |e: FormEvent| {
        let plan = plan.clone();
        let suggested = suggested_for_run.clone();
        async move {
            e.prevent_default();
            submitted.set(true);
            form_error.set(None);
            let Ok(target) = output.read().resolve(&suggested, single) else { return };
            match start_class_merge(plan, target, !single && delete_inputs()).await {
                Ok(started) => {
                    set_if_newer(job, started);
                    submitted.set(false);
                    delete_inputs.set(false);
                }
                Err(e) => form_error.set(Some(error_message(&e))),
            }
        }
    };

    rsx! {
        form { class: "section-stack", novalidate: true, onsubmit: submit,
            OutputSelect { choice: output, suggested_name: suggested, source_name, show_errors: submitted(), disabled: running }
            if !single {
                label { class: "check",
                    input {
                        r#type: "checkbox",
                        checked: delete_inputs(),
                        disabled: running,
                        onchange: move |e| delete_inputs.set(e.checked()),
                    }
                    span { "Delete the {sources_text} once the merged one is saved. This can't be undone." }
                }
            }
            if let Some(message) = form_error() {
                p { class: "form-error", role: "alert", "{message}" }
            }
            div { class: "form-actions",
                button { class: "button", r#type: "submit", disabled: !can_run,
                    if running { "Working…" } else if single { "Apply" } else if delete_inputs() { "Merge and delete sources" } else { "Merge" }
                }
                if !preview.can_run() {
                    span { class: "form-actions-note", "Resolve the problems in Result first." }
                }
            }
        }
        if let Some(current) = job() {
            JobProgress { job: current, title: progress_title, noun: "dataset", verb: "written" }
        }
    }
}

/// The latest run, for when no dataset is selected.
#[component]
fn LastRun() -> Element {
    let (job, _) = use_job(JobKind::ClassesMerge);
    rsx! {
        if let Some(current) = job() {
            JobProgress { job: current, title: "Last run", noun: "dataset", verb: "written" }
        }
    }
}

/// `base`, or `base-2`, `base-3`… if a dataset already has that name.
fn free_name(base: &str, taken: &[String]) -> String {
    let free = |name: &String| !taken.iter().any(|t| t.eq_ignore_ascii_case(name));
    std::iter::once(base.to_string()).chain((2..).map(|n| format!("{base}-{n}"))).find(free).unwrap()
}

#[cfg(test)]
mod tests {
    use super::free_name;

    #[test]
    fn picks_a_free_name() {
        assert_eq!(free_name("new-dataset", &[]), "new-dataset");
        let taken = vec!["New-Dataset".to_string(), "new-dataset-2".to_string()];
        assert_eq!(free_name("new-dataset", &taken), "new-dataset-3");
    }
}
