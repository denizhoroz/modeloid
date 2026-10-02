use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::workspace::{parse_seed, update_workspace, WorkspaceInfo};
use crate::components::InfoTip;
use crate::state::use_workspace;

/// Default label format choices: stored value and name. Empty = no default.
const FORMATS: [(&str, &str); 4] = [("", "No default (follow each dataset)"), ("yolo", "YOLO"), ("coco", "COCO JSON"), ("voc", "Pascal VOC")];

/// The workspace options shared by the create form and Settings: seed and
/// default label format. `seed` holds the typed text (checked with `parse_seed`).
#[component]
pub fn WorkspaceOptions(
    seed: Signal<String>,
    format: Signal<String>,
    /// Show the seed's validation message (after a submit attempt).
    #[props(default)]
    show_errors: bool,
) -> Element {
    let (mut seed, mut format) = (seed, format);
    let seed_error = parse_seed(&seed.read()).err().filter(|_| show_errors);

    rsx! {
        div { class: "field",
            label { class: "field-label", r#for: "ws-seed",
                "Seed "
                InfoTip { label: "About the seed", text: "Random choices (splits, subsets, cuts) start from this number, so the same settings give the same result again." }
            }
            input {
                id: "ws-seed",
                class: "input input-narrow",
                r#type: "number",
                min: "0",
                aria_invalid: if seed_error.is_some() { "true" },
                aria_describedby: "ws-seed-help",
                value: "{seed}",
                oninput: move |e| seed.set(e.value()),
            }
            match &seed_error {
                Some(message) => rsx! { p { id: "ws-seed-help", class: "field-error", role: "alert", "{message}" } },
                None => rsx! {},
            }
        }
        div { class: "field",
            label { class: "field-label", r#for: "ws-format",
                "Default label format "
                InfoTip { label: "About the default format", text: "The format Convert and Classes & Merge start on. You can still choose another one each time." }
            }
            select {
                id: "ws-format",
                class: "input select",
                value: "{format}",
                onchange: move |e| format.set(e.value()),
                for (value, label) in FORMATS {
                    option { key: "{value}", value, selected: format() == value, "{label}" }
                }
            }
        }
    }
}

/// Settings > Workspace: description, seed and default label format of the
/// open workspace (`info`), saved to its workspace.toml.
#[component]
pub fn WorkspaceSettings(info: WorkspaceInfo) -> Element {
    let mut workspace = use_workspace();
    let mut description = use_signal(|| info.description.clone());
    let seed = use_signal(|| info.seed.to_string());
    let format = use_signal(|| info.default_format.clone());

    let mut submitted = use_signal(|| false);
    let mut saving = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);

    // What is typed differs from what is stored.
    let changed = *description.read().trim() != info.description || *seed.read().trim() != info.seed.to_string() || *format.read() != info.default_format;

    let save = move |e: FormEvent| async move {
        e.prevent_default();
        submitted.set(true);
        error.set(None);
        let Ok(seed) = parse_seed(&seed.read()) else { return };
        saving.set(true);
        match update_workspace(description(), seed, format()).await {
            Ok(updated) => {
                workspace.set(Some(updated));
                submitted.set(false);
            }
            Err(e) => error.set(Some(error_message(&e))),
        }
        saving.set(false);
    };

    rsx! {
        form { class: "settings-card form", novalidate: true, onsubmit: save,
            div { class: "field",
                label { class: "field-label", r#for: "ws-settings-description", "Description" }
                textarea {
                    id: "ws-settings-description",
                    class: "input textarea",
                    rows: "3",
                    placeholder: "What is this workspace for?",
                    value: "{description}",
                    oninput: move |e| description.set(e.value()),
                }
            }
            WorkspaceOptions { seed, format, show_errors: submitted() }

            if let Some(message) = error() {
                p { class: "form-error", role: "alert", "{message}" }
            }
            div { class: "form-actions",
                button {
                    class: "button",
                    r#type: "submit",
                    disabled: saving() || !changed,
                    aria_busy: if saving() { "true" },
                    if saving() { "Saving…" } else { "Save" }
                }
                span { class: "form-actions-note", role: "status",
                    if changed { "Unsaved changes" } else { "Saved" }
                }
            }
        }
    }
}
