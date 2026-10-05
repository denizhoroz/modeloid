//! Pieces every table in the side panel shares, so they all work the same:
//! sortable column headers (`SortHeader` + `Sort`), the Select all / None bar
//! with its count (`SelectionBar`), and the row checkbox (`RowCheck`).
//! Rows themselves: click selects only that row (again: clears), the
//! checkbox adds or removes it, the name cell is a `.row-pick` button for the
//! keyboard. See `datasets_panel.rs` and `side_panel.rs` (`ParametersPanel`).

use dioxus::prelude::*;

/// A column a table can be sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Column {
    pub label: &'static str,
    /// Counts sort largest first and align right; text sorts A to Z.
    pub numeric: bool,
    /// The id column: lowest first, shown as "#".
    pub id: bool,
}

impl Column {
    pub const fn text(label: &'static str) -> Self {
        Column { label, numeric: false, id: false }
    }

    pub const fn number(label: &'static str) -> Self {
        Column { label, numeric: true, id: false }
    }

    pub const fn id() -> Self {
        Column { label: "#", numeric: true, id: true }
    }
}

/// Which column (by position) the rows are sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub column: usize,
    pub ascending: bool,
}

impl Sort {
    /// Sorted by the first column (the id), lowest first.
    pub const fn by_id() -> Self {
        Sort { column: 0, ascending: true }
    }

    /// The sort after a click on `column`'s header: the other direction for
    /// the current column; a new column starts A to Z for text and largest
    /// first for counts (ids lowest first).
    pub fn clicked(self, column: usize, def: Column) -> Sort {
        if column == self.column {
            Sort { column, ascending: !self.ascending }
        } else {
            Sort { column, ascending: !def.numeric || def.id }
        }
    }

    /// `order` for ascending, reversed for descending.
    pub fn directed(self, order: std::cmp::Ordering) -> std::cmp::Ordering {
        if self.ascending { order } else { order.reverse() }
    }
}

/// A column header that sorts the table by its column.
#[component]
pub fn SortHeader(column: usize, def: Column, sort: Signal<Sort>) -> Element {
    let current = sort();
    let active = current.column == column;
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

    rsx! {
        th { class: if def.numeric { "num" } else { "" }, scope: "col", aria_sort,
            button {
                class: if active { "sort-button is-active" } else { "sort-button" },
                r#type: "button",
                title: if def.id { "Sort by ID" } else { "Sort by {def.label}" },
                onclick: move |_| sort.set(sort().clicked(column, def)),
                if def.id {
                    abbr { title: "ID", "#" }
                } else {
                    "{def.label}"
                }
                span { class: "sort-arrow", "aria-hidden": "true", "{arrow}" }
            }
        }
    }
}

/// "Select all", "None", a red "Remove" and "N selected", above a table
/// with checkboxes. Remove asks first, in a small popup under the button
/// (so the table doesn't move); `on_remove` then deletes the selected rows.
#[component]
pub fn SelectionBar(
    /// What the rows are, for screen readers, e.g. "Dataset selection".
    #[props(into)]
    label: String,
    selected: usize,
    total: usize,
    on_all: EventHandler<()>,
    on_none: EventHandler<()>,
    /// What one row is, for the question, e.g. "dataset" ("Delete 2 datasets?").
    #[props(into)]
    noun: String,
    /// What deleting does, e.g. "Their folders are deleted from the workspace."
    #[props(into)]
    remove_note: String,
    on_remove: EventHandler<()>,
    /// Something is in the way (e.g. a job runs); Remove is disabled.
    #[props(default)]
    busy: bool,
) -> Element {
    let mut confirming = use_signal(|| false);
    // A changed selection has to be confirmed again.
    use_effect(use_reactive((&selected,), move |_| confirming.set(false)));
    let what = if selected == 1 { format!("1 {noun}") } else { format!("{selected} {noun}s") };

    rsx! {
        div { class: "panel-selection", role: "group", aria_label: "{label}",
            button {
                class: "button button-ghost button-sm",
                r#type: "button",
                disabled: selected == total,
                onclick: move |_| on_all.call(()),
                "Select all"
            }
            button {
                class: "button button-ghost button-sm",
                r#type: "button",
                disabled: selected == 0,
                onclick: move |_| on_none.call(()),
                "None"
            }
            div {
                class: "remove-anchor",
                onkeydown: move |e| {
                    if e.key() == Key::Escape {
                        confirming.set(false);
                    }
                },
                button {
                    class: "button button-danger button-sm",
                    r#type: "button",
                    disabled: selected == 0 || busy,
                    aria_haspopup: "dialog",
                    aria_expanded: if confirming() { "true" } else { "false" },
                    onclick: move |_| confirming.toggle(),
                    "Remove"
                }
                if confirming() && selected > 0 {
                    // A click anywhere else cancels.
                    div { class: "panel-confirm-backdrop", onclick: move |_| confirming.set(false) }
                    div { class: "panel-confirm", role: "alertdialog", aria_label: "Delete {what}?",
                        span { strong { "Delete {what}? " } "{remove_note} This can't be undone." }
                        div { class: "panel-confirm-actions",
                            button {
                                class: "button button-danger button-sm",
                                r#type: "button",
                                disabled: busy,
                                onclick: move |_| {
                                    confirming.set(false);
                                    on_remove.call(());
                                },
                                "Delete"
                            }
                            button { class: "button button-secondary button-sm", r#type: "button", onclick: move |_| confirming.set(false), "Cancel" }
                        }
                    }
                }
            }
            span { class: "section-card-meta", aria_live: "polite",
                if selected == 0 { "Nothing selected" } else { "{selected} selected" }
            }
        }
    }
}

/// The checkbox cell of a row: adds it to or removes it from the selection
/// without the row click's "select only this".
#[component]
pub fn RowCheck(#[props(into)] name: String, checked: bool, on_change: EventHandler<bool>) -> Element {
    rsx! {
        td { class: "check-col",
            input {
                r#type: "checkbox",
                aria_label: "Select {name}",
                checked,
                onclick: move |e| e.stop_propagation(),
                onchange: move |e| on_change.call(e.checked()),
            }
        }
    }
}

/// The first header cell of a table with checkboxes.
#[component]
pub fn CheckHeader() -> Element {
    rsx! {
        th { class: "check-col", scope: "col", span { class: "visually-hidden", "Selected" } }
    }
}

/// Replaces a selection with `change(current)`.
pub fn update<T: Clone + 'static>(mut selection: Signal<Vec<T>>, change: impl FnOnce(&[T]) -> Vec<T>) {
    let next = change(&selection.peek());
    selection.set(next);
}

/// `selected` after a checkbox change: `key` added at the end or removed.
pub fn toggled<T: PartialEq + Clone>(selected: &[T], key: &T, on: bool) -> Vec<T> {
    let mut next: Vec<T> = selected.iter().filter(|k| *k != key).cloned().collect();
    if on {
        next.push(key.clone());
    }
    next
}

/// `selected` after a row click: only that row, or nothing when it was already the only one.
pub fn clicked<T: PartialEq + Clone>(selected: &[T], key: &T) -> Vec<T> {
    if selected.len() == 1 && selected[0] == *key { Vec::new() } else { vec![key.clone()] }
}

/// `selected` after "Select all": the current order, then the rest as listed.
pub fn all<T: PartialEq + Clone>(selected: &[T], listed: &[T]) -> Vec<T> {
    let mut next = selected.to_vec();
    next.extend(listed.iter().filter(|k| !selected.contains(k)).cloned());
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorting_clicks() {
        let (id, name, size) = (Column::id(), Column::text("Name"), Column::number("Size"));
        let sort = Sort::by_id();
        assert_eq!(sort.clicked(0, id), Sort { column: 0, ascending: false });
        assert_eq!(sort.clicked(2, size), Sort { column: 2, ascending: false }, "largest first");
        assert_eq!(sort.clicked(1, name), Sort { column: 1, ascending: true });
    }

    #[test]
    fn selection_changes() {
        assert_eq!(toggled(&[1, 2], &3, true), [1, 2, 3]);
        assert_eq!(toggled(&[1, 2], &1, false), [2]);
        assert_eq!(clicked(&[1, 2], &2), [2]);
        assert!(clicked(&[2], &2).is_empty());
        assert_eq!(all(&[3], &[1, 2, 3]), [3, 1, 2]);
    }
}
