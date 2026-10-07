use std::collections::HashMap;

use dioxus::prelude::*;

use crate::api::datasets::{list_datasets, short_task, DatasetRow};
use crate::api::error_message;
use crate::api::datasets::start_delete;
use crate::api::health::start_rescan;
use crate::api::jobs::{ItemStatus, JobKind};
use crate::components::table::{self, CheckHeader, Column, RowCheck, SelectionBar, Sort, SortHeader};
use crate::components::{set_if_newer, use_job, Icon, IconName, PanelSwitcher, PanelTable};
use crate::state::{use_batch_selected, use_datasets_version, use_selected_datasets, use_workspace};
use crate::util::{format_bytes, format_count, plural};

/// The side panel's Datasets table: the open workspace's datasets.
#[component]
pub fn DatasetsPanel(table: Signal<PanelTable>) -> Element {
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

    // The server renumbers datasets after jobs (no gaps once one is deleted),
    // so the selection follows each dataset by its name, the folder name that
    // stays the same; datasets that are gone are dropped from it.
    let mut selected = use_selected_datasets();
    let mut names_before = use_signal(HashMap::<u32, String>::new);
    use_effect(move || {
        if let Some(Ok(rows)) = &*datasets.read() {
            let now = remap_selection(&selected.peek(), &names_before.peek(), rows);
            if now != *selected.peek() {
                selected.set(now);
            }
            names_before.set(rows.iter().map(|r| (r.id, r.name.clone())).collect());
        }
    });
    let sort = use_signal(Sort::by_id);
    let sorted: Option<Vec<DatasetRow>> = match &*datasets.read() {
        Some(Ok(rows)) => {
            let mut rows = rows.clone();
            sort_rows(sort(), &mut rows);
            Some(rows)
        }
        _ => None,
    };
    let all_ids: Vec<u32> = sorted.iter().flatten().map(|r| r.id).collect();
    let names_by_id: HashMap<u32, String> = sorted.iter().flatten().map(|r| (r.id, r.name.clone())).collect();
    let mut batch = use_batch_selected();
    let selected_count = selected.read().len();
    let total = all_ids.len();

    rsx! {
        div { class: "panel-view", hidden: table() != PanelTable::Datasets,
            div { class: "datasets-panel-header",
                PanelSwitcher { table }
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
            if total > 0 {
                SelectionBar {
                    label: "Dataset selection",
                    selected: selected_count,
                    total,
                    on_all: move |_| {
                        // Keep the current order and add the rest after it, as listed.
                        batch.set(true);
                        table::update(selected, |s| table::all(s, &all_ids));
                    },
                    on_none: move |_| {
                        batch.set(false);
                        selected.set(Vec::new());
                    },
                    noun: "dataset",
                    remove_note: "Their folders and cached previews are deleted from the workspace.",
                    busy: rescanning,
                    on_remove: move |_| {
                        let ids = selected.peek().clone();
                        let names: Vec<String> = ids.iter().map(|id| names_by_id.get(id).cloned().unwrap_or_default()).collect();
                        async move {
                            rescan_error.set(None);
                            // A job, so the status bar shows it; the panel drops deleted ones itself.
                            if let Err(e) = start_delete(ids, names).await {
                                rescan_error.set(Some(error_message(&e)));
                            }
                        }
                    },
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
                    Some(Ok(_)) => rsx! { DatasetsTable { rows: sorted.unwrap_or_default(), sort } },
                }
            }
        }
    }
}

/// Datasets with a checkbox each. The checkbox adds or removes a dataset from
/// the selection; clicking elsewhere on a row selects only that dataset
/// (clicking it again when it's the only one clears the selection).
#[component]
fn DatasetsTable(rows: Vec<DatasetRow>, sort: Signal<Sort>) -> Element {
    let selected = use_selected_datasets();
    let mut batch = use_batch_selected();
    // "Linked to" only while the workspace has a dataset link.
    let links = rows.iter().any(|r| !r.linked_to.is_empty());

    rsx! {
        table { class: "data-table",
            thead {
                tr {
                    CheckHeader {}
                    for (column, def) in columns(links).into_iter().enumerate() {
                        SortHeader { key: "{column}", column, def, sort }
                    }
                }
            }
            tbody {
                for row in rows {
                    {
                        let id = row.id;
                        let is_selected = selected.read().contains(&id);
                        let linked = row.linked_to.join(", ");
                        rsx! {
                    tr {
                        key: "{id}",
                        class: if is_selected { "is-selected" },
                        onclick: move |_| {
                            batch.set(false);
                            table::update(selected, |s| table::clicked(s, &id));
                        },
                        RowCheck {
                            name: row.name.clone(),
                            checked: is_selected,
                            on_change: move |on| {
                                batch.set(true);
                                table::update(selected, |s| table::toggled(s, &id, on));
                            },
                        }
                        td { class: "num muted", "{id}" }
                        th { class: "name", scope: "row",
                            span { class: "name-with-tag",
                                // Keyboard access; the click bubbles to the row.
                                button {
                                    class: "row-pick",
                                    r#type: "button",
                                    title: "Select only {row.name}",
                                    aria_pressed: if is_selected { "true" } else { "false" },
                                    "{row.name}"
                                }
                                if !row.linked_to.is_empty() {
                                    span { class: "badge link-badge", title: "Linked to {linked}: its images are theirs", "Link" }
                                }
                            }
                        }
                        if links {
                            td { class: "muted linked-to", title: "{linked}", "{linked}" }
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
    }
}

/// The Datasets table's sortable columns, in order; "Linked to" after the
/// name when `links` (the workspace has a dataset link).
fn columns(links: bool) -> Vec<Column> {
    let mut columns = vec![Column::id(), Column::text("Name")];
    if links {
        columns.push(Column::text("Linked to"));
    }
    columns.extend([
        Column::text("Task"),
        Column::number("Images"),
        Column::number("Labels"),
        Column::number("Classes"),
        Column::text("Format"),
        Column::number("Size"),
    ]);
    columns
}

/// Sorts `rows` by `sort`'s column (by its label, so "Linked to" coming and
/// going doesn't change what the others sort by); equal rows stay in id order.
fn sort_rows(sort: Sort, rows: &mut [DatasetRow]) {
    let links = rows.iter().any(|r| !r.linked_to.is_empty());
    let column = columns(links).get(sort.column).copied().unwrap_or(Column::id());
    let text = |r: &DatasetRow| match column.label {
        "Name" => r.name.to_lowercase(),
        "Linked to" => r.linked_to.join(", ").to_lowercase(),
        "Task" => short_task(&r.task).to_lowercase(),
        "Format" => r.format.to_lowercase(),
        _ => String::new(),
    };
    let number = |r: &DatasetRow| match column.label {
        "Images" => u64::from(r.images),
        "Labels" => u64::from(r.labels),
        "Classes" => r.classes as u64,
        "Size" => r.size_bytes,
        _ => u64::from(r.id),
    };
    let numeric = column.numeric;
    rows.sort_by(|a, b| {
        let order = if numeric { number(a).cmp(&number(b)) } else { text(a).cmp(&text(b)) };
        sort.directed(order).then(a.id.cmp(&b.id))
    });
}

/// `selected` after the datasets were read again: each id follows its
/// dataset's name from `before` (id → name at the last read) to its id in
/// `rows`; ids whose dataset is gone are dropped. Order is kept.
fn remap_selection(selected: &[u32], before: &HashMap<u32, String>, rows: &[DatasetRow]) -> Vec<u32> {
    selected
        .iter()
        .filter_map(|id| match before.get(id) {
            Some(name) => rows.iter().find(|r| r.name == *name).map(|r| r.id),
            // Not seen before (first read): kept if it exists.
            None => rows.iter().any(|r| r.id == *id).then_some(*id),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u32, name: &str, images: u32) -> DatasetRow {
        DatasetRow {
            id,
            name: name.into(),
            task: "object-detection".into(),
            format: "yolo".into(),
            images,
            labels: 0,
            classes: 0,
            linked_to: Vec::new(),
            size_bytes: 0,
        }
    }

    #[test]
    fn sorts_rows() {
        let mut rows = vec![row(1, "b", 5), row(2, "A", 9), row(3, "c", 5)];
        let ids = |rows: &[DatasetRow]| rows.iter().map(|r| r.id).collect::<Vec<_>>();

        sort_rows(Sort { column: 1, ascending: true }, &mut rows);
        assert_eq!(ids(&rows), [2, 1, 3], "ignores case");
        sort_rows(Sort { column: 3, ascending: false }, &mut rows);
        assert_eq!(ids(&rows), [2, 1, 3], "ties stay in id order");
        sort_rows(Sort { column: 0, ascending: false }, &mut rows);
        assert_eq!(ids(&rows), [3, 2, 1]);
    }

    #[test]
    fn linked_to_column_comes_and_goes() {
        assert_eq!(columns(false).iter().map(|c| c.label).collect::<Vec<_>>()[1..3], ["Name", "Task"]);
        assert_eq!(columns(true).iter().map(|c| c.label).collect::<Vec<_>>()[1..4], ["Name", "Linked to", "Task"]);

        let mut rows = vec![row(1, "b", 5), row(2, "a", 9), row(3, "c", 1)];
        rows[0].linked_to = vec!["zebra".into()];
        rows[2].linked_to = vec!["apple".into()];
        let ids = |rows: &[DatasetRow]| rows.iter().map(|r| r.id).collect::<Vec<_>>();
        sort_rows(Sort { column: 2, ascending: true }, &mut rows);
        assert_eq!(ids(&rows), [2, 3, 1], "by source; none first");
        sort_rows(Sort { column: 4, ascending: false }, &mut rows);
        assert_eq!(ids(&rows), [2, 1, 3], "Images moved one to the right");
    }

    #[test]
    fn selection_follows_renumbered_datasets() {
        let before: HashMap<u32, String> = [(1, "a"), (2, "b"), (3, "c")].map(|(id, n)| (id, n.to_string())).into();
        // "a" deleted; "b" and "c" moved down.
        let rows = vec![row(1, "b", 0), row(2, "c", 0)];
        assert_eq!(remap_selection(&[3, 1, 2], &before, &rows), [2, 1]);
        assert_eq!(remap_selection(&[2], &HashMap::new(), &rows), [2], "first read keeps ids");
        assert!(remap_selection(&[5], &HashMap::new(), &rows).is_empty());
    }
}
