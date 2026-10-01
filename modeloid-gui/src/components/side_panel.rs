//! The right-hand panel of the tool pages. It holds several tables; the
//! panel's title switches between them. Each category of the sidebar opens it
//! on its own table (`PanelTable::for_section`).

use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::parameters::{list_parameters, FrameworkParams};
use crate::components::{DatasetsPanel, Icon, IconName};
use crate::state::{use_parameters_version, use_selected_parameters, use_workspace};
use crate::util::plural;

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

/// Saved parameter sets. Clicking a row (or its name button) opens it in
/// the Parameters form; clicking the open one again starts a new set.
#[component]
fn ParametersPanel(table: Signal<PanelTable>) -> Element {
    let workspace = use_workspace();
    let version = use_parameters_version();
    let mut selected = use_selected_parameters();
    let list = use_resource(move || async move {
        let _ = (workspace.read().as_ref().map(|w| w.path.clone()), version());
        list_parameters().await
    });
    let count = match &*list.read() {
        Some(Ok(list)) => Some(list.sets.len()),
        _ => None,
    };
    let mut toggle = move |name: String| {
        let again = selected.peek().as_deref() == Some(name.as_str());
        selected.set(if again { None } else { Some(name) });
    };

    rsx! {
        div { class: "panel-view", hidden: table() != PanelTable::Parameters,
            div { class: "datasets-panel-header",
                PanelSwitcher { table }
                if let Some(n) = count {
                    span { class: "section-card-meta", "{plural(n, \"set\")}" }
                }
            }
            div { class: "datasets-panel-body",
                match &*list.read() {
                    _ if workspace.read().is_none() => rsx! {
                        p { class: "panel-empty", "No workspace loaded." }
                    },
                    None => rsx! { p { class: "panel-empty", "Loading…" } },
                    Some(Err(e)) => rsx! {
                        p { class: "field-error panel-empty", role: "alert", "{error_message(e)}" }
                    },
                    Some(Ok(list)) => rsx! {
                        for warning in list.warnings.iter() {
                            p { key: "{warning}", class: "field-error panel-status", "{warning}" }
                        }
                        if list.sets.is_empty() {
                            p { class: "panel-empty", "No parameter sets yet. Fill in the form and choose Add Parameter." }
                        } else {
                            table { class: "data-table",
                                thead {
                                    tr {
                                        th { scope: "col", "Name" }
                                        th { scope: "col", "Model" }
                                        th { scope: "col", class: "num", "Epochs" }
                                        th { scope: "col", class: "num", "Img" }
                                        th { scope: "col", "Updated" }
                                    }
                                }
                                tbody {
                                    for set in list.sets.iter().cloned() {
                                        {
                                            let is_selected = selected.read().as_deref() == Some(set.name.as_str());
                                            let FrameworkParams::Ultralytics(p) = &set.params;
                                            let (epochs, imgsz) = (p.training.epochs, p.training.imgsz);
                                            let updated = set.updated.get(..10).unwrap_or(&set.updated).to_string();
                                            let name = set.name.clone();
                                            rsx! {
                                                tr {
                                                    key: "{set.name}",
                                                    class: if is_selected { "is-selected" },
                                                    onclick: move |_| toggle(name.clone()),
                                                    th { class: "name", scope: "row",
                                                        // Keyboard access; the click bubbles to the row.
                                                        button {
                                                            class: "row-pick",
                                                            r#type: "button",
                                                            title: set.description.clone().unwrap_or_else(|| format!("Open {}", set.name)),
                                                            aria_pressed: if is_selected { "true" } else { "false" },
                                                            "{set.name}"
                                                        }
                                                    }
                                                    td { "{set.summary()}" }
                                                    td { class: "num", "{epochs}" }
                                                    td { class: "num", "{imgsz}" }
                                                    td { "{updated}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_open_their_table() {
        assert_eq!(PanelTable::for_section("dataset"), Some(PanelTable::Datasets));
        assert_eq!(PanelTable::for_section("training"), Some(PanelTable::Parameters));
        assert_eq!(PanelTable::for_section("settings"), None);
    }
}
