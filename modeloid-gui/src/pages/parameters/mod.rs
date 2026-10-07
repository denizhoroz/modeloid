//! Training → Parameters: build a reusable set of model and training
//! settings, saved in the workspace as `training/parameters/<name>.toml`.
//! Choosing a set in the side panel's Parameters table opens it here.

pub(crate) mod fields;

use dioxus::prelude::*;

use crate::api::datasets::{list_datasets, DatasetRow};
use crate::components::{IconName, TitleIcon};
use crate::api::error_message;
use crate::api::parameters::{
    list_parameters, save_parameters, AugmentationConfig, AutoAugment, Batch, Cache, Family, FrameworkParams,
    LossConfig, ModelSize, Optimizer, ParameterSet, RunConfig, ScheduleConfig, Task, UltralyticsParams, Weights,
};
use crate::components::{use_panel_table, use_toasts, InfoTip, NoWorkspace, PanelTable, PythonNotice, ToastKind};
use crate::components::table;
use crate::state::{
    use_datasets_version, use_parameters_version, use_primary_dataset, use_primary_parameters, use_selected_parameters,
    use_workspace, use_workspace_checked,
};
use crate::util::format_count;
use fields::{options, BoolField, ChoiceField, FloatField, IntField, OptionalFloatField, OptionalIntField};

/// The form's Ultralytics settings.
pub(crate) fn ultra(set: &ParameterSet) -> &UltralyticsParams {
    let FrameworkParams::Ultralytics(p) = &set.params;
    p
}

/// Changes the form's Ultralytics settings.
pub(crate) fn edit(mut form: Signal<ParameterSet>, f: impl FnOnce(&mut UltralyticsParams)) {
    let mut set = form.write();
    let FrameworkParams::Ultralytics(p) = &mut set.params;
    f(p);
}

/// The choice whose `id` is `value`.
fn pick<T: Copy>(all: &[T], id: impl Fn(T) -> &'static str, value: &str) -> Option<T> {
    all.iter().copied().find(|&t| id(t) == value)
}

#[component]
pub fn Parameters() -> Element {
    let workspace = use_workspace();
    let checked = use_workspace_checked();

    rsx! {
        document::Title { "Parameters · Modeloid" }
        div { class: "page page-wide",
            p { class: "page-eyebrow", "Training" }
            h1 { "Parameters" }
            p { class: "page-lead",
                "A reusable recipe of model and training settings. Train Model picks the dataset; pick a set in the Parameters table to change it."
            }
            PythonNotice { text: "You can save parameter sets now. Training them needs the training libraries (PyTorch, Ultralytics, Optuna), which aren't installed yet." }
            if workspace.read().is_some() {
                ParameterEditor {}
            } else if checked() {
                NoWorkspace { text: "Parameter sets are saved in a workspace." }
            }
        }
    }
}

#[component]
fn ParameterEditor() -> Element {
    let mut selected = use_selected_parameters();
    let primary = use_primary_parameters();
    // Selects only `name` (nothing for `None`).
    let mut select_only = move |name: Option<String>| selected.set(name.into_iter().collect());
    let mut version = use_parameters_version();
    let toasts = use_toasts();
    let list = use_resource(move || async move {
        let _ = version();
        list_parameters().await
    });

    // The set the form is based on (`None` = a new one) and its saved state.
    let mut editing = use_signal(|| None::<String>);
    let mut base = use_signal(|| ParameterSet::new(""));
    let mut form = use_signal(|| ParameterSet::new(""));
    // A table choice waiting on "discard your changes?".
    let mut pending = use_signal(|| None::<Option<String>>);
    let mut saving = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);

    let dirty = use_memo(move || !form.read().same_settings(&base.read()));

    let mut open = move |name: Option<String>, sets: &[ParameterSet]| {
        let set = match &name {
            Some(n) => match sets.iter().find(|s| &s.name == n) {
                Some(set) => set.clone(),
                None => return, // not in the list (yet)
            },
            None => ParameterSet::new(""),
        };
        editing.set(name);
        base.set(set.clone());
        form.set(set);
        pending.set(None);
        error.set(None);
    };

    // Follow the table: open the chosen set, unless that would throw away changes.
    use_effect(move || {
        let wanted = primary();
        let Some(Ok(list)) = &*list.read() else { return };
        // The set being edited was removed (Remove in the table, or on disk):
        // the form keeps its values as a new, unsaved set.
        let gone = editing.peek().as_ref().is_some_and(|name| !list.sets.iter().any(|s| &s.name == name));
        if gone {
            editing.set(None);
            base.set(ParameterSet::new(""));
        }
        if wanted == *editing.peek() {
            return;
        }
        if let Some(name) = &wanted {
            if !list.sets.iter().any(|s| &s.name == name) {
                select_only(editing.peek().clone()); // gone from disk
                return;
            }
        }
        if *dirty.peek() {
            pending.set(Some(wanted));
        } else {
            open(wanted, &list.sets);
        }
    });

    let sets = list.read().as_ref().and_then(|r| r.as_ref().ok()).map(|l| l.sets.clone()).unwrap_or_default();
    let set = form();
    let p = ultra(&set).clone();
    let problems = set.problems();
    let is_edit = editing().is_some();
    let can_save = problems.is_empty() && !saving() && (!is_edit || dirty());

    let save = move |e: FormEvent| async move {
        e.prevent_default();
        let replacing = editing();
        saving.set(true);
        error.set(None);
        match save_parameters(form(), replacing.clone()).await {
            Ok(saved) => {
                let verb = if replacing.is_some() { "Saved" } else { "Added" };
                toasts.show(ToastKind::Success, format!("{verb} parameter set {}", saved.name));
                editing.set(Some(saved.name.clone()));
                base.set(saved.clone());
                form.set(saved.clone());
                // A renamed set keeps its place in the selection.
                let new_name = saved.name.clone();
                match replacing {
                    Some(old) if selected.peek().contains(&old) => {
                        table::update(selected, |s| s.iter().map(|n| if *n == old { new_name.clone() } else { n.clone() }).collect())
                    }
                    _ => select_only(Some(saved.name)),
                }
                *version.write() += 1;
            }
            Err(e) => error.set(Some(error_message(&e))),
        }
        saving.set(false);
    };

    rsx! {
        form { class: "section-stack", novalidate: true, onsubmit: save,
            // The selected set's name (a new one has none yet).
            h2 { class: "param-title",
                if let Some(name) = editing() { "{name}" } else { "New parameter set" }
            }
            if let Some(wanted) = pending() {
                div { class: "notice notice-warning param-confirm", role: "alert",
                    span { "You have unsaved changes. Open "
                        strong { {wanted.clone().unwrap_or_else(|| "a new set".into())} }
                        " anyway?"
                    }
                    button {
                        class: "button button-danger button-sm",
                        r#type: "button",
                        onclick: move |_| open(wanted.clone(), &sets),
                        "Discard changes"
                    }
                    button {
                        class: "button button-secondary button-sm",
                        r#type: "button",
                        onclick: move |_| {
                            pending.set(None);
                            select_only(editing.peek().clone());
                        },
                        "Keep editing"
                    }
                }
            }

            PrepareSection { key: "{editing().unwrap_or_default()}", form, editing: editing().is_some() }
            ModelSection { form, p: p.clone() }
            TrainingSection { form, p: p.clone() }
            ScheduleSection { form, p: p.clone() }
            AugmentationSection { form, p: p.clone() }
            LossSection { form, p: p.clone() }
            RunSection { form, p: p.clone() }

            section { class: "section-card", aria_labelledby: "param-output-title",
                h2 { id: "param-output-title", class: "section-card-title", TitleIcon { name: IconName::Save } "Output" }
                div { class: "field",
                    label { class: "field-label", r#for: "param-name", "Parameter name" }
                    input {
                        id: "param-name",
                        class: "input",
                        value: "{set.name}",
                        placeholder: "e.g. yolo11n-baseline",
                        aria_describedby: "param-name-hint",
                        oninput: move |e| form.write().name = e.value(),
                    }
                    p { id: "param-name-hint", class: "field-hint",
                        if is_edit { "Changing the name renames the set's file." } else { "Saved as training/parameters/<name>.toml in the workspace." }
                    }
                }
                div { class: "field",
                    label { class: "field-label", r#for: "param-description", "Description (optional)" }
                    input {
                        id: "param-description",
                        class: "input",
                        value: "{set.description.clone().unwrap_or_default()}",
                        oninput: move |e| {
                            let text = e.value();
                            form.write().description = if text.trim().is_empty() { None } else { Some(text) };
                        },
                    }
                }
                if !problems.is_empty() {
                    ul { class: "param-problems", aria_label: "Fix these first",
                        for problem in problems.iter() {
                            li { key: "{problem}", class: "field-error", "{problem}" }
                        }
                    }
                }
                if let Some(message) = error() {
                    p { class: "form-error", role: "alert", "{message}" }
                }
                div { class: "form-actions",
                    button { class: "button", r#type: "submit", disabled: !can_save,
                        if saving() { "Saving…" } else if is_edit { "Edit Parameter" } else { "Add Parameter" }
                    }
                    if is_edit && !dirty() {
                        span { class: "form-actions-note", "Change something to save it." }
                    }
                }
            }
        }
    }
}

/// The task a dataset's labels train: outlines segment, class folders
/// classify, boxes detect.
fn dataset_task(row: &DatasetRow) -> Task {
    match row.task.as_str() {
        "instance-segmentation" => Task::Segment,
        "classification" | "single-label-classification" => Task::Classify,
        _ => Task::Detect,
    }
}

/// Sets the model up for `row`: its task, and a family that has (pretrained)
/// weights for it. The name is suggested when empty.
fn prepare_from(set: &mut ParameterSet, row: &DatasetRow) {
    let task = dataset_task(row);
    let FrameworkParams::Ultralytics(p) = &mut set.params;
    let m = &mut p.model;
    m.task = task;
    let pretrained = m.weights == Weights::Pretrained;
    if !m.family.tasks().contains(&task) || (pretrained && !m.family.pretrained_tasks().contains(&task)) {
        m.family = Family::Yolo26;
    }
    if !m.family.sizes().contains(&m.size) {
        m.size = m.family.sizes()[0];
    }
    if set.name.trim().is_empty() {
        set.name = format!("{}-{}", row.name, task.id());
    }
}

/// What training does with a dataset's label format.
fn format_note(format: &str) -> &'static str {
    let format = format.trim().to_ascii_lowercase();
    if format.starts_with("yolo") {
        "YOLO labels are trained on as they are."
    } else if format == "folders" {
        "Class folders are trained on as they are."
    } else if format.starts_with("coco") || format.starts_with("voc") || format == "pascal-voc" {
        "Training writes a YOLO copy of the labels into the run folder."
    } else {
        "This label format can't be trained on yet; Convert can turn it into YOLO."
    }
}

/// Fill the model settings from a dataset (open by default): the dataset
/// selected last (anywhere in the app) is used as soon as it is selected, and
/// again whenever the selection changes. A saved set opened for editing isn't
/// changed by the selection it opens with. "Open Datasets" only shows the table.
#[component]
fn PrepareSection(form: Signal<ParameterSet>, editing: bool) -> Element {
    let mut form = form;
    let mut panel = use_panel_table();
    let primary = use_primary_dataset();
    let datasets_version = use_datasets_version();
    let datasets = use_resource(move || async move {
        let _ = datasets_version();
        list_datasets().await
    });
    // The dataset applied last; a saved set starts from the current selection, so it stays as saved.
    let mut applied = use_signal(move || if editing { Some(*primary.peek()) } else { None });
    // The dataset used last, for the summary.
    let mut used = use_signal(|| None::<DatasetRow>);

    // A newly selected dataset is applied once its row is known.
    use_effect(move || {
        let current = primary();
        let rows = datasets.read();
        let Some(Ok(rows)) = rows.as_ref() else { return };
        if applied.peek().as_ref() == Some(&current) {
            return;
        }
        applied.set(Some(current));
        if let Some(row) = current.and_then(|id| rows.iter().find(|r| r.id == id).cloned()) {
            prepare_from(&mut form.write(), &row);
            used.set(Some(row));
        }
    });

    let meta = match used() {
        Some(row) => format!("from {}", row.name),
        None => "optional".to_string(),
    };

    rsx! {
        details { class: "section-card param-group", open: true, aria_labelledby: "param-prepare-title",
            summary { class: "param-group-summary",
                h2 { id: "param-prepare-title", class: "section-card-title",
                    TitleIcon { name: IconName::Wand }
                    "Prepare from dataset "
                    InfoTip { label: "About preparing from a dataset", text: "Sets the task and a model that fits it from the selected dataset's labels (the last one when several are selected). Everything stays editable." }
                }
                span { class: "section-card-meta", "{meta}" }
            }
            match (primary(), used()) {
                (Some(_), Some(row)) => rsx! {
                    div { class: "train-input-chosen",
                        span { class: "train-input-name", "{row.name}" }
                        span { class: "train-input-detail",
                            "{row.task} · {row.format} · {format_count(row.images)} images · {row.classes} classes → task {dataset_task(&row).label()}. {format_note(&row.format)}"
                        }
                    }
                },
                (Some(_), None) => rsx! {},
                (None, _) => rsx! {
                    p { class: "field-hint", "Select a dataset to prepare the parameters from it, according to its properties." }
                },
            }
            div { class: "form-actions",
                button {
                    class: "button button-secondary button-sm",
                    r#type: "button",
                    onclick: move |_| panel.set(PanelTable::Datasets),
                    "Open Datasets"
                }
            }
        }
    }
}

#[component]
pub(crate) fn ModelSection(form: Signal<ParameterSet>, p: UltralyticsParams) -> Element {
    let m = p.model.clone();
    let weights_kind = match &m.weights {
        Weights::Pretrained => "pretrained",
        Weights::FromScratch => "scratch",
        Weights::File(_) => "file",
    };
    let sizes: Vec<(String, String)> = m.family.sizes().iter().map(|s| (s.id().to_string(), format!("{} ({})", s.label(), s.id()))).collect();
    let tasks = options(m.family.tasks(), Task::id, Task::label);

    rsx! {
        section { class: "section-card", aria_labelledby: "param-model-title",
            div { class: "section-card-header",
                h2 { id: "param-model-title", class: "section-card-title", TitleIcon { name: IconName::Cpu } "Model" }
                span { class: "section-card-meta", "Ultralytics · {p.model_name()}" }
            }
            div { class: "param-grid",
                ChoiceField {
                    label: "Task",
                    tip: "What the model learns: boxes (detect), masks (segment), one label per image (classify), keypoints (pose) or rotated boxes.",
                    options: tasks,
                    value: m.task.id(),
                    onchange: move |v: String| if let Some(t) = pick(&Task::ALL, Task::id, &v) { edit(form, |p| p.model.task = t) },
                }
                ChoiceField {
                    label: "Model family",
                    tip: "YOLO26 is Ultralytics' newest and the default (every task); YOLO11 and YOLOv8 are widely used (every task); YOLO12, YOLOv10, YOLOv5u and RT-DETR (transformer) are detection only.",
                    options: options(&Family::ALL, Family::label, Family::label),
                    value: m.family.label(),
                    onchange: move |v: String| if let Some(f) = pick(&Family::ALL, Family::label, &v) {
                        edit(form, |p| {
                            p.model.family = f;
                            // Keep the choice valid for the new family.
                            if !f.sizes().contains(&p.model.size) { p.model.size = f.sizes()[0]; }
                            if !f.tasks().contains(&p.model.task) { p.model.task = Task::Detect; }
                        })
                    },
                }
                ChoiceField {
                    label: "Size",
                    tip: "Bigger models are more accurate but slower and need more GPU memory. Nano and Small suit most first runs.",
                    options: sizes,
                    value: m.size.id(),
                    onchange: move |v: String| if let Some(s) = pick(&ModelSize::ALL, ModelSize::id, &v) { edit(form, |p| p.model.size = s) },
                }
                ChoiceField {
                    label: "Starting weights",
                    tip: "Pretrained weights (COCO, or ImageNet for classify) train faster and better on small datasets. From scratch starts random; a file continues from your own weights.",
                    options: vec![
                        ("pretrained".into(), "Pretrained".into()),
                        ("scratch".into(), "From scratch".into()),
                        ("file".into(), "Weights file".into()),
                    ],
                    value: weights_kind,
                    onchange: move |v: String| edit(form, |p| p.model.weights = match v.as_str() {
                        "scratch" => Weights::FromScratch,
                        "file" => Weights::File(String::new()),
                        _ => Weights::Pretrained,
                    }),
                }
            }
            if let Weights::File(path) = &m.weights {
                div { class: "field",
                    label { class: "field-label", r#for: "param-weights-file",
                        "Weights file"
                        InfoTip { text: "Path to a .pt (weights) or .yaml (architecture) file, e.g. an earlier run's best.pt.", label: "About the weights file" }
                    }
                    input {
                        id: "param-weights-file",
                        class: "input",
                        value: "{path}",
                        placeholder: "e.g. training/trains/baseline/weights/best.pt",
                        oninput: move |e| edit(form, |p| p.model.weights = Weights::File(e.value())),
                    }
                }
            }
        }
    }
}

#[component]
fn TrainingSection(form: Signal<ParameterSet>, p: UltralyticsParams) -> Element {
    let t = p.training.clone();
    let task = p.model.task;

    rsx! {
        section { class: "section-card", aria_labelledby: "param-training-title",
            h2 { id: "param-training-title", class: "section-card-title", TitleIcon { name: IconName::Sliders } "Training" }
            div { class: "param-grid",
                IntField { label: "Epochs", tip: "Passes over the training images.", value: t.epochs, max: 10_000,
                    onchange: move |v| edit(form, |p| p.training.epochs = v) }
                IntField { label: "Image size", tip: "Images are resized to this many pixels on their long side. A multiple of 32; 640 is the usual choice.", value: t.imgsz, max: 4096,
                    onchange: move |v| edit(form, |p| p.training.imgsz = v) }
                div { class: "field param-field",
                    span { class: "field-label", "Batch size" InfoTip { text: "Images per training step. Auto picks the largest batch that fits about 60% of GPU memory.", label: "About batch size" } }
                    div { class: "param-optional",
                        if let Batch::Size(n) = t.batch {
                            crate::components::NumberInput { value: n, label: "Batch size", max: 1024,
                                onchange: move |v| edit(form, |p| p.training.batch = Batch::Size(v)) }
                        }
                        label { class: "check",
                            input {
                                r#type: "checkbox",
                                checked: t.batch == Batch::Auto,
                                onchange: move |e| edit(form, |p| p.training.batch = if e.checked() { Batch::Auto } else { Batch::Size(16) }),
                            }
                            span { "Auto" }
                        }
                    }
                }
                IntField { label: "Patience", tip: "Stop early after this many epochs without improvement. 0 never stops early.", value: t.patience, max: 10_000,
                    onchange: move |v| edit(form, |p| p.training.patience = v) }
                ChoiceField {
                    label: "Optimizer",
                    tip: "Auto picks AdamW for short runs and SGD for long ones, and sets its own learning rate.",
                    options: options(&Optimizer::ALL, Optimizer::id, Optimizer::id),
                    value: t.optimizer.id(),
                    onchange: move |v: String| if let Some(o) = pick(&Optimizer::ALL, Optimizer::id, &v) { edit(form, |p| p.training.optimizer = o) },
                }
                FloatField { label: "Learning rate", tip: "Initial learning rate (lr0). Ignored when the optimizer is Auto. SGD ~0.01, Adam ~0.001.", value: t.lr0, min: 0.0, max: 1.0, step: 0.001,
                    onchange: move |v| edit(form, |p| p.training.lr0 = v) }
                FloatField { label: "Fraction", tip: "Share of the training images used, e.g. 0.1 for a quick trial run.", value: t.fraction, min: 0.01, max: 1.0, step: 0.05,
                    onchange: move |v| edit(form, |p| p.training.fraction = v) }
                OptionalIntField { label: "Freeze layers", tip: "Keep the first N layers fixed (transfer learning). Off trains every layer.", value: t.freeze, on_value: 10, max: 100,
                    onchange: move |v| edit(form, |p| p.training.freeze = v) }
                if task == Task::Classify {
                    FloatField { label: "Dropout", tip: "Randomly drops connections while training, against overfitting. Classification only.", value: t.dropout, min: 0.0, max: 0.99, step: 0.05,
                        onchange: move |v| edit(form, |p| p.training.dropout = v) }
                }
            }
            div { class: "param-checks",
                BoolField { label: "Validate during training", tip: "Measure accuracy on the validation split after each epoch.", value: t.val,
                    onchange: move |v| edit(form, |p| p.training.val = v) }
                BoolField { label: "Rectangular batches", tip: "Less padding for non-square images; turns off shuffling.", value: t.rect,
                    onchange: move |v| edit(form, |p| p.training.rect = v) }
                BoolField { label: "Multi-scale", tip: "Vary the image size ±50% between batches for robustness; slower.", value: t.multi_scale,
                    onchange: move |v| edit(form, |p| p.training.multi_scale = v) }
                BoolField { label: "Single class", tip: "Treat every class as one 'object' class.", value: t.single_cls,
                    onchange: move |v| edit(form, |p| p.training.single_cls = v) }
            }
        }
    }
}

/// A collapsed section with "Reset to defaults".
#[component]
fn Group(#[props(into)] id: String, #[props(into)] title: String, #[props(into)] meta: String, on_reset: EventHandler<()>, children: Element) -> Element {
    rsx! {
        details { class: "section-card param-group", aria_labelledby: "{id}",
            summary { class: "param-group-summary",
                h2 { id: "{id}", class: "section-card-title", TitleIcon { name: IconName::Sliders } "{title}" }
                span { class: "section-card-meta", "{meta}" }
            }
            {children}
            div { class: "form-actions",
                button { class: "button button-ghost button-sm", r#type: "button", onclick: move |_| on_reset.call(()), "Reset to defaults" }
            }
        }
    }
}

#[component]
fn ScheduleSection(form: Signal<ParameterSet>, p: UltralyticsParams) -> Element {
    let s = p.schedule.clone();
    let meta = if s == ScheduleConfig::default() { "defaults" } else { "changed" };

    rsx! {
        Group { id: "param-schedule-title", title: "LR schedule & warmup", meta, on_reset: move |_| edit(form, |p| p.schedule = ScheduleConfig::default()),
            div { class: "param-grid",
                FloatField { label: "Final learning rate", tip: "Learning rate at the end, as a share of the initial one (lrf).", value: s.lrf, min: 0.0, max: 1.0, step: 0.01,
                    onchange: move |v| edit(form, |p| p.schedule.lrf = v) }
                FloatField { label: "Momentum", tip: "SGD momentum, or Adam's beta1.", value: s.momentum, min: 0.0, max: 1.0, step: 0.001,
                    onchange: move |v| edit(form, |p| p.schedule.momentum = v) }
                FloatField { label: "Weight decay", tip: "L2 penalty on weights, against overfitting.", value: s.weight_decay, min: 0.0, max: 0.1, step: 0.0001,
                    onchange: move |v| edit(form, |p| p.schedule.weight_decay = v) }
                FloatField { label: "Warmup epochs", tip: "Epochs at the start where the learning rate ramps up.", value: s.warmup_epochs, min: 0.0, max: 1000.0, step: 0.5,
                    onchange: move |v| edit(form, |p| p.schedule.warmup_epochs = v) }
                FloatField { label: "Warmup momentum", tip: "Momentum at the start of warmup.", value: s.warmup_momentum, min: 0.0, max: 1.0, step: 0.05,
                    onchange: move |v| edit(form, |p| p.schedule.warmup_momentum = v) }
                FloatField { label: "Warmup bias LR", tip: "Learning rate of bias parameters during warmup.", value: s.warmup_bias_lr, min: 0.0, max: 1.0, step: 0.01,
                    onchange: move |v| edit(form, |p| p.schedule.warmup_bias_lr = v) }
                IntField { label: "Nominal batch size", tip: "The loss is scaled as if batches had this many images (nbs).", value: s.nbs, max: 1024,
                    onchange: move |v| edit(form, |p| p.schedule.nbs = v) }
            }
            div { class: "param-checks",
                BoolField { label: "Cosine schedule", tip: "Lower the learning rate along a cosine curve instead of linearly.", value: s.cos_lr,
                    onchange: move |v| edit(form, |p| p.schedule.cos_lr = v) }
            }
        }
    }
}

/// Label, tip, value, max, step and setter of one augmentation number.
type AugField = (&'static str, &'static str, f64, f64, f64, fn(&mut AugmentationConfig, f64));

#[component]
fn AugmentationSection(form: Signal<ParameterSet>, p: UltralyticsParams) -> Element {
    let a = p.augmentation.clone();
    let task = p.model.task;
    let meta = if a == AugmentationConfig::default() { "defaults" } else { "changed" };
    let probs: [AugField; 15] = [
        ("Hue shift", "Random hue change, as a share of the full circle.", a.hsv_h, 1.0, 0.005, |a, v| a.hsv_h = v),
        ("Saturation shift", "Random saturation change.", a.hsv_s, 1.0, 0.05, |a, v| a.hsv_s = v),
        ("Brightness shift", "Random brightness change.", a.hsv_v, 1.0, 0.05, |a, v| a.hsv_v = v),
        ("Rotation (°)", "Random rotation up to this many degrees either way.", a.degrees, 180.0, 1.0, |a, v| a.degrees = v),
        ("Translate", "Random shift, as a share of the image size.", a.translate, 1.0, 0.05, |a, v| a.translate = v),
        ("Scale", "Random zoom in or out by up to this share.", a.scale, 1.0, 0.05, |a, v| a.scale = v),
        ("Shear (°)", "Random shear up to this many degrees.", a.shear, 180.0, 1.0, |a, v| a.shear = v),
        ("Perspective", "Random perspective warp; keep tiny (up to 0.001).", a.perspective, 0.001, 0.0001, |a, v| a.perspective = v),
        ("Vertical flip", "Chance of flipping upside down. Leave 0 unless up/down doesn't matter (e.g. aerial images).", a.flipud, 1.0, 0.1, |a, v| a.flipud = v),
        ("Horizontal flip", "Chance of a left–right flip.", a.fliplr, 1.0, 0.1, |a, v| a.fliplr = v),
        ("Channel swap", "Chance of swapping RGB to BGR.", a.bgr, 1.0, 0.1, |a, v| a.bgr = v),
        ("Mosaic", "Chance of combining 4 images into one. Strong for detection.", a.mosaic, 1.0, 0.1, |a, v| a.mosaic = v),
        ("Mixup", "Chance of blending two images and their labels.", a.mixup, 1.0, 0.05, |a, v| a.mixup = v),
        ("CutMix", "Chance of pasting a patch of another image.", a.cutmix, 1.0, 0.05, |a, v| a.cutmix = v),
        ("Copy-paste", "Chance of copying objects between images. Segmentation only.", a.copy_paste, 1.0, 0.05, |a, v| a.copy_paste = v),
    ];

    rsx! {
        Group { id: "param-aug-title", title: "Augmentation", meta, on_reset: move |_| edit(form, |p| p.augmentation = AugmentationConfig::default()),
            div { class: "param-grid",
                for (label, tip, value, max, step, set) in probs {
                    if label != "Copy-paste" || task == Task::Segment {
                        FloatField { key: "{label}", label, tip, value, min: 0.0, max, step,
                            onchange: move |v| edit(form, |p| set(&mut p.augmentation, v)) }
                    }
                }
                IntField { label: "Close mosaic", tip: "Turn mosaic off for the last N epochs, so the model finishes on real-looking images.", value: a.close_mosaic, max: 10_000,
                    onchange: move |v| edit(form, |p| p.augmentation.close_mosaic = v) }
                if task == Task::Classify {
                    FloatField { label: "Random erasing", tip: "Chance of erasing a random patch. Classification only.", value: a.erasing, min: 0.0, max: 1.0, step: 0.05,
                        onchange: move |v| edit(form, |p| p.augmentation.erasing = v) }
                    ChoiceField {
                        label: "Auto augment",
                        tip: "Automatic augmentation policy. Classification only.",
                        options: options(&AutoAugment::ALL, AutoAugment::label, AutoAugment::label),
                        value: a.auto_augment.label(),
                        onchange: move |v: String| if let Some(x) = pick(&AutoAugment::ALL, AutoAugment::label, &v) { edit(form, |p| p.augmentation.auto_augment = x) },
                    }
                }
            }
        }
    }
}

#[component]
fn LossSection(form: Signal<ParameterSet>, p: UltralyticsParams) -> Element {
    let l = p.loss.clone();
    let task = p.model.task;
    let meta = if l == LossConfig::default() { "defaults" } else { "changed" };

    rsx! {
        Group { id: "param-loss-title", title: "Loss weights", meta, on_reset: move |_| edit(form, |p| p.loss = LossConfig::default()),
            div { class: "param-grid",
                FloatField { label: "Box", tip: "Weight of the box position loss.", value: l.box_gain, min: 0.0, max: 100.0, step: 0.5,
                    onchange: move |v| edit(form, |p| p.loss.box_gain = v) }
                FloatField { label: "Class", tip: "Weight of the classification loss. Raise it when classes get mixed up.", value: l.cls, min: 0.0, max: 100.0, step: 0.1,
                    onchange: move |v| edit(form, |p| p.loss.cls = v) }
                FloatField { label: "DFL", tip: "Weight of the distribution focal loss, for box edge precision.", value: l.dfl, min: 0.0, max: 100.0, step: 0.1,
                    onchange: move |v| edit(form, |p| p.loss.dfl = v) }
                if task == Task::Pose {
                    FloatField { label: "Pose", tip: "Weight of the keypoint loss. Pose only.", value: l.pose, min: 0.0, max: 100.0, step: 0.5,
                        onchange: move |v| edit(form, |p| p.loss.pose = v) }
                    FloatField { label: "Keypoint objectness", tip: "Weight of the keypoint visibility loss. Pose only.", value: l.kobj, min: 0.0, max: 100.0, step: 0.1,
                        onchange: move |v| edit(form, |p| p.loss.kobj = v) }
                }
                if task == Task::Segment {
                    IntField { label: "Mask ratio", tip: "Masks are downsampled by this factor while training. Segmentation only.", value: l.mask_ratio, max: 32,
                        onchange: move |v| edit(form, |p| p.loss.mask_ratio = v) }
                }
            }
            if task == Task::Segment {
                div { class: "param-checks",
                    BoolField { label: "Overlapping masks", tip: "Merge object masks into one image mask while training (faster). Segmentation only.", value: l.overlap_mask,
                        onchange: move |v| edit(form, |p| p.loss.overlap_mask = v) }
                }
            }
        }
    }
}

#[component]
fn RunSection(form: Signal<ParameterSet>, p: UltralyticsParams) -> Element {
    let r = p.run.clone();
    let meta = if r == RunConfig::default() { "defaults".to_string() } else { format!("device {}", r.device) };

    rsx! {
        Group { id: "param-run-title", title: "Run & hardware", meta, on_reset: move |_| edit(form, |p| p.run = RunConfig::default()),
            div { class: "param-grid",
                div { class: "field param-field",
                    label { class: "field-label", r#for: "param-device",
                        "Device"
                        InfoTip { text: "auto uses a GPU when there is one. Or cpu, mps (Apple), or GPU numbers like 0 or 0,1.", label: "About the device" }
                    }
                    input {
                        id: "param-device",
                        class: "input input-narrow",
                        value: "{r.device}",
                        oninput: move |e| edit(form, |p| p.run.device = e.value().trim().to_string()),
                    }
                }
                IntField { label: "Workers", tip: "Processes loading images per GPU. Lower it if memory runs out.", value: r.workers, max: 64,
                    onchange: move |v| edit(form, |p| p.run.workers = v) }
                ChoiceField {
                    label: "Cache images",
                    tip: "Keep decoded images in RAM or on disk for faster epochs. RAM needs a lot of memory on big datasets.",
                    options: options(&Cache::ALL, Cache::label, Cache::label),
                    value: r.cache.label(),
                    onchange: move |v: String| if let Some(c) = pick(&Cache::ALL, Cache::label, &v) { edit(form, |p| p.run.cache = c) },
                }
                IntField { label: "Seed", tip: "Random seed; the same seed and settings repeat a run.", value: r.seed, max: u32::MAX,
                    onchange: move |v| edit(form, |p| p.run.seed = v) }
                OptionalIntField { label: "Checkpoint every", tip: "Also save the weights every N epochs. Off keeps only the last and best.", value: r.save_period, on_value: 10, max: 10_000,
                    onchange: move |v| edit(form, |p| p.run.save_period = v) }
                OptionalFloatField { label: "Time limit (hours)", tip: "Stop after this many hours, whatever the epoch count.", value: r.time, on_value: 1.0, min: 0.1, max: 1000.0, step: 0.5,
                    onchange: move |v| edit(form, |p| p.run.time = v) }
            }
            div { class: "param-checks",
                BoolField { label: "Mixed precision (AMP)", tip: "Faster training with less GPU memory; turn off only if training is unstable.", value: r.amp,
                    onchange: move |v| edit(form, |p| p.run.amp = v) }
                BoolField { label: "Deterministic", tip: "Repeatable results, sometimes a little slower.", value: r.deterministic,
                    onchange: move |v| edit(form, |p| p.run.deterministic = v) }
                BoolField { label: "Save plots", tip: "Save training curves and example predictions.", value: r.plots,
                    onchange: move |v| edit(form, |p| p.run.plots = v) }
                BoolField { label: "Resume", tip: "A stopped run with these parameters can be continued from its last finished epoch: select it in the Models table on Train Model.", value: r.resume,
                    onchange: move |v| edit(form, |p| p.run.resume = v) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(task: &str) -> DatasetRow {
        DatasetRow { id: 1, name: "cars".into(), task: task.into(), format: "coco".into(), images: 10, labels: 20, classes: 2, linked_to: Vec::new(), size_bytes: 0 }
    }

    #[test]
    fn prepares_task_and_family() {
        let mut set = ParameterSet::new("");
        let FrameworkParams::Ultralytics(p) = &mut set.params;
        p.model.family = Family::RtDetr;
        p.model.size = ModelSize::L;

        prepare_from(&mut set, &row("instance-segmentation"));
        let m = &ultra(&set).model;
        // RT-DETR can't segment: back to YOLO26, size kept when it has it.
        assert_eq!((m.task, m.family, m.size), (Task::Segment, Family::Yolo26, ModelSize::L));
        assert_eq!(set.name, "cars-segment");

        set.name = "mine".into();
        prepare_from(&mut set, &row("classification"));
        assert_eq!(ultra(&set).model.task, Task::Classify);
        assert_eq!(set.name, "mine");
        assert_eq!(dataset_task(&row("object-detection")), Task::Detect);
        assert!(format_note("yolov8").starts_with("YOLO"));
    }
}
