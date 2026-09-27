use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::workspace::pick_folder;
use crate::components::{Icon, IconName};

/// Which folder the user picked in a `FolderSelect`.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum FolderChoice {
    /// The default folder the page offers (e.g. the workspace's `datasets/`).
    #[default]
    Default,
    /// A folder chosen with the picker; `None` until one is chosen.
    Custom(Option<String>),
}

impl FolderChoice {
    /// `Ok(None)` for the default folder, `Ok(Some(path))` for a custom one.
    pub fn resolve(&self) -> Result<Option<String>, String> {
        match self {
            FolderChoice::Default => Ok(None),
            FolderChoice::Custom(Some(path)) => Ok(Some(path.clone())),
            FolderChoice::Custom(None) => Err("Choose a folder, or use the default.".into()),
        }
    }
}

/// Reusable "where should this go" section: a default folder or a custom one
/// picked with the native folder dialog.
#[component]
pub fn FolderSelect(
    /// Section title, e.g. "Target folder".
    #[props(into)]
    title: String,
    /// Name of the default option, e.g. "Workspace datasets folder".
    #[props(into)]
    default_label: String,
    /// Path of the default folder, shown under its name.
    #[props(into)]
    default_path: String,
    choice: Signal<FolderChoice>,
    /// Title of the native folder dialog.
    #[props(into, default = "Choose a folder".to_string())]
    dialog_title: String,
    /// Validation message from the parent form.
    #[props(default)]
    error: Option<String>,
    #[props(default)] disabled: bool,
) -> Element {
    let mut choice = choice;
    let mut picking = use_signal(|| false);
    let mut pick_error = use_signal(|| None::<String>);
    let id = use_hook(|| format!("folder-select-{}", title.to_lowercase().replace(' ', "-")));

    let choose = move |_| {
        let dialog_title = dialog_title.clone();
        async move {
            picking.set(true);
            pick_error.set(None);
            match pick_folder(dialog_title).await {
                Ok(Some(dir)) => choice.set(FolderChoice::Custom(Some(dir))),
                Ok(None) => {} // dialog cancelled
                Err(e) => pick_error.set(Some(error_message(&e))),
            }
            picking.set(false);
        }
    };

    let is_default = choice() == FolderChoice::Default;
    let custom_path = match choice() {
        FolderChoice::Custom(path) => path,
        FolderChoice::Default => None,
    };
    let message = error.or(pick_error());

    rsx! {
        fieldset { class: "section-card", disabled,
            legend { class: "section-card-title", "{title}" }

            div { class: "choice-list",
                label { class: if is_default { "choice is-selected" } else { "choice" },
                    input {
                        r#type: "radio",
                        name: "{id}",
                        checked: is_default,
                        onchange: move |_| choice.set(FolderChoice::Default),
                    }
                    span { class: "choice-body",
                        span { class: "choice-label", "{default_label}" }
                        span { class: "choice-detail", title: "{default_path}", "{default_path}" }
                    }
                }

                label { class: if !is_default { "choice is-selected" } else { "choice" },
                    input {
                        r#type: "radio",
                        name: "{id}",
                        checked: !is_default,
                        onchange: move |_| {
                            if choice() == FolderChoice::Default {
                                choice.set(FolderChoice::Custom(None));
                            }
                        },
                    }
                    span { class: "choice-body",
                        span { class: "choice-label", "Custom folder" }
                        match &custom_path {
                            Some(path) => rsx! { span { class: "choice-detail", title: "{path}", "{path}" } },
                            None => rsx! { span { class: "choice-detail is-empty", "No folder chosen" } },
                        }
                    }
                }
            }

            if !is_default {
                div {
                    button {
                        class: "button button-secondary",
                        r#type: "button",
                        disabled: picking() || disabled,
                        aria_busy: if picking() { "true" },
                        onclick: choose,
                        Icon { name: IconName::FolderOpen, size: 14 }
                        span { if picking() { "Opening…" } else if custom_path.is_some() { "Change folder" } else { "Choose folder" } }
                    }
                }
            }

            if let Some(message) = message {
                p { class: "field-error", role: "alert", "{message}" }
            }
        }
    }
}
