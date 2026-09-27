use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::workspace::{create_workspace, pick_folder, validate_workspace_name};
use crate::components::{Icon, IconName};
use crate::routes::Route;
use crate::state::use_workspace;

/// Form for a new workspace: name, description and the folder to create it in.
#[component]
pub fn CreateWorkspace() -> Element {
    let mut name = use_signal(String::new);
    let mut description = use_signal(String::new);
    let mut parent = use_signal(|| None::<String>);

    // Errors show once a field was left, or after a submit attempt.
    let mut name_touched = use_signal(|| false);
    let mut submitted = use_signal(|| false);

    let mut picking = use_signal(|| false);
    let mut submitting = use_signal(|| false);
    let mut form_error = use_signal(|| None::<String>);

    let mut workspace = use_workspace();
    let navigator = use_navigator();

    let name_error = use_memo(move || validate_workspace_name(&name.read()).err());
    let show_name_error = name_touched() || submitted();
    let show_folder_error = submitted() && parent.read().is_none();

    // Full path the workspace folder will get, shown under the location.
    let target = use_memo(move || {
        let parent = parent.read();
        let parent = parent.as_deref()?;
        let name = name.read();
        let name = name.trim();
        let sep = if parent.contains('\\') { '\\' } else { '/' };
        let parent = parent.trim_end_matches(['/', '\\']);
        Some(if name.is_empty() { format!("{parent}{sep}…") } else { format!("{parent}{sep}{name}") })
    });

    let choose_folder = move |_| async move {
        picking.set(true);
        match pick_folder("Choose where to create the workspace".into()).await {
            Ok(Some(dir)) => parent.set(Some(dir)),
            Ok(None) => {} // dialog cancelled
            Err(e) => form_error.set(Some(error_message(&e))),
        }
        picking.set(false);
    };

    let submit = move |e: FormEvent| async move {
        e.prevent_default();
        submitted.set(true);
        form_error.set(None);
        let Some(dir) = parent() else { return };
        if name_error().is_some() || submitting() {
            return;
        }

        submitting.set(true);
        match create_workspace(name(), description(), dir).await {
            Ok(info) => {
                workspace.set(Some(info));
                navigator.push(Route::Home {});
            }
            Err(e) => form_error.set(Some(error_message(&e))),
        }
        submitting.set(false);
    };

    rsx! {
        document::Title { "Create Workspace · Modeloid" }
        div { class: "page",
            p { class: "page-eyebrow", "Workspace" }
            h1 { "Create Workspace" }
            p { class: "page-lead", "A folder with this name is created in the location you choose." }

            form { class: "form", novalidate: true, onsubmit: submit,
                div { class: "field",
                    label { class: "field-label", r#for: "ws-name",
                        "Name"
                        span { class: "field-required", aria_hidden: "true", " *" }
                    }
                    input {
                        id: "ws-name",
                        class: "input",
                        r#type: "text",
                        required: true,
                        autocomplete: "off",
                        spellcheck: "false",
                        placeholder: "e.g. traffic-signs",
                        aria_invalid: if show_name_error && name_error().is_some() { "true" },
                        aria_describedby: "ws-name-help",
                        value: "{name}",
                        oninput: move |e| name.set(e.value()),
                        onblur: move |_| name_touched.set(true),
                    }
                    match name_error() {
                        Some(message) if show_name_error => rsx! {
                            p { id: "ws-name-help", class: "field-error", role: "alert", "{message}" }
                        },
                        _ => rsx! {
                            p { id: "ws-name-help", class: "field-hint", "Also used as the folder name." }
                        },
                    }
                }

                div { class: "field",
                    label { class: "field-label", r#for: "ws-description",
                        "Description"
                        span { class: "field-optional", " (optional)" }
                    }
                    textarea {
                        id: "ws-description",
                        class: "input textarea",
                        rows: "3",
                        placeholder: "What is this workspace for?",
                        value: "{description}",
                        oninput: move |e| description.set(e.value()),
                    }
                }

                div { class: "field",
                    span { id: "ws-location-label", class: "field-label",
                        "Location"
                        span { class: "field-required", aria_hidden: "true", " *" }
                    }
                    div { class: "folder-picker",
                        button {
                            class: "button button-secondary",
                            r#type: "button",
                            aria_describedby: "ws-location-label ws-location-help",
                            disabled: picking() || submitting(),
                            aria_busy: if picking() { "true" },
                            onclick: choose_folder,
                            Icon { name: IconName::FolderOpen, size: 14 }
                            span { if picking() { "Opening…" } else if parent.read().is_some() { "Change folder" } else { "Choose folder" } }
                        }
                        match parent() {
                            Some(dir) => rsx! { span { class: "folder-path", title: "{dir}", "{dir}" } },
                            None => rsx! { span { class: "folder-path is-empty", "No folder chosen" } },
                        }
                    }
                    if show_folder_error {
                        p { id: "ws-location-help", class: "field-error", role: "alert", "Choose a folder." }
                    } else if let Some(path) = target() {
                        p { id: "ws-location-help", class: "field-hint",
                            "Will be created at "
                            code { "{path}" }
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
                        disabled: submitting(),
                        aria_busy: if submitting() { "true" },
                        if submitting() { "Creating…" } else { "Create workspace" }
                    }
                    Link { class: "button button-ghost", to: Route::Home {}, "Cancel" }
                }
            }
        }
    }
}
