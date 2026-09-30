use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::workspace::load_workspace;
use crate::components::{Icon, IconName};
use crate::routes::Route;
use crate::state::use_workspace;

/// Loading a workspace through the native folder picker: whether the picker
/// is open, the last error, and the action to start it.
fn use_load_workspace() -> (Signal<bool>, Signal<Option<String>>, impl FnMut(MouseEvent) + Copy) {
    let mut workspace = use_workspace();
    let mut loading = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);

    let load = move |_| {
        if loading() {
            return;
        }
        spawn(async move {
            loading.set(true);
            error.set(None);
            match load_workspace().await {
                Ok(Some(info)) => workspace.set(Some(info)),
                Ok(None) => {} // dialog cancelled
                Err(e) => error.set(Some(error_message(&e))),
            }
            loading.set(false);
        });
    };
    (loading, error, load)
}

/// Shown by a tool page while no workspace is open: what the tool needs, and
/// buttons to create a workspace or load one.
#[component]
pub fn NoWorkspace(#[props(into)] text: String) -> Element {
    let (loading, error, load) = use_load_workspace();

    rsx! {
        div { class: "notice",
            p { "{text} Create a workspace or load one." }
            div { class: "notice-actions",
                Link { class: "button", to: Route::CreateWorkspace {}, "Create workspace" }
                button {
                    class: "button button-secondary",
                    r#type: "button",
                    disabled: loading(),
                    aria_busy: if loading() { "true" },
                    onclick: load,
                    if loading() { "Opening…" } else { "Load workspace" }
                }
            }
            if let Some(message) = error() {
                p { class: "field-error notice-error", role: "alert", "{message}" }
            }
        }
    }
}

/// Sidebar box showing the current workspace.
/// Create opens the create form; Load opens a native folder picker.
#[component]
pub fn WorkspaceCard(on_navigate: EventHandler<()>) -> Element {
    let workspace = use_workspace();
    let (loading, error, load) = use_load_workspace();
    let current = use_route::<Route>();
    let creating = current == Route::CreateWorkspace {};

    rsx! {
        section { class: "workspace-card", aria_labelledby: "workspace-card-label",
            p { id: "workspace-card-label", class: "workspace-card-label", "Current Workspace" }

            // One line in both states, so the navbar below doesn't shift on load.
            match workspace() {
                Some(info) => rsx! {
                    p { class: "workspace-card-name", title: "{info.folder}", "{info.folder}" }
                },
                None => rsx! {
                    p { class: "workspace-card-name is-empty", "No workspace loaded" }
                },
            }

            div { class: "workspace-card-actions",
                Link {
                    class: if creating { "workspace-button is-active" } else { "workspace-button" },
                    aria_current: if creating { "page" },
                    to: Route::CreateWorkspace {},
                    onclick: move |_| on_navigate.call(()),
                    Icon { name: IconName::Plus, size: 14 }
                    span { "Create" }
                }
                button {
                    class: "workspace-button",
                    r#type: "button",
                    disabled: loading(),
                    aria_busy: if loading() { "true" },
                    onclick: load,
                    Icon { name: IconName::FolderOpen, size: 14 }
                    span { if loading() { "Opening…" } else { "Load" } }
                }
            }

            if let Some(message) = error() {
                p { class: "workspace-card-error", role: "alert", "{message}" }
            }
        }
    }
}
