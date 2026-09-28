use dioxus::prelude::*;

use crate::api::datasets::{list_datasets, short_task, DatasetRow};
use crate::api::error_message;
use crate::api::health::start_rescan;
use crate::api::jobs::{ItemStatus, JobKind};
use crate::components::{set_if_newer, use_job, Icon, IconName};
use crate::state::{use_dataset_pick, use_datasets_version, use_selected_dataset, use_workspace, DatasetPick};
use crate::util::{format_bytes, format_count, plural};

/// Right-hand panel listing the open workspace's datasets.
#[component]
pub fn DatasetsPanel() -> Element {
    let workspace = use_workspace();
    let version = use_datasets_version();

    // Rescan runs as a job; the panel shows where it is and any failures.
    let (rescan, _) = use_job(JobKind::Rescan);
    let mut rescan_error = use_signal(|| None::<String>);
    let rescanning = rescan.read().as_ref().is_some_and(|j| !j.finished);
    let spin_class = if rescanning { "is-spinning" } else { "" };
    let rescan_status = rescan.read().as_ref().and_then(|job| {
        let failed = job.items.iter().filter(|i| i.status == ItemStatus::Failed).count();
        if !job.finished {
            let done = job.items.iter().filter(|i| matches!(i.status, ItemStatus::Done | ItemStatus::Failed)).count();
            Some(format!("Rescanning {} of {}…", (done + 1).min(job.items.len()), job.items.len()))
        } else if failed > 0 {
            Some(format!("Rescan: {} failed", plural(failed, "dataset")))
        } else {
            None
        }
    });

    let datasets = use_resource(move || async move {
        // Re-run when the workspace or its datasets change.
        let _ = (workspace.read().as_ref().map(|w| w.path.clone()), version());
        list_datasets().await
    });

    let count = match &*datasets.read() {
        Some(Ok(rows)) => Some(rows.len()),
        _ => None,
    };

    // Drop the selection when its dataset is gone (deleted, or moved outside the app).
    let mut selected = use_selected_dataset();
    use_effect(move || {
        if let Some(Ok(rows)) = &*datasets.read() {
            if selected.peek().is_some_and(|id| !rows.iter().any(|r| r.id == id)) {
                selected.set(None);
            }
        }
    });

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
                    aria_label: "Rescan datasets",
                    title: "Rescan: re-read every dataset folder and add new ones",
                    disabled: rescanning || workspace.read().is_none(),
                    onclick: move |_| async move {
                        rescan_error.set(None);
                        match start_rescan().await {
                            Ok(job) => set_if_newer(rescan, job),
                            Err(e) => rescan_error.set(Some(error_message(&e))),
                        }
                    },
                    Icon { name: IconName::Refresh, size: 14, class: spin_class }
                }
            }
            if let Some(status) = rescan_status {
                p { class: "panel-status", aria_live: "polite", "{status}" }
            }
            if let Some(message) = rescan_error() {
                p { class: "field-error panel-status", role: "alert", "{message}" }
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
    let mut selected = use_selected_dataset();
    // With a DatasetPicker on the page, a click also picks the dataset as input.
    let pickable = pickers() > 0;
    let hint = if pickable { "Select, and use as input" } else { "Select" };

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
                    tr {
                        key: "{row.id}",
                        class: if selected() == Some(row.id) { "is-selected" },
                        // Clicking a selected row again clears the selection; picking always adds.
                        onclick: move |_| {
                            let id = row.id;
                            selected.set(if selected() == Some(id) { None } else { Some(id) });
                            if pickable {
                                picked.set(Some(id));
                            }
                        },
                        td { class: "num muted", "{row.id}" }
                        th { class: "name", scope: "row",
                            // Keyboard access; the click bubbles to the row.
                            button {
                                class: "row-pick",
                                r#type: "button",
                                title: "{hint}: {row.name}",
                                aria_pressed: if selected() == Some(row.id) { "true" } else { "false" },
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
