//! Training → Train Model: train a model on the dataset chosen in the
//! Datasets table with the parameter set chosen in the Parameters table.
//! The run goes to `training/trains/<model name>/`; Ultralytics' console
//! output shows live. A stopped run selected in the Models table can be
//! continued from its last epoch instead.

use dioxus::prelude::*;

use crate::api::datasets::list_datasets;
use crate::components::{IconName, TitleIcon};
use crate::api::error_message;
use crate::api::jobs::{ItemStatus, Job, JobKind};
use crate::api::parameters::{list_parameters, Batch, FrameworkParams, ParameterSet};
use crate::api::pyenv::python_env_overview;
use crate::api::train::{list_models, name_taken_message, resume_preview, resume_training, start_training, train_log, train_preview, TrainRecord, TrainStatus};
use crate::api::tune::tune_log;
use crate::components::{set_if_newer, use_job, use_panel_table, InfoTip, JobProgress, LibrariesMissing, NoWorkspace, PanelTable};
use crate::state::{
    use_datasets_version, use_models_version, use_parameters_version, use_primary_dataset, use_primary_parameters, use_selected_models, use_workspace,
    use_workspace_checked,
};
use crate::util::format_count;

/// Lines of console output kept on the page (the full log is in console.log).
const CONSOLE_LINES: usize = 3000;

#[component]
pub fn TrainModel() -> Element {
    let workspace = use_workspace();
    let checked = use_workspace_checked();

    rsx! {
        document::Title { "Train Model · Modeloid" }
        div { class: "page page-wide",
            p { class: "page-eyebrow", "Training" }
            h1 { "Train Model" }
            p { class: "page-lead",
                "Train a model on a dataset from the Datasets table with a parameter set from the Parameters table. Results go to training/trains/<model name>/."
            }
            if workspace.read().is_some() {
                TrainForm {}
            } else if checked() {
                NoWorkspace { text: "Training works on a workspace's datasets." }
            }
        }
    }
}

/// "YOLO11n · detect · 100 epochs · 640 px · batch 16".
fn describe(set: &ParameterSet) -> String {
    let FrameworkParams::Ultralytics(p) = &set.params;
    let batch = match p.training.batch {
        Batch::Auto => "auto batch".to_string(),
        Batch::Size(n) => format!("batch {n}"),
    };
    format!("{} · {} epochs · {} px · {batch}", set.summary(), p.training.epochs, p.training.imgsz)
}

/// The model without its file extension or folder, e.g. "yolo11n-seg".
pub(crate) fn model_stem(set: &ParameterSet) -> String {
    let FrameworkParams::Ultralytics(p) = &set.params;
    let file = p.model_name();
    let file = file.rsplit(['/', '\\']).next().unwrap_or_default().to_string();
    match file.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => file,
    }
}

/// `<dataset>-<model>`, numbered ("-2", "-3", …) past names in `taken`.
pub(crate) fn suggest_name(dataset: &str, model: &str, taken: &[String]) -> String {
    let base = format!("{dataset}-{model}");
    let mut name = base.clone();
    let mut n = 2;
    while taken.contains(&name) {
        name = format!("{base}-{n}");
        n += 1;
    }
    name
}

#[component]
fn TrainForm() -> Element {
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
    let set: Option<ParameterSet> =
        params_name().and_then(|name| params.read().as_ref()?.as_ref().ok()?.sets.iter().find(|s| s.name == name).cloned());

    let (job, poll_error) = use_job(JobKind::Train);
    // The run the job is (or was) training, named after its folder.
    let job_run = job.read().as_ref().and_then(|j| j.items.first()).map(|i| i.label.clone());

    // Model name: "<dataset>-<model>", the next free one, until the user types one.
    let models_version = use_models_version();
    // A run that starts or ends changes the list (and its status) at once.
    let train_state = use_memo(move || job.read().as_ref().map(|j| (j.finished, j.items.first().map(|i| i.label.clone()))));
    let models = use_resource(move || async move {
        let _ = (models_version(), train_state());
        list_models().await
    });
    let mut taken: Vec<String> = models.read().as_ref().and_then(|r| r.as_ref().ok()).map(|l| l.models.iter().map(|m| m.name.clone()).collect()).unwrap_or_default();
    taken.extend(job_run.clone());
    let mut typed_name = use_signal(|| None::<String>);
    let suggested = match (&set, &dataset) {
        (Some(set), Some(ds)) => suggest_name(&ds.name, &model_stem(set), &taken),
        _ => String::new(),
    };
    let name = typed_name().unwrap_or(suggested);
    let (name_for_preview, name_for_start) = (name.clone(), name.clone());
    let (dataset_ready, params_ready) = (dataset.as_ref().map(|d| d.id), set.as_ref().map(|s| s.name.clone()));
    // Its own run's folder isn't "taken" while that run shows below.
    let own_run = job_run.as_deref() == Some(name.as_str());

    let running = job.read().as_ref().is_some_and(|j| !j.finished);
    // The model last selected in the Models table, when it's a stopped run (not the one training now).
    let selected_models = use_selected_models();
    let stopped: Option<TrainRecord> = selected_models.read().last().and_then(|name| {
        let list = models.read();
        let record = list.as_ref()?.as_ref().ok()?.models.iter().find(|m| &m.name == name)?.clone();
        (record.status == TrainStatus::Stopped && !(running && job_run.as_deref() == Some(name.as_str()))).then_some(record)
    });
    let finished_version = use_memo(move || job.read().as_ref().filter(|j| j.finished).map(|j| j.version));

    let preview = use_resource(use_reactive((&dataset_ready, &params_ready, &name_for_preview), move |(ds, ps, name)| async move {
        let _ = finished_version(); // a finished run takes its name
        match (ds, ps) {
            (Some(ds), Some(ps)) => Some(train_preview(ds, ps, name).await),
            _ => None,
        }
    }));
    let mut libraries_missing = use_signal(|| false);
    let mut start_error = use_signal(|| None::<String>);

    let can_run = !running
        && matches!(&*preview.read(), Some(Some(Ok(p))) if p.can_run());
    let start = move |_| {
        let (ds, ps, name) = (dataset_ready, params_ready.clone(), name_for_start.clone());
        async move {
            let (Some(ds), Some(ps)) = (ds, ps) else { return };
            start_error.set(None);
            // Libraries first: the reminder leads to Settings.
            if !matches!(python_env_overview().await, Ok(ov) if ov.status.usable()) {
                libraries_missing.set(true);
                return;
            }
            libraries_missing.set(false);
            match start_training(ds, ps, name).await {
                Ok(started) => {
                    set_if_newer(job, started);
                    // The next run gets the next free name.
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

    rsx! {
        div { class: "section-stack",
            section { class: "section-card", aria_labelledby: "train-inputs-title",
                h2 { id: "train-inputs-title", class: "section-card-title", TitleIcon { name: IconName::Database } "Inputs" }
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
                        button {
                            class: "button button-secondary button-sm",
                            r#type: "button",
                            onclick: move |_| panel.set(PanelTable::Datasets),
                            "Choose in the Datasets table"
                        }
                    }
                    div { class: "train-input",
                        span { class: "field-label", "Parameters" }
                        match &set {
                            Some(set) => rsx! {
                                div { class: "train-input-chosen",
                                    span { class: "train-input-name", "{set.name}" }
                                    span { class: "train-input-detail", "{describe(set)}" }
                                }
                            },
                            None => rsx! { span { class: "train-input-detail", "None chosen" } },
                        }
                        button {
                            class: "button button-secondary button-sm",
                            r#type: "button",
                            onclick: move |_| panel.set(PanelTable::Parameters),
                            "Choose in the Parameters table"
                        }
                    }
                }
            }

            if let Some(run) = stopped {
                ContinueRun { key: "{run.name}-{run.epochs_done}", run, running, job }
            }

            section { class: "section-card", aria_labelledby: "train-output-title",
                h2 { id: "train-output-title", class: "section-card-title", TitleIcon { name: IconName::Save } "Output" }
                div { class: "field",
                    label { class: "field-label", r#for: "train-name",
                        "Model name "
                        InfoTip { label: "About the model name", text: "Saved in training/trains/<model name>/: weights, plots and results in its runs folder." }
                    }
                    input {
                        id: "train-name",
                        class: "input",
                        value: "{name}",
                        placeholder: "e.g. yolo11n-cars",
                        oninput: move |e| typed_name.set(Some(e.value())),
                    }
                }
                match &*preview.read() {
                    Some(Some(Ok(p))) => rsx! {
                        p { class: "field-hint", "Trains {p.model} on {p.device}." }
                        if !p.problems.is_empty() || (p.name_taken && !own_run) {
                            ul { class: "param-problems", aria_label: "Fix these first",
                                if p.name_taken && !own_run {
                                    li { class: "field-error", {name_taken_message(&name)} }
                                }
                                for problem in p.problems.iter() {
                                    li { key: "{problem}", class: "field-error", "{problem}" }
                                }
                            }
                        }
                        for note in p.notes.iter() {
                            p { key: "{note}", class: "field-hint", "{note}" }
                        }
                    },
                    Some(Some(Err(e))) => rsx! { p { class: "form-error", role: "alert", "{error_message(e)}" } },
                    Some(None) => rsx! { p { class: "field-hint", "Choose a dataset and a parameter set." } },
                    None => rsx! {},
                }
                if libraries_missing() {
                    LibrariesMissing { on_close: move |_| libraries_missing.set(false) }
                }
                if let Some(message) = start_error() {
                    p { class: "form-error", role: "alert", "{message}" }
                }
                if let Some(message) = poll_error() {
                    p { class: "form-error", role: "alert", "{message}" }
                }
                div { class: "form-actions",
                    button { class: "button", r#type: "button", disabled: !can_run, onclick: start,
                        if running { "Training…" } else { "Train" }
                    }
                }
            }

            if let Some(job) = job() {
                JobProgress { job, title: "Training", noun: "model", verb: "trained", output_label: "Result" }
            }
            if let Some(run) = job_run {
                TrainConsole { key: "{run}", run, version: job_version, live: live_line }
            }
        }
    }
}

/// Continuing stopped run `run` from its last finished epoch, with the
/// settings and dataset it started with.
#[component]
fn ContinueRun(run: TrainRecord, running: bool, job: Signal<Option<Job>>) -> Element {
    let name = run.name.clone();
    let preview = use_resource(use_reactive((&name,), |(name,)| async move { resume_preview(name).await }));
    let mut libraries_missing = use_signal(|| false);
    let mut start_error = use_signal(|| None::<String>);
    let can_run = !running && matches!(&*preview.read(), Some(Ok(p)) if p.can_run());

    let start = move |_| {
        let name = name.clone();
        async move {
            start_error.set(None);
            if !matches!(python_env_overview().await, Ok(ov) if ov.status.usable()) {
                libraries_missing.set(true);
                return;
            }
            libraries_missing.set(false);
            match resume_training(name).await {
                Ok(started) => set_if_newer(job, started),
                Err(e) => start_error.set(Some(error_message(&e))),
            }
        }
    };

    rsx! {
        section { class: "section-card", aria_labelledby: "train-continue-title",
            h2 { id: "train-continue-title", class: "section-card-title",
                TitleIcon { name: IconName::History }
                "Continue training "
                InfoTip {
                    label: "About continuing",
                    text: "Picks up the run selected in the Models table where it stopped: same dataset, parameters and folder, with the learning rate and optimizer as they were. To train it anew instead, use Train below with another name.",
                }
            }
            div { class: "train-input-chosen",
                span { class: "train-input-name", "{run.name}" }
                span { class: "train-input-detail", "{run.dataset_name} · {run.model} · stopped at epoch {run.epochs_done} of {run.epochs}" }
            }
            match &*preview.read() {
                Some(Ok(p)) => rsx! {
                    if p.problems.is_empty() {
                        p { class: "field-hint", "Continues on {p.device}." }
                    } else {
                        ul { class: "param-problems", aria_label: "Can't continue",
                            for problem in p.problems.iter() {
                                li { key: "{problem}", class: "field-error", "{problem}" }
                            }
                        }
                    }
                    for note in p.notes.iter() {
                        p { key: "{note}", class: "field-hint", "{note}" }
                    }
                },
                Some(Err(e)) => rsx! { p { class: "form-error", role: "alert", "{error_message(e)}" } },
                None => rsx! {},
            }
            if libraries_missing() {
                LibrariesMissing { on_close: move |_| libraries_missing.set(false) }
            }
            if let Some(message) = start_error() {
                p { class: "form-error", role: "alert", "{message}" }
            }
            div { class: "form-actions",
                button { class: "button", r#type: "button", disabled: !can_run, onclick: start, "Continue" }
            }
        }
    }
}

/// Ultralytics' output for run `run`, fetched as the job moves (`version`);
/// `live` is the progress bar being redrawn right now. `tuning`: a tuning
/// run's console (`training/tuning/<run>/console.log`).
#[component]
pub(crate) fn TrainConsole(run: String, version: u64, live: Option<String>, #[props(default)] tuning: bool) -> Element {
    let mut text = use_signal(String::new);
    let mut line_count = use_signal(|| 0usize);
    let mut offset = use_signal(|| 0u64);
    let mut fetching = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let run_name = run.clone();

    use_effect(use_reactive((&version,), move |_| {
        if *fetching.peek() {
            return;
        }
        fetching.set(true);
        let run = run_name.clone();
        spawn(async move {
            // Catch up in chunks (a long log after a reload).
            loop {
                let from = *offset.peek();
                let read = if tuning { tune_log(run.clone(), from).await } else { train_log(run.clone(), from).await };
                match read {
                    Ok(log) => {
                        let got = !log.text.is_empty();
                        if got {
                            // Appended in place; only the new text is counted.
                            let mut lines = *line_count.peek() + log.text.matches('\n').count();
                            let mut all = text.write();
                            all.push_str(&log.text);
                            if lines > CONSOLE_LINES {
                                let cut = all.match_indices('\n').nth(lines - CONSOLE_LINES - 1).map_or(0, |(i, _)| i + 1);
                                all.drain(..cut);
                                lines = CONSOLE_LINES;
                            }
                            drop(all);
                            line_count.set(lines);
                        }
                        offset.set(log.offset);
                        if !got {
                            break;
                        }
                    }
                    Err(e) => {
                        error.set(Some(error_message(&e)));
                        break;
                    }
                }
            }
            fetching.set(false);
            // Follow the end unless the user scrolled up.
            document::eval(
                "const e = document.getElementById('train-console'); if (e) { \
                 if (!e.dataset.hooked) { e.dataset.hooked = '1'; e.addEventListener('scroll', () => { \
                 e.dataset.follow = (e.scrollHeight - e.scrollTop - e.clientHeight < 30) ? '1' : '0'; }); } \
                 if (e.dataset.follow !== '0') e.scrollTop = e.scrollHeight; }",
            );
        });
    }));

    rsx! {
        // Collapsed by default; the progress panel above says how it goes.
        details { class: "section-card param-group", aria_labelledby: "train-console-title",
            summary { class: "param-group-summary",
                h2 { id: "train-console-title", class: "section-card-title", TitleIcon { name: IconName::Terminal } "Console" }
                span { class: "section-card-meta", if tuning { "training/tuning/{run}/console.log" } else { "training/trains/{run}/console.log" } }
            }
            pre { id: "train-console", class: "train-console", tabindex: "0", aria_label: "Training output",
                if text.read().is_empty() && live.is_none() { "Waiting for output…" }
                "{text}"
                if let Some(line) = &live {
                    span { class: "train-console-live", "{line}" }
                }
            }
            if let Some(message) = error() {
                p { class: "field-error", "{message}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::parameters::Weights;

    #[test]
    fn suggests_dataset_and_model_names() {
        let mut set = ParameterSet::new("baseline");
        assert_eq!(model_stem(&set), "yolo26n");
        let FrameworkParams::Ultralytics(p) = &mut set.params;
        p.model.weights = Weights::File("training/trains/old/weights/best.pt".into());
        assert_eq!(model_stem(&set), "best");

        let taken = vec!["cars-yolo11n".to_string(), "cars-yolo11n-2".to_string()];
        assert_eq!(suggest_name("cars", "yolo11n", &[]), "cars-yolo11n");
        assert_eq!(suggest_name("cars", "yolo11n", &taken), "cars-yolo11n-3");
    }
}
