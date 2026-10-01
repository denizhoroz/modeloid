//! The right-hand panel of the tool pages. It holds several tables; the
//! panel's title switches between them. Each category of the sidebar opens it
//! on its own table (`PanelTable::for_section`).

use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::parameters::{list_parameters, FrameworkParams, ParameterSet};
use crate::components::table::{self, CheckHeader, Column, RowCheck, SelectionBar, Sort, SortHeader};
use crate::components::{DatasetsPanel, Icon, IconName};
use crate::state::{use_parameters_version, use_selected_parameters, use_workspace};
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
}

impl PanelTable {
    /// In menu order.
    pub const ALL: [PanelTable; 2] = [PanelTable::Datasets, PanelTable::Parameters];

    pub fn label(self) -> &'static str {
        match self {
            PanelTable::Datasets => "Datasets",
            PanelTable::Parameters => "Parameters",
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
                }
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
    fn categories_open_their_table() {
        assert_eq!(PanelTable::for_section("dataset"), Some(PanelTable::Datasets));
        assert_eq!(PanelTable::for_section("training"), Some(PanelTable::Parameters));
        assert_eq!(PanelTable::for_section("settings"), None);
    }
}
