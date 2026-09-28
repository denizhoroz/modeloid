use dioxus::prelude::*;

use crate::api::datasets::{list_datasets, short_task, DatasetRow};
use crate::api::error_message;
use crate::components::{Icon, IconName};
use crate::state::{use_dataset_pick, DatasetPick, use_datasets_version, use_workspace};
use crate::util::{format_bytes, format_count, plural};

/// Right-hand panel listing the open workspace's datasets.
#[component]
pub fn DatasetsPanel() -> Element {
    let workspace = use_workspace();
    let mut version = use_datasets_version();

    let datasets = use_resource(move || async move {
        // Re-run when the workspace or its datasets change.
        let _ = (workspace.read().as_ref().map(|w| w.path.clone()), version());
        list_datasets().await
    });

    let count = match &*datasets.read() {
        Some(Ok(rows)) => Some(rows.len()),
        _ => None,
    };

    rsx! {
        aside { id: "datasets-panel", class: "datasets-panel", aria_labelledby: "datasets-panel-title",
            div { class: "datasets-panel-header",
                h2 { id: "datasets-panel-title", class: "section-card-title", "Datasets" }
                if let Some(n) = count {
                    span { class: "section-card-meta", "{plural(n, \"dataset\")}" }
                }
                button {
                    class: "icon-button icon-button-sm",
                    r#type: "button",
                    aria_label: "Refresh datasets",
                    title: "Refresh",
                    onclick: move |_| *version.write() += 1,
                    Icon { name: IconName::Refresh, size: 14 }
                }
            }

            div { class: "datasets-panel-body",
                match &*datasets.read() {
                    _ if workspace.read().is_none() => rsx! {
                        p { class: "panel-empty", "No workspace loaded." }
                    },
                    None => rsx! { p { class: "panel-empty", "Loading…" } },
                    Some(Err(e)) => rsx! {
                        p { class: "field-error panel-empty", role: "alert", "{error_message(e)}" }
                    },
                    Some(Ok(rows)) if rows.is_empty() => rsx! {
                        p { class: "panel-empty", "No datasets yet. Download one to get started." }
                    },
                    Some(Ok(rows)) => rsx! { DatasetsTable { rows: rows.clone() } },
                }
            }
        }
    }
}

#[component]
fn DatasetsTable(rows: Vec<DatasetRow>) -> Element {
    let DatasetPick { pickers, mut picked } = use_dataset_pick();
    // Rows pick a dataset only when the page has a DatasetPicker.
    let pickable = pickers() > 0;

    rsx! {
        table { class: "data-table",
            thead {
                tr {
                    th { class: "num", scope: "col", abbr: "ID", "#" }
                    th { scope: "col", "Name" }
                    th { scope: "col", "Task" }
                    th { class: "num", scope: "col", "Images" }
                    th { class: "num", scope: "col", "Labels" }
                    th { class: "num", scope: "col", "Classes" }
                    th { scope: "col", "Format" }
                    th { class: "num", scope: "col", "Size" }
                }
            }
            tbody {
                for row in rows {
                    tr { key: "{row.id}",
                        td { class: "num muted", "{row.id}" }
                        th { class: "name", scope: "row", title: "{row.name}",
                            if pickable {
                                button {
                                    class: "row-pick",
                                    r#type: "button",
                                    title: "Use {row.name} as input",
                                    onclick: move |_| picked.set(Some(row.id)),
                                    "{row.name}"
                                }
                            } else {
                                "{row.name}"
                            }
                        }
                        td { title: "{row.task}", "{short_task(&row.task)}" }
                        td { class: "num", "{format_count(row.images)}" }
                        td { class: "num", "{format_count(row.labels)}" }
                        td { class: "num", "{row.classes}" }
                        td { class: "muted", "{row.format}" }
                        td { class: "num muted", "{format_bytes(row.size_bytes)}" }
                    }
                }
            }
        }
    }
}
