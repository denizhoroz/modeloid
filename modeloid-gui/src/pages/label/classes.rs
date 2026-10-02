//! The Classes panel right of the image: the dataset's classes with their
//! label counts, like every other table (checkboxes, Select all / None /
//! Remove, `#`, sortable headers, count and reload). A row click makes that
//! class the one new boxes get (and gives it to the selected boxes).

use dioxus::prelude::*;

use super::{class_color, use_editor};
use crate::api::error_message;
use crate::api::label::{add_label_class, remove_label_classes, rename_label_class, validate_class_name};
use crate::components::table::{self, CheckHeader, Column, RowCheck, SelectionBar, Sort, SortHeader};
use crate::components::{Icon, IconName};
use crate::util::{format_count, plural};

const COLUMNS: [Column; 4] = [Column::id(), Column::text("Name"), Column::number("Image"), Column::number("Total")];

#[derive(Debug, Clone, PartialEq)]
struct ClassRow {
    /// Index in the class list; shown 1-based (keys 1–9 pick the first nine).
    class: usize,
    name: String,
    on_image: u32,
    total: u32,
}

fn sort_rows(sort: Sort, rows: &mut [ClassRow]) {
    rows.sort_by(|a, b| {
        let order = match sort.column {
            1 => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            2 => a.on_image.cmp(&b.on_image),
            3 => a.total.cmp(&b.total),
            _ => a.class.cmp(&b.class),
        };
        sort.directed(order).then(a.class.cmp(&b.class))
    });
}

#[component]
pub fn ClassesPanel() -> Element {
    let mut editor = use_editor();
    let sort = use_signal(Sort::by_id);
    let mut search = use_signal(String::new);
    let mut checked = use_signal(Vec::<usize>::new);
    let mut new_name = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    // The class being renamed and the name typed so far.
    let mut renaming = use_signal(|| None::<(usize, String)>);

    let session = editor.session.read().clone();
    let classes = session.as_ref().map(|s| s.classes.clone()).unwrap_or_default();
    let totals = session.as_ref().map(|s| s.totals.clone()).unwrap_or_default();
    let read_only = editor.read_only();
    let active = (editor.active_class)();

    // Selection only holds classes that exist.
    use_effect(move || {
        let count = editor.session.read().as_ref().map_or(0, |s| s.classes.len());
        if checked.peek().iter().any(|&c| c >= count) {
            table::update(checked, |s| s.iter().copied().filter(|&c| c < count).collect());
        }
    });

    let mut on_image = vec![0u32; classes.len()];
    for o in editor.objects.read().iter() {
        if let Some(n) = on_image.get_mut(o.class) {
            *n += 1;
        }
    }
    let query = search().trim().to_lowercase();
    let mut rows: Vec<ClassRow> = classes
        .iter()
        .enumerate()
        .filter(|(_, name)| query.is_empty() || name.to_lowercase().contains(&query))
        .map(|(class, name)| ClassRow { class, name: name.clone(), on_image: on_image[class], total: totals.get(class).copied().unwrap_or(0) })
        .collect();
    sort_rows(sort(), &mut rows);
    let listed: Vec<usize> = rows.iter().map(|r| r.class).collect();
    let selected_count = checked.read().len();
    let removing_labels: u32 = checked.read().iter().filter_map(|&c| totals.get(c)).sum();

    let add = move || async move {
        let name = match validate_class_name(&new_name.peek(), &editor.classes(), None) {
            Ok(name) => name,
            Err(message) => {
                error.set(Some(message));
                return;
            }
        };
        let Some(dataset) = editor.session.peek().as_ref().map(|s| s.dataset.clone()) else { return };
        match add_label_class(editor.id, dataset, name).await {
            Ok(classes) => {
                if let Some(s) = editor.session.write().as_mut() {
                    s.totals.resize(classes.len(), 0);
                    s.classes = classes.clone();
                }
                editor.active_class.set(classes.len().saturating_sub(1));
                new_name.set(String::new());
                error.set(None);
            }
            Err(e) => error.set(Some(error_message(&e))),
        }
    };

    let rename = move |class: usize, name: String| async move {
        renaming.set(None);
        if editor.classes().get(class) == Some(&name.trim().to_string()) {
            return;
        }
        if let Err(message) = validate_class_name(&name, &editor.classes(), Some(class)) {
            error.set(Some(message));
            return;
        }
        let Some(dataset) = editor.session.peek().as_ref().map(|s| s.dataset.clone()) else { return };
        match rename_label_class(editor.id, dataset, class, name).await {
            Ok(classes) => {
                if let Some(s) = editor.session.write().as_mut() {
                    s.classes = classes;
                }
                error.set(None);
            }
            Err(e) => error.set(Some(error_message(&e))),
        }
    };

    rsx! {
        aside { id: "label-classes", class: "datasets-panel label-classes", aria_label: "Classes",
            div { class: "datasets-panel-header",
                h2 { class: "section-card-title label-classes-title", "Classes" }
                span { class: "section-card-meta", "{plural(classes.len(), \"class\")}" }
                button {
                    class: "icon-button icon-button-sm",
                    r#type: "button",
                    aria_label: "Reload classes",
                    title: "Reload: save, then read the labels again",
                    onclick: move |_| async move {
                        if editor.save().await {
                            *editor.session_version.write() += 1;
                            *editor.image_version.write() += 1;
                        }
                    },
                    Icon { name: IconName::Refresh, size: 14, class: if session.is_none() { "is-spinning" } else { "" } }
                }
            }
            div { class: "label-classes-search",
                input {
                    class: "input",
                    r#type: "search",
                    placeholder: "Search classes",
                    aria_label: "Search classes",
                    value: "{search}",
                    oninput: move |e| search.set(e.value()),
                    onkeydown: move |e| e.stop_propagation(),
                }
            }
            if !classes.is_empty() {
                SelectionBar {
                    label: "Class selection",
                    selected: selected_count,
                    total: classes.len(),
                    on_all: move |_| table::update(checked, |s| table::all(s, &listed)),
                    on_none: move |_| table::update(checked, |_| Vec::new()),
                    noun: "class",
                    remove_note: format!("Their {} are deleted from every image, and the classes after them move up.", plural(removing_labels as usize, "label")),
                    busy: read_only,
                    on_remove: move |_| async move {
                        let classes = checked.peek().clone();
                        let Some(dataset) = editor.session.peek().as_ref().map(|s| s.dataset.clone()) else { return };
                        // Ids move, so the open image is saved first and loaded again after.
                        if !editor.save().await {
                            return;
                        }
                        match remove_label_classes(editor.id, dataset, classes).await {
                            Ok(session) => {
                                let count = session.classes.len();
                                editor.session.set(Some(session));
                                if *editor.active_class.peek() >= count {
                                    editor.active_class.set(count.saturating_sub(1));
                                }
                                checked.set(Vec::new());
                                *editor.image_version.write() += 1;
                                error.set(None);
                            }
                            Err(e) => error.set(Some(error_message(&e))),
                        }
                    },
                }
            }
            if let Some(message) = error() {
                p { class: "field-error panel-status", role: "alert", "{message}" }
            }
            div { class: "datasets-panel-body",
                if session.is_none() {
                    p { class: "panel-empty", "Loading…" }
                } else {
                    if classes.is_empty() {
                        p { class: "panel-empty", "No classes yet. Add the first one below; new boxes get the class chosen here." }
                    } else if rows.is_empty() {
                        p { class: "panel-empty", "No class matches “{search}”." }
                    }
                    table { class: "data-table",
                        if !rows.is_empty() {
                            thead {
                                tr {
                                    CheckHeader {}
                                    for (column, def) in COLUMNS.into_iter().enumerate() {
                                        SortHeader { key: "{column}", column, def, sort }
                                    }
                                }
                            }
                        }
                        tbody {
                            for row in rows {
                                {
                                    let class = row.class;
                                    let is_checked = checked.read().contains(&class);
                                    let is_active = class == active;
                                    let editing = renaming().filter(|(c, _)| *c == class).map(|(_, n)| n);
                                    let row_class = match (is_active, is_checked) {
                                        (true, _) => "is-selected",
                                        (false, true) => "is-checked",
                                        _ => "",
                                    };
                                    let hint = if class < 9 { format!(" (key {})", class + 1) } else { String::new() };
                                    let name = row.name.clone();
                                    rsx! {
                                        tr {
                                            key: "{class}",
                                            class: row_class,
                                            onclick: move |_| {
                                                editor.choose_class(class);
                                                table::update(checked, |s| table::clicked(s, &class));
                                            },
                                            RowCheck {
                                                name: row.name.clone(),
                                                checked: is_checked,
                                                on_change: move |on| table::update(checked, |s| table::toggled(s, &class, on)),
                                            }
                                            td { class: "num muted", "{class + 1}" }
                                            th { class: "name", scope: "row",
                                                // The cell stays a table cell (its border runs on); the flex box is inside.
                                                span { class: "class-name-cell",
                                                    span { class: "class-swatch", style: "background: {class_color(class)}", "aria-hidden": "true" }
                                                    if let Some(typed) = editing {
                                                        input {
                                                            class: "input label-rename",
                                                            aria_label: "New name for {row.name}",
                                                            value: "{typed}",
                                                            onmounted: move |e| async move {
                                                                let _ = e.set_focus(true).await;
                                                            },
                                                            onclick: move |e| e.stop_propagation(),
                                                            oninput: move |e| renaming.set(Some((class, e.value()))),
                                                            onkeydown: move |e| {
                                                                e.stop_propagation();
                                                                match e.key() {
                                                                    Key::Enter => {
                                                                        let name = renaming.peek().as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
                                                                        spawn(async move { rename(class, name).await });
                                                                    }
                                                                    Key::Escape => renaming.set(None),
                                                                    _ => {}
                                                                }
                                                            },
                                                            onblur: move |_| {
                                                                if let Some((_, name)) = renaming.peek().clone() {
                                                                    spawn(async move { rename(class, name).await });
                                                                }
                                                            },
                                                        }
                                                    } else {
                                                        button {
                                                            class: "row-pick",
                                                            r#type: "button",
                                                            title: "Use {row.name} for new boxes{hint}. Double-click or F2 to rename.",
                                                            aria_pressed: if is_active { "true" } else { "false" },
                                                            ondoubleclick: move |e| {
                                                                e.stop_propagation();
                                                                if !read_only {
                                                                    renaming.set(Some((class, name.clone())));
                                                                }
                                                            },
                                                            onkeydown: {
                                                                let name = row.name.clone();
                                                                move |e: KeyboardEvent| {
                                                                    if e.key() == Key::F2 && !read_only {
                                                                        e.stop_propagation();
                                                                        renaming.set(Some((class, name.clone())));
                                                                    }
                                                                }
                                                            },
                                                            "{row.name}"
                                                        }
                                                    }
                                                }
                                            }
                                            td { class: "num", "{format_count(row.on_image)}" }
                                            td { class: "num", "{format_count(row.total)}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    form {
                        class: "label-add-class",
                        onsubmit: move |e| async move {
                            e.prevent_default();
                            add().await;
                        },
                        input {
                            class: "input",
                            r#type: "text",
                            placeholder: "New class name",
                            aria_label: "New class name",
                            disabled: read_only,
                            value: "{new_name}",
                            oninput: move |e| new_name.set(e.value()),
                            onkeydown: move |e| e.stop_propagation(),
                        }
                        button { class: "button button-sm", r#type: "submit", disabled: read_only || new_name().trim().is_empty(),
                            Icon { name: IconName::Plus, size: 14 }
                            "Add class"
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
    fn sorts_classes() {
        let row = |class: usize, name: &str, total: u32| ClassRow { class, name: name.into(), on_image: 0, total };
        let mut rows = vec![row(0, "dog", 5), row(1, "Cat", 9), row(2, "bird", 5)];
        sort_rows(Sort { column: 1, ascending: true }, &mut rows);
        assert_eq!(rows.iter().map(|r| r.class).collect::<Vec<_>>(), [2, 1, 0]);
        sort_rows(Sort { column: 3, ascending: false }, &mut rows);
        assert_eq!(rows.iter().map(|r| r.class).collect::<Vec<_>>(), [1, 0, 2], "ties stay in id order");
    }
}
