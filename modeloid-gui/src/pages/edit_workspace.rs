use dioxus::prelude::*;

use crate::components::{IconName, NoWorkspace, TitleIcon, WorkspaceSettings};
use crate::state::use_workspace;

/// The open workspace's options (description, seed, default label format),
/// opened from the gear on the Current Workspace card.
#[component]
pub fn EditWorkspace() -> Element {
    let workspace = use_workspace();

    rsx! {
        document::Title { "Workspace · Modeloid" }
        div { class: "page",
            h1 { "Workspace" }
            match workspace() {
                Some(info) => rsx! {
                    section { class: "settings-section", aria_labelledby: "workspace-options-title",
                        h2 { id: "workspace-options-title", class: "settings-section-title", TitleIcon { name: IconName::FolderOpen } "{info.folder}" }
                        p { class: "settings-section-lead", "Options of the open workspace, stored in its workspace.toml." }
                        // Keyed by folder: another workspace starts from its own values.
                        WorkspaceSettings { key: "{info.path}", info }
                    }
                },
                None => rsx! { NoWorkspace { text: "Create or load a workspace to change its options." } },
            }
        }
    }
}
