use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::workspace::load_workspace;
use crate::components::{Icon, IconName};
use crate::routes::Route;
use crate::state::use_workspace;

/// Sidebar box showing the current workspace.
/// Create opens the create form; Load opens a native folder picker.
#[component]
pub fn WorkspaceCard(on_navigate: EventHandler<()>) -> Element {
    let mut workspace = use_workspace();
    let mut loading = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let current = use_route::<Route>();
    let creating = current == Route::CreateWorkspace {};

    let load = move |_| async move {
        if loading() {
            return;
        }
        loading.set(true);
        error.set(None);
        match load_workspace().await {
            Ok(Some(info)) => workspace.set(Some(info)),
            Ok(None) => {} // dialog cancelled
            Err(e) => error.set(Some(error_message(&e))),
        }
        loading.set(false);
    };

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
