//! Training → Tune Hyperparameters: search training settings with Optuna on
//! the dataset chosen in the Datasets table, starting from a parameter set
//! (chosen in the Parameters table, or a new one made here). Each trial
//! trains briefly; the best trial's values are saved into that parameter set.
//! Runs go to `training/tuning/<name>/`; the console shows live.

use dioxus::prelude::*;

use crate::api::datasets::list_datasets;
use crate::components::{IconName, TitleIcon};
use crate::api::error_message;
use crate::api::jobs::{ItemStatus, JobKind};
use crate::api::parameters::{list_parameters, Batch, Optimizer, ParameterSet, Task, UltralyticsParams};
use crate::api::pyenv::python_env_overview;
use crate::api::tune::{
    hyper_value, hyperparameter, start_tune, tune_names, tune_name_taken_message, tune_preview, tune_record, DefaultSpace, TunePlan, TuneParam,
    TuneSpace, TuneValue, HYPERPARAMETERS, MAX_TRIAL_EPOCHS,
};
use crate::components::table::{Column, Sort, SortHeader};
use crate::components::{
    open_python_settings, set_if_newer, use_job, use_panel_table, InfoTip, JobProgress, NoWorkspace, PanelTable, Slider,
};
use crate::pages::parameters::fields::IntField;
use crate::pages::parameters::{edit, ultra, ModelSection};
use crate::pages::train::{model_stem, suggest_name, TrainConsole};
use crate::routes::Route;
use crate::state::{
    use_datasets_version, use_parameters_version, use_primary_dataset, use_primary_parameters, use_selected_parameters, use_workspace,
    use_workspace_checked,
};
use crate::util::{format_count, plural};

#[component]
pub fn TuneHyperparameters() -> Element {
    let workspace = use_workspace();
    let checked = use_workspace_checked();

    rsx! {
        document::Title { "Tune Hyperparameters · Modeloid" }
        div { class: "page page-wide",
            p { class: "page-eyebrow", "Training" }
            h1 { "Tune Hyperparameters" }
            p { class: "page-lead",
                "Search for the training settings that work best on a dataset. Each trial trains briefly with other values; the best ones are saved into a parameter set."
            }
            if workspace.read().is_some() {
                TuneForm {}
            } else if checked() {
                NoWorkspace { text: "Tuning works on a workspace's datasets." }
            }
        }
    }
}

/// Where the best values go.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    /// The parameter set chosen in the Parameters table (updated).
    Existing,
    /// A new set made on this page.
    New,
}

#[component]
fn TuneForm() -> Element {
    let dataset_id = use_primary_dataset();
    let params_name = use_primary_parameters();
    let datasets_version = use_datasets_version();
    let parameters_version = use_parameters_version();
    let mut panel = use_panel_table();
    let datasets = use_resource(move || async move {
        let _ = datasets_version();
        list_datasets().await
    });
    let params = use_resource(move || async move {
        let _ = parameters_version();
        list_parameters().await
    });
    let dataset = dataset_id().and_then(|id| datasets.read().as_ref()?.as_ref().ok()?.iter().find(|r| r.id == id).cloned());
    let existing: Option<ParameterSet> =
        params_name().and_then(|name| params.read().as_ref()?.as_ref().ok()?.sets.iter().find(|s| s.name == name).cloned());

    // A set chosen in the Parameters table is updated; without one, a new set is made here.
    let target = if existing.is_some() { Target::Existing } else { Target::New };
    let new_set = use_signal(|| ParameterSet::new(""));
    let mut trials = use_signal(|| 20u32);
    let mut epochs = use_signal(|| 10u32);
    let mut data_percent = use_signal(|| 100u32);
    let space = use_signal(|| HYPERPARAMETERS.iter().map(TuneParam::default_for).collect::<Vec<_>>());

    let (job, poll_error) = use_job(JobKind::Tune);
    let job_run = job.read().as_ref().and_then(|j| j.items.first()).map(|i| i.label.clone());
    let running = job.read().as_ref().is_some_and(|j| !j.finished);
    let finished_version = use_memo(move || job.read().as_ref().filter(|j| j.finished).map(|j| j.version));

    let base: Option<ParameterSet> = match target {
        Target::Existing => existing.clone(),
        Target::New => Some(new_set()),
    };
    let task = base.as_ref().map(|b| ultra(b).model.task).unwrap_or_default();

    // Run name: "<dataset>-<set>-tune", the next free one, until the user types one.
    let names = use_resource(move || async move {
        let _ = finished_version();
        tune_names().await
    });
    let mut taken: Vec<String> = names.read().as_ref().and_then(|r| r.as_ref().ok()).cloned().unwrap_or_default();
    taken.extend(job_run.clone());
    let mut typed_name = use_signal(|| None::<String>);
    let suggested = match (&dataset, &base) {
        (Some(ds), Some(b)) => {
            let what = if b.name.trim().is_empty() { model_stem(b) } else { b.name.trim().to_string() };
            suggest_name(&ds.name, &format!("{what}-tune"), &taken)
        }
        _ => String::new(),
    };
    let name = typed_name().unwrap_or(suggested);
    let own_run = job_run.as_deref() == Some(name.as_str());

    let plan: Option<TunePlan> = match (&dataset, &base) {
        (Some(ds), Some(b)) => Some(TunePlan {
            dataset: ds.id,
            parameters: (target == Target::Existing).then(|| b.name.clone()),
            new_set: (target == Target::New).then(|| b.clone()),
            name: name.clone(),
            trials: trials(),
            epochs: epochs(),
            fraction: f64::from(data_percent()) / 100.0,
            space: space(),
        }),
        _ => None,
    };
    let plan_for_start = plan.clone();
    let preview = use_resource(use_reactive((&plan,), move |(plan,)| async move {
        let _ = finished_version(); // a finished run takes its name
        match plan {
            Some(plan) => Some(tune_preview(plan).await),
            None => None,
        }
    }));
    let libraries_ready = use_resource(move || async move {
        let _ = finished_version();
        python_env_overview().await.ok().map(|ov| ov.status.usable())
    });
    let libraries_missing = matches!(&*libraries_ready.read(), Some(Some(false)) | Some(None));

    let mut start_error = use_signal(|| None::<String>);
    let can_run = !running && !libraries_missing && matches!(&*preview.read(), Some(Some(Ok(p))) if p.problems.is_empty() && (!p.name_taken || own_run) && !own_run);
    let start = move |_| {
        let plan = plan_for_start.clone();
        async move {
            let Some(plan) = plan else { return };
            start_error.set(None);
            match start_tune(plan).await {
                Ok(started) => {
                    set_if_newer(job, started);
                    typed_name.set(None);
                }
                Err(e) => start_error.set(Some(error_message(&e))),
            }
        }
    };

    let live_line = job
        .read()
        .as_ref()
        .filter(|j| !j.finished)
        .and_then(|j| j.items.first())
        .filter(|i| i.status == ItemStatus::Running)
        .and_then(|i| i.message.clone());
    let job_version = job.read().as_ref().map(|j| j.version).unwrap_or(0);
    let current = base.as_ref().map(|b| ultra(b).clone());

    rsx! {
        div { class: "section-stack",
            section { class: "section-card", aria_labelledby: "tune-inputs-title",
                h2 { id: "tune-inputs-title", class: "section-card-title", TitleIcon { name: IconName::Database } "Inputs" }
                div { class: "train-inputs",
                    div { class: "train-input",
                        span { class: "field-label", "Dataset" }
                        match &dataset {
                            Some(ds) => rsx! {
                                div { class: "train-input-chosen",
                                    span { class: "train-input-name", "{ds.name}" }
                                    span { class: "train-input-detail", "#{ds.id} · {ds.task} · {ds.format} · {format_count(ds.images)} images · {ds.classes} classes" }
                                }
                            },
                            None => rsx! { span { class: "train-input-detail", "None chosen" } },
                        }
                        button { class: "button button-secondary button-sm", r#type: "button", onclick: move |_| panel.set(PanelTable::Datasets),
                            "Choose in the Datasets table"
                        }
                    }
                    div { class: "train-input",
                        span { class: "field-label",
                            "Parameter set "
                            InfoTip {
                                label: "About the parameter set",
                                text: "Tuning starts from the chosen set's settings (model, image size, device…) and saves the best values into it. With no set chosen, you make a new one here; it's saved when tuning ends.",
                            }
                        }
                        match &existing {
                            Some(set) => rsx! {
                                div { class: "train-input-chosen",
                                    span { class: "train-input-name", "{set.name}" }
                                    span { class: "train-input-detail", "{set.summary()} · its best values are replaced" }
                                }
                            },
                            None => rsx! {
                                span { class: "train-input-detail", "None chosen: a new set is made below and saved with the best values." }
                            },
                        }
                        button { class: "button button-secondary button-sm", r#type: "button", onclick: move |_| panel.set(PanelTable::Parameters),
                            "Choose in the Parameters table"
                        }
                    }
                }
            }

            if target == Target::New {
                NewSetCard { form: new_set }
            }

            section { class: "section-card", aria_labelledby: "tune-run-title",
                h2 { id: "tune-run-title", class: "section-card-title", TitleIcon { name: IconName::Target } "Tuning" }
                div { class: "param-grid tune-run-grid",
                    div { class: "field param-field",
                        span { class: "field-label",
                            "Trials "
                            InfoTip { label: "About trials", text: "How many combinations Optuna tries. Each one learns from the trials before it; more trials find better values but take longer." }
                        }
                        Slider { value: trials(), label: "Trials", min: 1, max: 100, onchange: move |n| trials.set(n) }
                    }
                    div { class: "field param-field",
                        span { class: "field-label",
                            "Epochs per trial "
                            InfoTip { label: "About epochs per trial", text: "Each trial trains this long; enough to compare settings, far less than a full training." }
                        }
                        Slider { value: epochs(), label: "Epochs per trial", min: 1, max: 100.min(MAX_TRIAL_EPOCHS), onchange: move |n| epochs.set(n) }
                    }
                    div { class: "field param-field",
                        span { class: "field-label",
                            "Data per trial "
                            InfoTip { label: "About data per trial", text: "Share of the training images each trial uses. Less is faster, but the scores are less reliable." }
                        }
                        Slider { value: data_percent(), label: "Data per trial in percent", min: 10, max: 100, unit: "%", onchange: move |n| data_percent.set(n) }
                    }
                }
                div { class: "field",
                    label { class: "field-label", r#for: "tune-name",
                        "Run name "
                        InfoTip { label: "About the run name", text: "Saved in training/tuning/<run name>/: the trials, their scores and the console output." }
                    }
                    input {
                        id: "tune-name",
                        class: "input",
                        value: "{name}",
                        placeholder: "e.g. cars-baseline-tune",
                        oninput: move |e| typed_name.set(Some(e.value())),
                    }
                }
            }

            SearchSpace { space, task, current }

            section { class: "section-card", aria_labelledby: "tune-start-title",
                h2 { id: "tune-start-title", class: "section-card-title", TitleIcon { name: IconName::Play } "Run" }
                if libraries_missing {
                    div { class: "notice notice-warning python-notice", role: "status",
                        span { "The training libraries aren't installed yet." }
                        button { class: "button button-sm", r#type: "button", onclick: move |_| open_python_settings(), "Open Python settings" }
                    }
                }
                match &*preview.read() {
                    Some(Some(Ok(p))) => {
                        let mut problems = p.problems.clone();
                        if p.name_taken && !own_run {
                            problems.insert(0, tune_name_taken_message(&name));
                        }
                        rsx! {
                            p { class: "field-hint", "Trains {p.model} on {p.device}." }
                            if !problems.is_empty() {
                                ul { class: "param-problems", aria_label: "Fix these first",
                                    for problem in problems.iter() {
                                        li { key: "{problem}", class: "field-error", "{problem}" }
                                    }
                                }
                            }
                            for note in p.notes.iter() {
                                p { key: "{note}", class: "field-hint", "{note}" }
                            }
                        }
                    }
                    Some(Some(Err(e))) => rsx! { p { class: "form-error", role: "alert", "{error_message(e)}" } },
                    Some(None) => rsx! { p { class: "field-hint", "Choose a dataset and a parameter set." } },
                    None => rsx! {},
                }
                if let Some(message) = start_error() {
                    p { class: "form-error", role: "alert", "{message}" }
                }
                if let Some(message) = poll_error() {
                    p { class: "form-error", role: "alert", "{message}" }
                }
                div { class: "form-actions",
                    button { class: "button", r#type: "button", disabled: !can_run, onclick: start,
                        if running { "Tuning…" } else { "Tune" }
                    }
                }
            }

            if let Some(job) = job() {
                JobProgress { job, title: "Tuning", noun: "run", verb: "tuned", output_label: "Result" }
            }
            if let Some(run) = job_run {
                // Keyed by the run, so a new run starts with fresh state.
                div { key: "{run}", class: "section-stack",
                    TuneResult { run: run.clone(), version: job_version }
                    TrainConsole { run, version: job_version, live: live_line, tuning: true }
                }
            }
        }
    }
}

/// The new parameter set's name, model and main training settings.
#[component]
fn NewSetCard(form: Signal<ParameterSet>) -> Element {
    let mut form = form;
    let set = form();
    let p = ultra(&set).clone();
    let t = p.training.clone();

    rsx! {
        section { class: "section-card", aria_labelledby: "tune-new-title",
            h2 { id: "tune-new-title", class: "section-card-title",
                TitleIcon { name: IconName::Plus }
                "New parameter set "
                InfoTip { label: "About the new set", text: "Like Add Parameter on the Parameters page. Settings not shown here keep their defaults; edit them there later." }
            }
            div { class: "param-grid",
                div { class: "field param-field",
                    label { class: "field-label", r#for: "tune-set-name", "Name" }
                    input {
                        id: "tune-set-name",
                        class: "input",
                        value: "{set.name}",
                        placeholder: "e.g. yolo11n-tuned",
                        oninput: move |e| form.write().name = e.value(),
                    }
                }
                div { class: "field param-field",
                    label { class: "field-label", r#for: "tune-set-description", "Description (optional)" }
                    input {
                        id: "tune-set-description",
                        class: "input",
                        value: "{set.description.clone().unwrap_or_default()}",
                        oninput: move |e| {
                            let text = e.value();
                            form.write().description = if text.trim().is_empty() { None } else { Some(text) };
                        },
                    }
                }
            }
        }
        ModelSection { form, p: p.clone() }
        section { class: "section-card", aria_labelledby: "tune-new-training-title",
            h2 { id: "tune-new-training-title", class: "section-card-title", TitleIcon { name: IconName::Sliders } "Training" }
            div { class: "param-grid",
                IntField { label: "Epochs", tip: "Passes over the training images when this set trains a model (trials use Epochs per trial).", value: t.epochs, max: 10_000,
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
                div { class: "field param-field",
                    label { class: "field-label", r#for: "tune-device",
                        "Device"
                        InfoTip { text: "auto uses a GPU when there is one. Or cpu, mps (Apple), or GPU numbers like 0 or 0,1.", label: "About the device" }
                    }
                    input {
                        id: "tune-device",
                        class: "input",
                        value: "{p.run.device}",
                        oninput: move |e| edit(form, |p| p.run.device = e.value().trim().to_string()),
                    }
                }
            }
        }
    }
}

const SPACE_COLUMNS: [Column; 4] = [Column::id(), Column::text("Hyperparameter"), Column::text("Group"), Column::text("Current")];

/// Every hyperparameter that applies to `task`: whether it's searched (the
/// checkbox), its value in the parameter set, and what is searched — a range
/// or candidate values the user adds. A collapsible table.
#[component]
fn SearchSpace(space: Signal<Vec<TuneParam>>, task: Task, current: Option<UltralyticsParams>) -> Element {
    let mut space = space;
    let sort = use_signal(Sort::by_id);
    // (row number, position in `space`) for the rows that apply to the task.
    let mut rows: Vec<(usize, usize)> = HYPERPARAMETERS
        .iter()
        .enumerate()
        .filter(|(_, h)| h.applies.to(task))
        .filter_map(|(n, h)| space.read().iter().position(|p| p.key == h.key).map(|i| (n + 1, i)))
        .collect();
    let s = sort();
    rows.sort_by(|a, b| {
        let (ha, hb) = (hyperparameter(&space.read()[a.1].key), hyperparameter(&space.read()[b.1].key));
        let order = match s.column {
            1 => ha.map(|h| h.label).cmp(&hb.map(|h| h.label)),
            2 => ha.map(|h| h.group).cmp(&hb.map(|h| h.group)),
            3 => {
                let value = |i: usize| current.as_ref().and_then(|c| hyper_value(c, &space.read()[i].key)).map(|v| v.label()).unwrap_or_default();
                value(a.1).cmp(&value(b.1))
            }
            _ => a.0.cmp(&b.0),
        };
        s.directed(order).then(a.0.cmp(&b.0))
    });
    let total = rows.len();
    let searched = rows.iter().filter(|(_, i)| space.read()[*i].enabled).count();
    let listed: Vec<usize> = rows.iter().map(|(_, i)| *i).collect();
    let listed_for_none = listed.clone();

    rsx! {
        details { class: "section-card param-group tune-space", open: true,
            summary { class: "param-group-summary",
                h2 { class: "section-card-title",
                    TitleIcon { name: IconName::Sliders }
                    "Search space "
                    InfoTip {
                        label: "About the search space",
                        text: "Checked rows are searched: a range (log scale spreads tries evenly over orders of magnitude) or only the candidates you add. Unchecked ones keep the parameter set's value.",
                    }
                }
                span { class: "section-card-meta", "{searched} of {plural(total, \"hyperparameter\")} searched" }
            }
            div { class: "panel-selection tune-space-bar", role: "group", aria_label: "Hyperparameter selection",
                button { class: "button button-ghost button-sm", r#type: "button",
                    onclick: move |_| { let mut s = space.write(); for &i in &listed { s[i].enabled = true; } },
                    "Select all"
                }
                button { class: "button button-ghost button-sm", r#type: "button",
                    onclick: move |_| { let mut s = space.write(); for &i in &listed_for_none { s[i].enabled = false; } },
                    "None"
                }
                span { class: "section-card-meta", "{searched} selected" }
                button {
                    class: "icon-button icon-button-sm",
                    r#type: "button",
                    aria_label: "Reset the search space",
                    title: "Reset: the default ranges and choices",
                    onclick: move |_| space.set(HYPERPARAMETERS.iter().map(TuneParam::default_for).collect()),
                    crate::components::Icon { name: crate::components::IconName::Refresh, size: 14 }
                }
            }
            div { class: "table-scroll tune-space-scroll",
                table { class: "data-table tune-space-table",
                    thead {
                        tr {
                            th { class: "check-col", scope: "col", span { class: "visually-hidden", "Searched" } }
                            for (i, def) in SPACE_COLUMNS.iter().enumerate() {
                                SortHeader { key: "{i}", column: i, def: *def, sort }
                            }
                            th { scope: "col", "Search" }
                        }
                    }
                    tbody {
                        for (number, i) in rows.iter().copied() {
                            {
                                let param = space.read()[i].clone();
                                let h = hyperparameter(&param.key).copied();
                                let value = current.as_ref().and_then(|c| hyper_value(c, &param.key)).map(|v| v.label()).unwrap_or_else(|| "–".into());
                                rsx! {
                                    tr { key: "{param.key}", class: if param.enabled { "is-checked" } else { "" },
                                        td { class: "check-col",
                                            input {
                                                r#type: "checkbox",
                                                aria_label: "Search {param.key}",
                                                checked: param.enabled,
                                                onchange: move |e| space.write()[i].enabled = e.checked(),
                                            }
                                        }
                                        td { class: "num muted", "{number}" }
                                        th { class: "name", scope: "row",
                                            span { class: "tune-param-name",
                                                "{h.map(|h| h.label).unwrap_or_default()} "
                                                InfoTip { label: "About {param.key}", text: "{h.map(|h| h.tip).unwrap_or_default()} Ultralytics: {param.key}." }
                                            }
                                        }
                                        td { class: "muted", "{h.map(|h| h.group).unwrap_or_default()}" }
                                        td { class: "num", "{value}" }
                                        td {
                                            SpaceEditor { param: param.clone(), on_change: move |p: TuneParam| space.write()[i] = p }
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

/// What one row searches: a range (low, high, log scale) or candidates.
#[component]
fn SpaceEditor(param: TuneParam, on_change: EventHandler<TuneParam>) -> Element {
    let Some(h) = hyperparameter(&param.key).copied() else { return rsx! {} };
    let names_only = matches!(h.default, DefaultSpace::Texts(_));
    let mut adding = use_signal(String::new);
    let disabled = !param.enabled;
    let is_range = matches!(param.space, TuneSpace::Range { .. });

    let switch = {
        let param = param.clone();
        move |e: FormEvent| {
            let mut next = param.clone();
            next.space = match (e.value().as_str(), &param.space) {
                ("range", TuneSpace::Candidates(values)) => {
                    let numbers: Vec<f64> = values.iter().filter_map(TuneValue::number).collect();
                    match (numbers.iter().copied().reduce(f64::min), numbers.iter().copied().reduce(f64::max)) {
                        (Some(lo), Some(hi)) if lo < hi => TuneSpace::Range { low: lo, high: hi, log: false },
                        _ => TuneParam::default_for(&h).space,
                    }
                    
                }
                ("candidates", TuneSpace::Range { low, high, .. }) => {
                    let (low, high) = if h.integer { (low.round(), high.round()) } else { (*low, *high) };
                    TuneSpace::Candidates(vec![TuneValue::Number(low), TuneValue::Number(high)])
                }
                _ => return,
            };
            // A range over whole-number candidates' defaults (batch, image size) keeps them whole.
            if let TuneSpace::Range { low, high, .. } = &mut next.space {
                if h.integer {
                    *low = low.round();
                    *high = high.round();
                }
            }
            on_change.call(next);
        }
    };

    let body = match param.space.clone() {
        TuneSpace::Range { low, high, log } => {
            let (p1, p2, p3) = (param.clone(), param.clone(), param.clone());
            rsx! {
                span { class: "tune-range",
                    input {
                        class: "input input-narrow",
                        r#type: "number",
                        step: "any",
                        aria_label: "Lowest {param.key}",
                        disabled,
                        value: "{low}",
                        onchange: move |e| {
                            if let Ok(v) = e.value().trim().parse::<f64>() {
                                let mut next = p1.clone();
                                next.space = TuneSpace::Range { low: v, high, log };
                                on_change.call(next);
                            }
                        },
                    }
                    span { class: "muted", "to" }
                    input {
                        class: "input input-narrow",
                        r#type: "number",
                        step: "any",
                        aria_label: "Highest {param.key}",
                        disabled,
                        value: "{high}",
                        onchange: move |e| {
                            if let Ok(v) = e.value().trim().parse::<f64>() {
                                let mut next = p2.clone();
                                next.space = TuneSpace::Range { low, high: v, log };
                                on_change.call(next);
                            }
                        },
                    }
                    label { class: "check",
                        input {
                            r#type: "checkbox",
                            disabled,
                            checked: log,
                            onchange: move |e| {
                                let mut next = p3.clone();
                                next.space = TuneSpace::Range { low, high, log: e.checked() };
                                on_change.call(next);
                            },
                        }
                        span { "log" }
                    }
                }
            }
        }
        TuneSpace::Candidates(values) => {
            let options: Vec<String> = match h.default {
                DefaultSpace::Texts(_) => Optimizer::ALL
                    .iter()
                    .map(|o| o.id().to_string())
                    .filter(|id| !values.iter().any(|v| matches!(v, TuneValue::Text(t) if t == id)))
                    .collect(),
                _ => Vec::new(),
            };
            let add = {
                let param = param.clone();
                let values = values.clone();
                move || {
                    let text = adding.peek().trim().to_string();
                    if text.is_empty() {
                        return;
                    }
                    let value = if names_only {
                        TuneValue::Text(text)
                    } else {
                        match text.parse::<f64>() {
                            Ok(n) => TuneValue::Number(n),
                            Err(_) => return,
                        }
                    };
                    if values.contains(&value) {
                        adding.set(String::new());
                        return;
                    }
                    let mut next = param.clone();
                    let mut list = values.clone();
                    list.push(value);
                    if !names_only {
                        list.sort_by(|a, b| a.number().unwrap_or(0.0).total_cmp(&b.number().unwrap_or(0.0)));
                    }
                    next.space = TuneSpace::Candidates(list);
                    adding.set(String::new());
                    on_change.call(next);
                }
            };
            let mut add_on_enter = add.clone();
            let mut add_on_click = add;
            rsx! {
                span { class: "tune-candidates",
                    for (n, value) in values.iter().enumerate() {
                        {
                            let param = param.clone();
                            let values = values.clone();
                            rsx! {
                                span { key: "{n}", class: "tag tune-candidate",
                                    "{value.label()}"
                                    button {
                                        class: "tune-candidate-remove",
                                        r#type: "button",
                                        aria_label: "Remove {value.label()}",
                                        disabled,
                                        onclick: move |_| {
                                            let mut next = param.clone();
                                            let mut list = values.clone();
                                            list.remove(n);
                                            next.space = TuneSpace::Candidates(list);
                                            on_change.call(next);
                                        },
                                        "×"
                                    }
                                }
                            }
                        }
                    }
                    if names_only {
                        select {
                            class: "input select input-narrow",
                            aria_label: "Candidate to add",
                            disabled: disabled || options.is_empty(),
                            value: "{adding}",
                            onchange: move |e| adding.set(e.value()),
                            option { value: "", "Add…" }
                            for id in options.iter() {
                                option { key: "{id}", value: "{id}", "{id}" }
                            }
                        }
                    } else {
                        input {
                            class: "input input-narrow",
                            r#type: "number",
                            step: "any",
                            placeholder: "value",
                            aria_label: "Candidate to add",
                            disabled,
                            value: "{adding}",
                            oninput: move |e| adding.set(e.value()),
                            onkeydown: move |e| {
                                if e.key() == Key::Enter {
                                    e.prevent_default();
                                    add_on_enter();
                                }
                            },
                        }
                    }
                    button { class: "button button-secondary button-sm", r#type: "button", disabled: disabled || adding.read().trim().is_empty(),
                        onclick: move |_| add_on_click(), "Add" }
                }
            }
        }
    };

    rsx! {
        span { class: "tune-search",
            if !names_only {
                select {
                    class: "input select tune-mode",
                    aria_label: "How {param.key} is searched",
                    disabled,
                    value: if is_range { "range" } else { "candidates" },
                    onchange: switch,
                    option { value: "range", selected: is_range, "Range" }
                    option { value: "candidates", selected: !is_range, "Candidates" }
                }
            }
            {body}
        }
        if let Some(problem) = param.problem() {
            span { class: "field-error tune-row-problem", "{problem}" }
        }
    }
}

/// The trials of run `run` so far, the best one and where it was saved.
#[component]
fn TuneResult(run: String, version: u64) -> Element {
    let run_name = run;
    let record = use_resource(use_reactive((&version,), move |_| {
        let name = run_name.clone();
        async move { tune_record(name).await }
    }));
    let mut selected = use_selected_parameters();
    let mut parameters_version = use_parameters_version();
    let sort = use_signal(|| Sort { column: 1, ascending: false });
    let columns = [Column::id(), Column::number("Score"), Column::text("Values")];

    let Some(Ok(r)) = record.read().clone() else { return rsx! {} };
    let best = r.best.as_ref().map(|b| b.number);
    let mut trials = r.results.clone();
    let s = sort();
    trials.sort_by(|a, b| {
        let order = match s.column {
            1 => a.value.unwrap_or(f64::MIN).total_cmp(&b.value.unwrap_or(f64::MIN)),
            2 => values_text(a).cmp(&values_text(b)),
            _ => a.number.cmp(&b.number),
        };
        s.directed(order).then(a.number.cmp(&b.number))
    });

    rsx! {
        section { class: "section-card", aria_labelledby: "tune-result-title",
            div { class: "section-card-header",
                h2 { id: "tune-result-title", class: "section-card-title",
                    TitleIcon { name: IconName::Chart }
                    "Trials "
                    InfoTip { label: "About the score", text: "Ultralytics' fitness on the validation images: mostly mAP50-95 for boxes and masks, accuracy for classification. Higher is better." }
                }
                span { class: "section-card-meta", "{r.results.len()} of {plural(r.trials as usize, \"trial\")} · {plural(r.epochs as usize, \"epoch\")} each" }
            }
            if let Some(b) = &r.best {
                div { class: "notice", role: "status",
                    p {
                        "Best so far: trial {b.number}, score {score(b.value)}. "
                        match &r.saved {
                            Some(set) => rsx! { "Saved into parameter set " strong { "{set}" } "." },
                            None => rsx! { "It's saved into " strong { "{r.parameters}" } " when tuning ends." },
                        }
                    }
                    if let Some(set) = r.saved.clone() {
                        div { class: "notice-actions",
                            button {
                                class: "button button-secondary button-sm",
                                r#type: "button",
                                onclick: move |_| {
                                    *parameters_version.write() += 1;
                                    selected.set(vec![set.clone()]);
                                    navigator().push(Route::Tool { section: "training".into(), tool: "parameters".into() });
                                },
                                "Open in Parameters"
                            }
                        }
                    }
                }
            }
            if let Some(error) = &r.error {
                p { class: "form-error", role: "alert", "{error}" }
            }
            if trials.is_empty() {
                p { class: "field-hint", "No trial has finished yet." }
            } else {
                div { class: "table-scroll",
                    table { class: "data-table tune-trials",
                        thead {
                            tr {
                                for (i, def) in columns.iter().enumerate() {
                                    SortHeader { key: "{i}", column: i, def: *def, sort }
                                }
                            }
                        }
                        tbody {
                            for t in trials.iter() {
                                tr { key: "{t.number}", class: if best == Some(t.number) { "is-checked" } else { "" },
                                    td { class: "num muted", "{t.number}" }
                                    td { class: "num",
                                        match (t.failed, t.value) {
                                            (true, _) => rsx! { span { class: "tag tag-missing", title: "{t.error.clone().unwrap_or_default()}", "failed" } },
                                            (false, Some(v)) => rsx! { "{v:.4}" },
                                            (false, None) => rsx! { "–" },
                                        }
                                        if best == Some(t.number) {
                                            span { class: "tag tag-ok tune-best", "best" }
                                        }
                                    }
                                    td { class: "tune-values", title: "{values_text(t)}", "{values_text(t)}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn score(value: Option<f64>) -> String {
    value.map(|v| format!("{v:.4}")).unwrap_or_else(|| "–".into())
}

/// "lr0 0.0044 · optimizer SGD".
fn values_text(t: &crate::api::tune::TrialResult) -> String {
    t.params.iter().map(|(k, v)| format!("{k} {}", v.label())).collect::<Vec<_>>().join(" · ")
}
