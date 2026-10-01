use std::collections::HashMap;

use dioxus::prelude::*;

use crate::api::datasets::{list_datasets, short_task, DatasetRow};
use crate::api::error_message;
use crate::api::health::start_rescan;
use crate::api::jobs::{ItemStatus, JobKind};
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
    let sort = use_signal(|| Sort { key: SortKey::Id, ascending: true });
    let sorted: Option<Vec<DatasetRow>> = match &*datasets.read() {
        Some(Ok(rows)) => {
            let mut rows = rows.clone();
            sort().apply(&mut rows);
            Some(rows)
        }
        _ => None,
    };
    let all_ids: Vec<u32> = sorted.iter().flatten().map(|r| r.id).collect();
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
                div { class: "panel-selection", role: "group", aria_label: "Dataset selection",
                    button {
                        class: "button button-ghost button-sm",
                        r#type: "button",
                        disabled: selected_count == total,
                        onclick: move |_| {
                            // Keep the current order and add the rest after it, as listed.
                            let mut ids = selected();
                            let rest: Vec<u32> = all_ids.iter().filter(|id| !ids.contains(id)).copied().collect();
                            ids.extend(rest);
                            batch.set(true);
                            selected.set(ids);
                        },
                        "Select all"
                    }
                    button {
                        class: "button button-ghost button-sm",
                        r#type: "button",
                        disabled: selected_count == 0,
                        onclick: move |_| {
                            batch.set(false);
                            selected.set(Vec::new());
                        },
                        "None"
                    }
                    span { class: "section-card-meta", aria_live: "polite",
                        if selected_count == 0 { "Nothing selected" } else { "{selected_count} selected" }
                    }
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
                    Some(Ok(_)) => rsx! { DatasetsTable { rows: sorted.clone().unwrap_or_default(), sort } },
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
    let mut selected = use_selected_datasets();
    let mut batch = use_batch_selected();

    rsx! {
        table { class: "data-table",
            thead {
                tr {
                    th { class: "check-col", scope: "col", span { class: "visually-hidden", "Selected" } }
                    for column in SortKey::ALL {
                        SortHeader { key: "{column:?}", column, sort }
                    }
                }
            }
            tbody {
                for row in rows {
                    {
                        let id = row.id;
                        let is_selected = selected.read().contains(&id);
                        rsx! {
                    tr {
                        key: "{id}",
                        class: if is_selected { "is-selected" },
                        onclick: move |_| {
                            let only_this = *selected.peek() == [id];
                            batch.set(false);
                            selected.set(if only_this { Vec::new() } else { vec![id] });
                        },
                        td { class: "check-col",
                            input {
                                r#type: "checkbox",
                                aria_label: "Select {row.name}",
                                checked: is_selected,
                                // Toggle without the row's "select only this".
                                onclick: move |e| e.stop_propagation(),
                                onchange: move |e| {
                                    batch.set(true);
                                    if e.checked() {
                                        if !selected.peek().contains(&id) {
                                            selected.write().push(id);
                                        }
                                    } else {
                                        selected.write().retain(|s| *s != id);
                                    }
                                },
                            }
                        }
                        td { class: "num muted", "{id}" }
                        th { class: "name", scope: "row",
                            // Keyboard access; the click bubbles to the row.
                            button {
                                class: "row-pick",
                                r#type: "button",
                                title: "Select only {row.name}",
                                aria_pressed: if is_selected { "true" } else { "false" },
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
    }
}

/// Datasets table column the rows are sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Id,
    Name,
    Task,
    Images,
    Labels,
    Classes,
    Format,
    Size,
}

impl SortKey {
    /// In column order.
    const ALL: [SortKey; 8] =
        [SortKey::Id, SortKey::Name, SortKey::Task, SortKey::Images, SortKey::Labels, SortKey::Classes, SortKey::Format, SortKey::Size];

    fn label(self) -> &'static str {
        match self {
            SortKey::Id => "#",
            SortKey::Name => "Name",
            SortKey::Task => "Task",
            SortKey::Images => "Images",
            SortKey::Labels => "Labels",
            SortKey::Classes => "Classes",
            SortKey::Format => "Format",
            SortKey::Size => "Size",
        }
    }

    fn numeric(self) -> bool {
        matches!(self, SortKey::Id | SortKey::Images | SortKey::Labels | SortKey::Classes | SortKey::Size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Sort {
    key: SortKey,
    ascending: bool,
}

impl Sort {
    /// Sorts `rows` by the key; equal rows stay in id order.
    fn apply(self, rows: &mut [DatasetRow]) {
        let text = |r: &DatasetRow| match self.key {
            SortKey::Name => r.name.to_lowercase(),
            SortKey::Task => short_task(&r.task).to_lowercase(),
            SortKey::Format => r.format.to_lowercase(),
            _ => String::new(),
        };
        let number = |r: &DatasetRow| match self.key {
            SortKey::Id => u64::from(r.id),
            SortKey::Images => u64::from(r.images),
            SortKey::Labels => u64::from(r.labels),
            SortKey::Classes => r.classes as u64,
            SortKey::Size => r.size_bytes,
            _ => 0,
        };
        rows.sort_by(|a, b| {
            let order = if self.key.numeric() { number(a).cmp(&number(b)) } else { text(a).cmp(&text(b)) };
            let order = if self.ascending { order } else { order.reverse() };
            order.then(a.id.cmp(&b.id))
        });
    }

    /// The sort after a click on `key`'s header: the other direction for the
    /// current column; a new column starts A to Z for text and largest first
    /// for counts (ids lowest first).
    fn clicked(self, key: SortKey) -> Sort {
        if key == self.key {
            Sort { key, ascending: !self.ascending }
        } else {
            Sort { key, ascending: !key.numeric() || key == SortKey::Id }
        }
    }
}

/// A column header that sorts the table by its column.
#[component]
fn SortHeader(column: SortKey, sort: Signal<Sort>) -> Element {
    let key = column;
    let current = sort();
    let active = current.key == key;
    let aria_sort = match (active, current.ascending) {
        (false, _) => "none",
        (true, true) => "ascending",
        (true, false) => "descending",
    };
    let arrow = match (active, current.ascending) {
        (false, _) => "",
        (true, true) => "▲",
        (true, false) => "▼",
    };
    let class = if key.numeric() { "num" } else { "" };

    rsx! {
        th { class, scope: "col", aria_sort,
            button {
                class: if active { "sort-button is-active" } else { "sort-button" },
                r#type: "button",
                title: "Sort by {key.label()}",
                onclick: move |_| sort.set(sort().clicked(key)),
                if key == SortKey::Id {
                    abbr { title: "ID", "#" }
                } else {
                    "{key.label()}"
                }
                span { class: "sort-arrow", "aria-hidden": "true", "{arrow}" }
            }
        }
    }
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
            size_bytes: 0,
        }
    }

    #[test]
    fn sorts_rows() {
        let mut rows = vec![row(1, "b", 5), row(2, "A", 9), row(3, "c", 5)];
        let ids = |rows: &[DatasetRow]| rows.iter().map(|r| r.id).collect::<Vec<_>>();

        Sort { key: SortKey::Name, ascending: true }.apply(&mut rows);
        assert_eq!(ids(&rows), [2, 1, 3], "ignores case");
        Sort { key: SortKey::Images, ascending: false }.apply(&mut rows);
        assert_eq!(ids(&rows), [2, 1, 3], "ties stay in id order");
        Sort { key: SortKey::Id, ascending: false }.apply(&mut rows);
        assert_eq!(ids(&rows), [3, 2, 1]);

        let sort = Sort { key: SortKey::Id, ascending: true };
        assert_eq!(sort.clicked(SortKey::Id), Sort { key: SortKey::Id, ascending: false });
        assert_eq!(sort.clicked(SortKey::Size), Sort { key: SortKey::Size, ascending: false }, "largest first");
        assert_eq!(sort.clicked(SortKey::Name), Sort { key: SortKey::Name, ascending: true });
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
