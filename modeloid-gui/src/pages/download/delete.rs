//! Delete tab: remove the datasets selected in the datasets panel from the
//! workspace, for good.

use dioxus::prelude::*;

use crate::api::datasets::{list_datasets, start_delete, DatasetRow};
use crate::api::error_message;
use crate::api::jobs::JobKind;
use crate::components::{set_if_newer, use_job, JobProgress, SelectDatasetHint};
use crate::state::{use_datasets_version, use_selected_datasets};
use crate::util::{format_bytes, format_count, plural};

#[component]
pub fn DeleteForm() -> Element {
    let selected = use_selected_datasets();
    let datasets_version = use_datasets_version();
    let mut confirmed = use_signal(|| false);
    let mut starting = use_signal(|| false);
    let mut form_error = use_signal(|| None::<String>);
    let (job, _) = use_job(JobKind::Delete);
    let running = job.read().as_ref().is_some_and(|j| !j.finished);

    let datasets = use_resource(move || async move {
        let _ = datasets_version();
        list_datasets().await
    });
    // The selected datasets, in selection order.
    let rows: Vec<DatasetRow> = match &*datasets.read() {
        Some(Ok(all)) => selected.read().iter().filter_map(|id| all.iter().find(|r| r.id == *id).cloned()).collect(),
        _ => Vec::new(),
    };
    let count = rows.len();
    let size: u64 = rows.iter().map(|r| r.size_bytes).sum();

    // A changed selection has to be confirmed again.
    use_effect(move || {
        let _ = selected.read().len();
        confirmed.set(false);
    });

    let submit = move |e: FormEvent| async move {
        e.prevent_default();
        let ids = selected();
        if ids.is_empty() || !confirmed() || starting() {
            return;
        }
        form_error.set(None);
        starting.set(true);
        match start_delete(ids).await {
            Ok(started) => {
                set_if_newer(job, started);
                confirmed.set(false);
            }
            Err(e) => form_error.set(Some(error_message(&e))),
        }
        starting.set(false);
    };

    rsx! {
        if count == 0 {
            SelectDatasetHint {}
        } else {
            form { class: "section-stack", novalidate: true, onsubmit: submit,
                section { class: "section-card", aria_labelledby: "delete-title",
                    div { class: "section-card-header",
                        h2 { id: "delete-title", class: "section-card-title", "Delete" }
                        span { class: "section-card-meta", "{plural(count, \"dataset\")} · {format_bytes(size)}" }
                    }
                    div { class: "table-scroll",
                        table { class: "data-table delete-table",
                            thead {
                                tr {
                                    th { class: "num", scope: "col", abbr: "ID", "#" }
                                    th { scope: "col", "Name" }
                                    th { class: "num", scope: "col", "Images" }
                                    th { class: "num", scope: "col", "Labels" }
                                    th { class: "num", scope: "col", "Size" }
                                }
                            }
                            tbody {
                                for row in rows.iter() {
                                    tr { key: "{row.id}",
                                        td { class: "num muted", "{row.id}" }
                                        th { class: "name", scope: "row", title: "{row.name}", "{row.name}" }
                                        td { class: "num", "{format_count(row.images)}" }
                                        td { class: "num", "{format_count(row.labels)}" }
                                        td { class: "num muted", "{format_bytes(row.size_bytes)}" }
                                    }
                                }
                            }
                        }
                    }
                    p { class: "field-hint",
                        "Their folders are removed from the workspace's datasets folder, with all images and labels. Datasets made from them stay. Change the selection in the datasets panel."
                    }
                    label { class: "check",
                        input {
                            r#type: "checkbox",
                            checked: confirmed(),
                            disabled: running,
                            onchange: move |e| confirmed.set(e.checked()),
                        }
                        span {
                            "I understand "
                            strong { if count == 1 { "this dataset" } else { "these {count} datasets" } }
                            " will be deleted. This can't be undone."
                        }
                    }
                }

                if let Some(message) = form_error() {
                    p { class: "form-error", role: "alert", "{message}" }
                }
                div { class: "form-actions",
                    button {
                        class: "button button-danger",
                        r#type: "submit",
                        disabled: !confirmed() || running || starting(),
                        if running { "Deleting…" } else { "Delete {plural(count, \"dataset\")}" }
                    }
                    if !confirmed() && !running {
                        span { class: "form-actions-note", "Tick the box above first." }
                    }
                }
            }
        }

        if let Some(current) = job() {
            JobProgress { job: current, title: "Deleting", noun: "dataset", verb: "deleted", output_label: "" }
        }
    }
}
