//! The right-hand panel of the tool pages. It holds several tables; the
//! panel's title switches between them. Each category of the sidebar opens it
//! on its own table (`PanelTable::for_section`).

use dioxus::prelude::*;

use crate::components::{DatasetsPanel, Icon, IconName};

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

/// Placeholder until training parameters exist.
#[component]
fn ParametersPanel(table: Signal<PanelTable>) -> Element {
    rsx! {
        div { class: "panel-view", hidden: table() != PanelTable::Parameters,
            div { class: "datasets-panel-header",
                PanelSwitcher { table }
            }
            div { class: "datasets-panel-body",
                p { class: "panel-empty", "Training parameters will show here." }
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
