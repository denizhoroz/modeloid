use dioxus::prelude::*;

use crate::api::datasets::{list_datasets, short_task};
use crate::api::error_message;
use crate::state::{use_dataset_pick, use_datasets_version, use_workspace, DatasetPick};
use crate::util::format_count;

/// Choose the input dataset(s) of a tool. Clicking a row in the datasets panel
/// also picks it: added in `multiple` mode, replacing the choice otherwise.
/// `selected` keeps the order datasets were picked in.
#[component]
pub fn DatasetPicker(
    /// Section title, e.g. "Input datasets".
    #[props(into)]
    title: String,
    selected: Signal<Vec<u32>>,
    /// Allow several inputs (checkboxes) instead of one (radio buttons).
    #[props(default)]
    multiple: bool,
    /// Validation message from the parent form.
    #[props(default)]
    error: Option<String>,
    #[props(default)] disabled: bool,
) -> Element {
    let mut selected = selected;
    let workspace = use_workspace();
    let version = use_datasets_version();
    let DatasetPick { mut pickers, mut picked } = use_dataset_pick();
    let id = use_hook(|| format!("dataset-picker-{}", title.to_lowercase().replace(' ', "-")));

    // Let the panel know a picker is listening while this one is on screen.
    use_hook(move || *pickers.write() += 1);
    use_drop(move || *pickers.write() -= 1);

    use_effect(move || {
        let Some(dataset) = picked() else { return };
        picked.set(None);
        if disabled {
            return;
        }
        if !multiple {
            selected.set(vec![dataset]);
        } else if !selected.peek().contains(&dataset) {
            selected.write().push(dataset);
        }
    });

    let datasets = use_resource(move || async move {
        let _ = (workspace.read().as_ref().map(|w| w.path.clone()), version());
        list_datasets().await
    });

    let mut toggle = move |dataset: u32| {
        if !multiple {
            selected.set(vec![dataset]);
        } else if selected.peek().contains(&dataset) {
            selected.write().retain(|d| *d != dataset);
        } else {
            selected.write().push(dataset);
        }
    };

    let count = selected.read().len();
    let hint = if multiple { "Pick one or more. You can also click rows in the datasets panel." } else { "You can also click a row in the datasets panel." };

    rsx! {
        fieldset { class: "section-card", disabled,
            div { class: "section-card-header",
                legend { class: "section-card-title", "{title}" }
                if multiple && count > 0 {
                    span { class: "section-card-meta", "{count} selected" }
                }
            }
            p { class: "field-hint", "{hint}" }

            match &*datasets.read() {
                None => rsx! { p { class: "field-hint", "Loading datasets…" } },
                Some(Err(e)) => rsx! { p { class: "field-error", role: "alert", "{error_message(e)}" } },
                Some(Ok(rows)) if rows.is_empty() => rsx! {
                    p { class: "choice-detail is-empty", "No datasets in this workspace yet." }
                },
                Some(Ok(rows)) => rsx! {
                    div { class: "choice-list choice-list-scroll",
                        for row in rows.clone() {
                            {
                                let checked = selected.read().contains(&row.id);
                                // Pick order, shown when several inputs matter (e.g. merge order).
                                let order = selected.read().iter().position(|d| *d == row.id).filter(|_| multiple && count > 1);
                                rsx! {
                                    label { key: "{row.id}", class: if checked { "choice is-selected" } else { "choice" },
                                        input {
                                            r#type: if multiple { "checkbox" } else { "radio" },
                                            name: "{id}",
                                            checked,
                                            onchange: move |_| toggle(row.id),
                                        }
                                        span { class: "choice-body",
                                            span { class: "choice-label",
                                                if let Some(i) = order {
                                                    span { class: "choice-order", "{i + 1}" }
                                                }
                                                "{row.name}"
                                            }
                                            span { class: "choice-detail",
                                                "#{row.id} · {short_task(&row.task)} · {format_count(row.images)} images · {classes_label(row.classes)} · {row.format}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }

            if let Some(message) = error {
                p { class: "field-error", role: "alert", "{message}" }
            }
        }
    }
}

fn classes_label(n: usize) -> String {
    if n == 1 { "1 class".into() } else { format!("{n} classes") }
}
