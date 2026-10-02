//! Create tab: an empty dataset (name, description, type, label format) that
//! images are added to in the Edit tab.

use dioxus::prelude::*;

use crate::api::datasets::validate_dataset_name;
use crate::api::edit::{create_dataset, NewDataset, CREATE_FORMATS, CREATE_TYPES};
use crate::api::error_message;
use crate::components::InfoTip;
use crate::state::{use_datasets_version, use_selected_datasets};

#[component]
pub fn CreateForm(datasets_path: String, default_format: String, on_add_images: EventHandler<()>) -> Element {
    let mut name = use_signal(String::new);
    let mut description = use_signal(String::new);
    let mut dataset_type = use_signal(|| CREATE_TYPES[0].0.to_string());
    let start_format = if CREATE_FORMATS.iter().any(|(f, _)| *f == default_format) { default_format } else { CREATE_FORMATS[0].0.to_string() };
    let mut format = use_signal(|| start_format);

    let mut submitted = use_signal(|| false);
    let mut creating = use_signal(|| false);
    let mut form_error = use_signal(|| None::<String>);
    // Name of the dataset created last, for the note under the form.
    let mut created = use_signal(|| None::<String>);

    let mut selected = use_selected_datasets();
    let mut datasets_version = use_datasets_version();

    let name_error = if submitted() || !name.read().is_empty() { validate_dataset_name(&name.read()).err() } else { None };
    let sep = if datasets_path.contains('\\') { "\\" } else { "/" };

    let submit = move |e: FormEvent| async move {
        e.prevent_default();
        submitted.set(true);
        form_error.set(None);
        if validate_dataset_name(&name.read()).is_err() {
            return;
        }
        let new = NewDataset {
            name: name(),
            description: description(),
            dataset_type: dataset_type(),
            annotation_format: format(),
        };
        creating.set(true);
        match create_dataset(new).await {
            Ok(id) => {
                created.set(Some(name()));
                name.set(String::new());
                description.set(String::new());
                submitted.set(false);
                // Show it in the table and make it the one Edit works on.
                *datasets_version.write() += 1;
                selected.set(vec![id]);
            }
            Err(e) => form_error.set(Some(error_message(&e))),
        }
        creating.set(false);
    };

    rsx! {
        form { class: "section-stack", novalidate: true, onsubmit: submit,
            section { class: "section-card", aria_labelledby: "create-details-title",
                h2 { id: "create-details-title", class: "section-card-title",
                    "Details "
                    InfoTip { label: "About new datasets", text: "The dataset starts empty: 0 images, 0 labels. Add images in the Edit tab." }
                }
                div { class: "field",
                    label { class: "field-label", r#for: "create-name", "Name" }
                    input {
                        id: "create-name",
                        class: "input",
                        r#type: "text",
                        spellcheck: "false",
                        autocomplete: "off",
                        placeholder: "e.g. parking-lot-cars",
                        value: "{name}",
                        aria_invalid: if name_error.is_some() { "true" },
                        aria_describedby: "create-name-help",
                        oninput: move |e| name.set(e.value()),
                    }
                    match &name_error {
                        Some(message) => rsx! { p { id: "create-name-help", class: "field-error", role: "alert", "{message}" } },
                        None if name.read().is_empty() => rsx! {
                            p { id: "create-name-help", class: "field-hint", "Also the name of its folder in "
                                code { "{datasets_path}" }
                            }
                        },
                        None => rsx! {
                            p { id: "create-name-help", class: "field-hint", "Saved as "
                                code { "{datasets_path}{sep}{name}" }
                            }
                        },
                    }
                }
                div { class: "field",
                    label { class: "field-label", r#for: "create-description",
                        "Description"
                        span { class: "field-optional", " (optional)" }
                    }
                    textarea {
                        id: "create-description",
                        class: "input textarea",
                        rows: "3",
                        placeholder: "What's in it, where the images come from…",
                        value: "{description}",
                        oninput: move |e| description.set(e.value()),
                    }
                }
                div { class: "field",
                    label { class: "field-label", r#for: "create-type", "Dataset type" }
                    select {
                        id: "create-type",
                        class: "input select",
                        value: "{dataset_type}",
                        onchange: move |e| dataset_type.set(e.value()),
                        for (value, label) in CREATE_TYPES {
                            option { key: "{value}", value: *value, selected: dataset_type() == *value, "{label}" }
                        }
                    }
                }
                div { class: "field",
                    label { class: "field-label", r#for: "create-format", "Label format" }
                    select {
                        id: "create-format",
                        class: "input select",
                        value: "{format}",
                        onchange: move |e| format.set(e.value()),
                        for (value, label) in CREATE_FORMATS {
                            option { key: "{value}", value: *value, selected: format() == *value, "{label}" }
                        }
                    }
                }
            }

            if let Some(message) = form_error() {
                p { class: "form-error", role: "alert", "{message}" }
            }

            div { class: "form-actions",
                button {
                    class: "button",
                    r#type: "submit",
                    disabled: creating(),
                    aria_busy: if creating() { "true" },
                    if creating() { "Adding…" } else { "Add dataset" }
                }
            }
        }

        if let Some(done) = created() {
            div { class: "notice", role: "status",
                p { "Added '{done}' to the workspace. It's selected in the Datasets table." }
                div { class: "notice-actions",
                    button {
                        class: "button button-secondary",
                        r#type: "button",
                        onclick: move |_| on_add_images.call(()),
                        "Add images"
                    }
                }
            }
        }
    }
}
