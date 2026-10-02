//! The right-hand panel of the tool pages. It holds several tables; the
//! panel's title switches between them. Each category of the sidebar opens it
//! on its own table (`PanelTable::for_section`).

use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::parameters::{delete_parameters, list_parameters, FrameworkParams, ParameterSet};
use crate::components::table::{self, CheckHeader, Column, RowCheck, SelectionBar, Sort, SortHeader};
use crate::components::{DatasetsPanel, Icon, IconName};
use crate::api::jobs::JobKind;
use crate::api::train::{delete_models, list_models, TrainRecord, TrainStatus};
use crate::components::use_job;
use crate::state::{use_models_version, use_parameters_version, use_selected_models, use_selected_parameters, use_workspace};
use crate::util::plural;

/// The table the side panel shows; pages can switch it (e.g. "Choose in the
/// Datasets table"). Provided by `AppLayout`.
#[derive(Clone, Copy)]
pub struct PanelTableState(pub Signal<PanelTable>);

pub fn use_panel_table() -> Signal<PanelTable> {
    use_context::<PanelTableState>().0
}

/// A table the side panel can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelTable {
    Datasets,
    Parameters,
    Models,
}

impl PanelTable {
    /// In menu order.
    pub const ALL: [PanelTable; 3] = [PanelTable::Datasets, PanelTable::Parameters, PanelTable::Models];

    pub fn label(self) -> &'static str {
        match self {
            PanelTable::Datasets => "Datasets",
            PanelTable::Parameters => "Parameters",
            PanelTable::Models => "Models",
        }
    }

    /// The table a page opens on: its own (Performance works on a model),
    /// else its category's.
    pub fn for_tool(section: &str, tool: &str) -> Option<PanelTable> {
        match (section, tool) {
            ("training", "performance") => Some(PanelTable::Models),
            _ => PanelTable::for_section(section),
        }
    }

    /// The table a sidebar category opens on; `None` for pages without the panel.
    pub fn for_section(section: &str) -> Option<PanelTable> {
        match section {
            "dataset" => Some(PanelTable::Datasets),
            "training" => Some(PanelTable::Parameters),
            _ => None,
        }
    }
}

/// Both tables stay mounted and the other one is hidden, so the datasets
/// table keeps following the selection and its sort while Parameters shows.
#[component]
pub fn SidePanel(table: Signal<PanelTable>) -> Element {
    rsx! {
        aside { id: "side-panel", class: "datasets-panel", aria_label: "{table().label()}",
            DatasetsPanel { table }
            ParametersPanel { table }
            ModelsPanel { table }
        }
    }
}

/// The panel's title: a button that opens the list of tables.
#[component]
pub fn PanelSwitcher(table: Signal<PanelTable>) -> Element {
    let mut open = use_signal(|| false);
    let current = table();

    rsx! {
        div {
            class: "panel-switch",
            onkeydown: move |e| {
                if e.key() == Key::Escape {
                    open.set(false);
                }
            },
            button {
                class: "panel-switch-button section-card-title",
                r#type: "button",
                title: "Show another table",
                aria_haspopup: "menu",
                aria_expanded: if open() { "true" } else { "false" },
                onclick: move |_| open.toggle(),
                "{current.label()}"
                Icon { name: IconName::ChevronDown, size: 14, class: if open() { "panel-switch-chevron is-open" } else { "panel-switch-chevron" } }
            }
            if open() {
                // Catches a click anywhere else, which closes the menu.
                div { class: "panel-switch-backdrop", onclick: move |_| open.set(false) }
                ul { class: "panel-switch-menu", role: "menu", aria_label: "Tables",
                    for option in PanelTable::ALL {
                        li { key: "{option.label()}", role: "none",
                            button {
                                class: if option == current { "panel-switch-option is-selected" } else { "panel-switch-option" },
                                r#type: "button",
                                role: "menuitemradio",
                                aria_checked: if option == current { "true" } else { "false" },
                                onclick: move |_| {
                                    table.set(option);
                                    open.set(false);
                                },
                                "{option.label()}"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A parameter set as the table lists it.
#[derive(Debug, Clone, PartialEq)]
struct ParamRow {
    /// 1, 2, 3… in the order the sets were created (sets have no stored id).
    id: usize,
    name: String,
    description: Option<String>,
    model: String,
    epochs: u32,
    imgsz: u32,
    /// "2026-10-01"
    updated: String,
}

/// The Parameters table's sortable columns, in order.
const PARAM_COLUMNS: [Column; 6] =
    [Column::id(), Column::text("Name"), Column::text("Model"), Column::number("Epochs"), Column::number("Img"), Column::text("Updated")];

fn param_rows(sets: &[ParameterSet]) -> Vec<ParamRow> {
    let mut by_age: Vec<&ParameterSet> = sets.iter().collect();
    by_age.sort_by(|a, b| a.created.cmp(&b.created).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    by_age
        .into_iter()
        .enumerate()
        .map(|(i, set)| {
            let FrameworkParams::Ultralytics(p) = &set.params;
            ParamRow {
                id: i + 1,
                name: set.name.clone(),
                description: set.description.clone(),
                model: set.summary(),
                epochs: p.training.epochs,
                imgsz: p.training.imgsz,
                updated: set.updated.get(..10).unwrap_or(&set.updated).to_string(),
            }
        })
        .collect()
}

/// Sorts `rows` by `sort`'s column; equal rows stay in id order.
fn sort_param_rows(sort: Sort, rows: &mut [ParamRow]) {
    let text = |r: &ParamRow| match sort.column {
        1 => r.name.to_lowercase(),
        2 => r.model.to_lowercase(),
        5 => r.updated.clone(),
        _ => String::new(),
    };
    let number = |r: &ParamRow| match sort.column {
        0 => r.id as u64,
        3 => u64::from(r.epochs),
        4 => u64::from(r.imgsz),
        _ => 0,
    };
    let numeric = PARAM_COLUMNS.get(sort.column).is_some_and(|c| c.numeric);
    rows.sort_by(|a, b| {
        let order = if numeric { number(a).cmp(&number(b)) } else { text(a).cmp(&text(b)) };
        sort.directed(order).then(a.id.cmp(&b.id))
    });
}

/// Saved parameter sets, with the same features as the Datasets table:
/// checkboxes and Select all / None (the pages use the last one selected),
/// a row click selects only that set (again: clears, i.e. a new set),
/// sortable columns, a count and a Reload button.
#[component]
fn ParametersPanel(table: Signal<PanelTable>) -> Element {
    let workspace = use_workspace();
    let mut version = use_parameters_version();
    let selected = use_selected_parameters();
    let mut remove_error = use_signal(|| None::<String>);
    let list = use_resource(move || async move {
        let _ = (workspace.read().as_ref().map(|w| w.path.clone()), version());
        list_parameters().await
    });

    // Sets that are gone (deleted, renamed on disk) leave the selection.
    use_effect(move || {
        if let Some(Ok(list)) = &*list.read() {
            let names: Vec<&String> = list.sets.iter().map(|s| &s.name).collect();
            if selected.peek().iter().any(|n| !names.contains(&n)) {
                table::update(selected, |s| s.iter().filter(|n| names.contains(n)).cloned().collect());
            }
        }
    });

    let sort = use_signal(Sort::by_id);
    let rows: Option<Vec<ParamRow>> = match &*list.read() {
        Some(Ok(list)) => {
            let mut rows = param_rows(&list.sets);
            sort_param_rows(sort(), &mut rows);
            Some(rows)
        }
        _ => None,
    };
    let all_names: Vec<String> = rows.iter().flatten().map(|r| r.name.clone()).collect();
    let total = all_names.len();
    let selected_count = selected.read().len();
    let loading = list.read().is_none();

    rsx! {
        div { class: "panel-view", hidden: table() != PanelTable::Parameters,
            div { class: "datasets-panel-header",
                PanelSwitcher { table }
                if rows.is_some() {
                    span { class: "section-card-meta", "{plural(total, \"set\")}" }
                }
                button {
                    class: "icon-button icon-button-sm",
                    r#type: "button",
                    aria_label: "Reload parameter sets",
                    title: "Reload: read the parameter files again",
                    disabled: workspace.read().is_none(),
                    onclick: move |_| *version.write() += 1,
                    Icon { name: IconName::Refresh, size: 14, class: if loading { "is-spinning" } else { "" } }
                }
            }
            if let Some(Ok(list)) = &*list.read() {
                for warning in list.warnings.iter() {
                    p { key: "{warning}", class: "field-error panel-status", "{warning}" }
                }
            }
            if total > 0 {
                SelectionBar {
                    label: "Parameter set selection",
                    selected: selected_count,
                    total,
                    on_all: move |_| table::update(selected, |s| table::all(s, &all_names)),
                    on_none: move |_| table::update(selected, |_| Vec::new()),
                    noun: "parameter set",
                    remove_note: "Their files in training/parameters are deleted.",
                    on_remove: move |_| async move {
                        remove_error.set(None);
                        let names = selected.peek().clone();
                        for name in names {
                            if let Err(e) = delete_parameters(name.clone()).await {
                                remove_error.set(Some(format!("Couldn't delete {name}: {}", error_message(&e))));
                                break;
                            }
                        }
                        *version.write() += 1;
                    },
                }
            }
            if let Some(message) = remove_error() {
                p { class: "field-error panel-status", role: "alert", "{message}" }
            }
            div { class: "datasets-panel-body",
                match (&*list.read(), &rows) {
                    _ if workspace.read().is_none() => rsx! {
                        p { class: "panel-empty", "No workspace loaded." }
                    },
                    (None, _) => rsx! { p { class: "panel-empty", "Loading…" } },
                    (Some(Err(e)), _) => rsx! {
                        p { class: "field-error panel-empty", role: "alert", "{error_message(e)}" }
                    },
                    (_, Some(rows)) if rows.is_empty() => rsx! {
                        p { class: "panel-empty", "No parameter sets yet. Fill in the form on the Parameters page and choose Add Parameter." }
                    },
                    (_, Some(rows)) => rsx! { ParametersTable { rows: rows.clone(), sort } },
                    _ => rsx! {},
                }
            }
        }
    }
}

#[component]
fn ParametersTable(rows: Vec<ParamRow>, sort: Signal<Sort>) -> Element {
    let selected = use_selected_parameters();

    rsx! {
        table { class: "data-table",
            thead {
                tr {
                    CheckHeader {}
                    for (column, def) in PARAM_COLUMNS.into_iter().enumerate() {
                        SortHeader { key: "{column}", column, def, sort }
                    }
                }
            }
            tbody {
                for row in rows {
                    {
                        let is_selected = selected.read().contains(&row.name);
                        let (click_name, check_name) = (row.name.clone(), row.name.clone());
                        rsx! {
                            tr {
                                key: "{row.name}",
                                class: if is_selected { "is-selected" },
                                onclick: move |_| table::update(selected, |s| table::clicked(s, &click_name)),
                                RowCheck {
                                    name: row.name.clone(),
                                    checked: is_selected,
                                    on_change: move |on| table::update(selected, |s| table::toggled(s, &check_name, on)),
                                }
                                td { class: "num muted", "{row.id}" }
                                th { class: "name", scope: "row",
                                    // Keyboard access; the click bubbles to the row.
                                    button {
                                        class: "row-pick",
                                        r#type: "button",
                                        title: row.description.clone().unwrap_or_else(|| format!("Select only {}", row.name)),
                                        aria_pressed: if is_selected { "true" } else { "false" },
                                        "{row.name}"
                                    }
                                }
                                td { "{row.model}" }
                                td { class: "num", "{row.epochs}" }
                                td { class: "num", "{row.imgsz}" }
                                td { class: "muted", "{row.updated}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A trained model as the Models table lists it.
#[derive(Debug, Clone, PartialEq)]
struct ModelRow {
    /// 1, 2, 3… in the order the runs started.
    id: usize,
    name: String,
    dataset: String,
    model: String,
    status: TrainStatus,
    epochs_done: u32,
    epochs: u32,
    /// "2026-10-01 13:07"
    started: String,
    best: Option<String>,
}

/// The Models table's sortable columns, in order.
const MODEL_COLUMNS: [Column; 7] = [
    Column::id(),
    Column::text("Name"),
    Column::text("Dataset"),
    Column::text("Model"),
    Column::text("Status"),
    Column::number("Epochs"),
    Column::text("Started"),
];

pub fn status_label(status: TrainStatus) -> &'static str {
    match status {
        TrainStatus::Running => "running",
        TrainStatus::Done => "done",
        TrainStatus::Failed => "failed",
        TrainStatus::Stopped => "stopped",
    }
}

/// `records` come by start time, so their position is the id.
fn model_rows(records: &[TrainRecord]) -> Vec<ModelRow> {
    records
        .iter()
        .enumerate()
        .map(|(i, r)| ModelRow {
            id: i + 1,
            name: r.name.clone(),
            dataset: r.dataset_name.clone(),
            model: r.model.clone(),
            status: r.status,
            epochs_done: r.epochs_done,
            epochs: r.epochs,
            started: r.started.get(..16).unwrap_or(&r.started).replace('T', " "),
            best: r.best.clone(),
        })
        .collect()
}

/// Sorts `rows` by `sort`'s column; equal rows stay in id order.
fn sort_model_rows(sort: Sort, rows: &mut [ModelRow]) {
    let text = |r: &ModelRow| match sort.column {
        1 => r.name.to_lowercase(),
        2 => r.dataset.to_lowercase(),
        3 => r.model.to_lowercase(),
        4 => status_label(r.status).to_string(),
        6 => r.started.clone(),
        _ => String::new(),
    };
    let number = |r: &ModelRow| match sort.column {
        0 => r.id as u64,
        5 => u64::from(r.epochs_done),
        _ => 0,
    };
    let numeric = MODEL_COLUMNS.get(sort.column).is_some_and(|c| c.numeric);
    rows.sort_by(|a, b| {
        let order = if numeric { number(a).cmp(&number(b)) } else { text(a).cmp(&text(b)) };
        sort.directed(order).then(a.id.cmp(&b.id))
    });
}

/// Trained models (`training/trains/<name>`), with the same features as the
/// other tables.
#[component]
fn ModelsPanel(table: Signal<PanelTable>) -> Element {
    let workspace = use_workspace();
    let mut version = use_models_version();
    let selected = use_selected_models();
    let mut remove_error = use_signal(|| None::<String>);
    // A run that starts or ends changes the list at once.
    let (train_job, _) = use_job(JobKind::Train);
    let train_state = use_memo(move || train_job.read().as_ref().map(|j| (j.finished, j.items.first().map(|i| i.label.clone()))));
    let list = use_resource(move || async move {
        let _ = (workspace.read().as_ref().map(|w| w.path.clone()), version(), train_state());
        list_models().await
    });

    use_effect(move || {
        if let Some(Ok(list)) = &*list.read() {
            let names: Vec<&String> = list.models.iter().map(|m| &m.name).collect();
            if selected.peek().iter().any(|n| !names.contains(&n)) {
                table::update(selected, |s| s.iter().filter(|n| names.contains(n)).cloned().collect());
            }
        }
    });

    let sort = use_signal(Sort::by_id);
    let rows: Option<Vec<ModelRow>> = match &*list.read() {
        Some(Ok(list)) => {
            let mut rows = model_rows(&list.models);
            sort_model_rows(sort(), &mut rows);
            Some(rows)
        }
        _ => None,
    };
    let all_names: Vec<String> = rows.iter().flatten().map(|r| r.name.clone()).collect();
    let total = all_names.len();
    let selected_count = selected.read().len();
    let loading = list.read().is_none();

    rsx! {
        div { class: "panel-view", hidden: table() != PanelTable::Models,
            div { class: "datasets-panel-header",
                PanelSwitcher { table }
                if rows.is_some() {
                    span { class: "section-card-meta", "{plural(total, \"model\")}" }
                }
                button {
                    class: "icon-button icon-button-sm",
                    r#type: "button",
                    aria_label: "Reload models",
                    title: "Reload: read the training runs again",
                    disabled: workspace.read().is_none(),
                    onclick: move |_| *version.write() += 1,
                    Icon { name: IconName::Refresh, size: 14, class: if loading { "is-spinning" } else { "" } }
                }
            }
            if let Some(Ok(list)) = &*list.read() {
                for warning in list.warnings.iter() {
                    p { key: "{warning}", class: "field-error panel-status", "{warning}" }
                }
            }
            if total > 0 {
                SelectionBar {
                    label: "Model selection",
                    selected: selected_count,
                    total,
                    on_all: move |_| table::update(selected, |s| table::all(s, &all_names)),
                    on_none: move |_| table::update(selected, |_| Vec::new()),
                    noun: "model",
                    remove_note: "Their folders in training/trains (weights, results, console log) are deleted.",
                    on_remove: move |_| async move {
                        remove_error.set(None);
                        let names = selected.peek().clone();
                        if let Err(e) = delete_models(names).await {
                            remove_error.set(Some(error_message(&e)));
                        }
                        *version.write() += 1;
                    },
                }
            }
            if let Some(message) = remove_error() {
                p { class: "field-error panel-status", role: "alert", "{message}" }
            }
            div { class: "datasets-panel-body",
                match (&*list.read(), &rows) {
                    _ if workspace.read().is_none() => rsx! {
                        p { class: "panel-empty", "No workspace loaded." }
                    },
                    (None, _) => rsx! { p { class: "panel-empty", "Loading…" } },
                    (Some(Err(e)), _) => rsx! {
                        p { class: "field-error panel-empty", role: "alert", "{error_message(e)}" }
                    },
                    (_, Some(rows)) if rows.is_empty() => rsx! {
                        p { class: "panel-empty", "No trained models yet. Train one on the Train Model page." }
                    },
                    (_, Some(rows)) => rsx! { ModelsTable { rows: rows.clone(), sort } },
                    _ => rsx! {},
                }
            }
        }
    }
}

#[component]
fn ModelsTable(rows: Vec<ModelRow>, sort: Signal<Sort>) -> Element {
    let selected = use_selected_models();

    rsx! {
        table { class: "data-table",
            thead {
                tr {
                    CheckHeader {}
                    for (column, def) in MODEL_COLUMNS.into_iter().enumerate() {
                        SortHeader { key: "{column}", column, def, sort }
                    }
                }
            }
            tbody {
                for row in rows {
                    {
                        let is_selected = selected.read().contains(&row.name);
                        let (click_name, check_name) = (row.name.clone(), row.name.clone());
                        let status = status_label(row.status);
                        let title = match &row.best {
                            Some(best) => format!("Best weights: {best}"),
                            None => format!("Select only {}", row.name),
                        };
                        rsx! {
                            tr {
                                key: "{row.name}",
                                class: if is_selected { "is-selected" },
                                onclick: move |_| table::update(selected, |s| table::clicked(s, &click_name)),
                                RowCheck {
                                    name: row.name.clone(),
                                    checked: is_selected,
                                    on_change: move |on| table::update(selected, |s| table::toggled(s, &check_name, on)),
                                }
                                td { class: "num muted", "{row.id}" }
                                th { class: "name", scope: "row",
                                    button {
                                        class: "row-pick",
                                        r#type: "button",
                                        title,
                                        aria_pressed: if is_selected { "true" } else { "false" },
                                        "{row.name}"
                                    }
                                }
                                td { "{row.dataset}" }
                                td { class: "muted", "{row.model}" }
                                td { span { class: "run-status is-{status}", "{status}" } }
                                td { class: "num", "{row.epochs_done}/{row.epochs}" }
                                td { class: "muted", "{row.started}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_sets_by_creation_and_sorts() {
        let mut sets = vec![ParameterSet::new("b"), ParameterSet::new("a"), ParameterSet::new("c")];
        sets[0].created = "2026-10-02T00:00:00Z".into();
        sets[1].created = "2026-10-01T00:00:00Z".into();
        sets[2].created = "2026-10-03T00:00:00Z".into();
        let mut rows = param_rows(&sets);
        let names = |rows: &[ParamRow]| rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&rows), ["a", "b", "c"], "oldest is #1");
        sort_param_rows(Sort { column: 0, ascending: false }, &mut rows);
        assert_eq!(names(&rows), ["c", "b", "a"]);
        sort_param_rows(Sort { column: 1, ascending: true }, &mut rows);
        assert_eq!(names(&rows), ["a", "b", "c"]);
    }

    #[test]
    fn lists_models_by_start() {
        let record = |name: &str, epochs_done: u32| TrainRecord {
            name: name.into(),
            started: "2026-10-01T13:07:27Z".into(),
            epochs_done,
            epochs: 10,
            ..Default::default()
        };
        let mut rows = model_rows(&[record("a", 10), record("b", 3)]);
        assert_eq!(rows[0].started, "2026-10-01 13:07");
        sort_model_rows(Sort { column: 5, ascending: false }, &mut rows);
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), [1, 2], "most epochs first");
    }

    #[test]
    fn categories_open_their_table() {
        assert_eq!(PanelTable::for_section("dataset"), Some(PanelTable::Datasets));
        assert_eq!(PanelTable::for_section("training"), Some(PanelTable::Parameters));
        assert_eq!(PanelTable::for_section("settings"), None);
    }
}
