use dioxus::prelude::*;

use crate::api::download::{invalid_links, parse_links};
use crate::util::plural;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LinkMode {
    Single,
    Batch,
}

/// Link entry with a Single / Batch toggle. Batch takes one link per line.
#[component]
pub fn LinksSection(
    mode: Signal<LinkMode>,
    single: Signal<String>,
    batch: Signal<String>,
    /// Show validation errors (after a submit attempt).
    show_errors: bool,
) -> Element {
    let mut mode = mode;
    let mut single = single;
    let mut batch = batch;

    // Carry what was typed across when switching modes.
    let mut switch_to = move |next: LinkMode| {
        if mode() == next {
            return;
        }
        match next {
            LinkMode::Batch if batch.read().trim().is_empty() => batch.set(single().trim().to_string()),
            LinkMode::Single if single.read().trim().is_empty() => {
                single.set(parse_links(&batch.read()).into_iter().next().unwrap_or_default())
            }
            _ => {}
        }
        mode.set(next);
    };

    let batch_links = parse_links(&batch.read());
    let batch_invalid = invalid_links(&batch.read());
    let single_value = single.read().trim().to_string();
    let single_error = if single_value.is_empty() {
        Some("Paste a dataset link.".to_string())
    } else if !invalid_links(&single_value).is_empty() {
        Some("That isn't a link. It should start with https://".to_string())
    } else {
        None
    };
    let batch_error = if batch_links.is_empty() {
        Some("Paste at least one link.".to_string())
    } else {
        batch_invalid.first().map(|(line, _)| {
            let more = batch_invalid.len() - 1;
            if more == 0 {
                format!("Line {line} isn't a link.")
            } else {
                format!("Line {line} and {} aren't links.", plural(more, "other line"))
            }
        })
    };

    rsx! {
        section { class: "section-card", aria_labelledby: "download-links-title",
            div { class: "section-card-header",
                h2 { id: "download-links-title", class: "section-card-title", "Links" }
                div { class: "segmented", role: "radiogroup", aria_label: "Link input mode",
                    for (value, label) in [(LinkMode::Single, "Single"), (LinkMode::Batch, "Batch")] {
                        button {
                            key: "{label}",
                            class: if mode() == value { "segmented-option is-selected" } else { "segmented-option" },
                            r#type: "button",
                            role: "radio",
                            aria_checked: if mode() == value { "true" } else { "false" },
                            onclick: move |_| switch_to(value),
                            "{label}"
                        }
                    }
                }
            }

            match mode() {
                LinkMode::Single => rsx! {
                    div { class: "field",
                        label { class: "field-label", r#for: "download-link", "Dataset link" }
                        input {
                            id: "download-link",
                            class: "input",
                            r#type: "url",
                            spellcheck: "false",
                            autocomplete: "off",
                            placeholder: "https://universe.roboflow.com/<workspace>/<project>/dataset/<version>",
                            aria_invalid: if show_errors && single_error.is_some() { "true" },
                            aria_describedby: "download-link-help",
                            value: "{single}",
                            oninput: move |e| single.set(e.value()),
                        }
                        match &single_error {
                            Some(message) if show_errors => rsx! {
                                p { id: "download-link-help", class: "field-error", role: "alert", "{message}" }
                            },
                            _ => rsx! {
                                p { id: "download-link-help", class: "field-hint",
                                    "Roboflow Universe or app link. Without a version, the latest one is used."
                                }
                            },
                        }
                    }
                },
                LinkMode::Batch => rsx! {
                    div { class: "field",
                        div { class: "field-row",
                            label { class: "field-label", r#for: "download-links", "Dataset links" }
                            span { class: "field-count", "{plural(batch_links.len(), \"link\")}" }
                        }
                        textarea {
                            id: "download-links",
                            class: "input textarea textarea-code",
                            rows: "6",
                            spellcheck: "false",
                            wrap: "off",
                            placeholder: "One link per line",
                            aria_invalid: if show_errors && batch_error.is_some() { "true" },
                            aria_describedby: "download-links-help",
                            value: "{batch}",
                            oninput: move |e| batch.set(e.value()),
                        }
                        match &batch_error {
                            Some(message) if show_errors => rsx! {
                                p { id: "download-links-help", class: "field-error", role: "alert", "{message}" }
                            },
                            _ => rsx! {
                                p { id: "download-links-help", class: "field-hint",
                                    "One link per line. They download one after another."
                                }
                            },
                        }
                    }
                },
            }
        }
    }
}

/// Links to download for the current mode, or why they can't be used.
pub fn collect_links(mode: LinkMode, single: &str, batch: &str) -> Result<Vec<String>, ()> {
    let links = match mode {
        LinkMode::Single => parse_links(single).into_iter().take(1).collect::<Vec<_>>(),
        LinkMode::Batch => parse_links(batch),
    };
    let text = links.join("\n");
    if links.is_empty() || !invalid_links(&text).is_empty() {
        Err(())
    } else {
        Ok(links)
    }
}
