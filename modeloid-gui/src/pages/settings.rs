use dioxus::prelude::*;

use crate::api::settings::ServiceId;
use crate::components::{AppearanceSettings, CredentialField, WorkspaceSettings};
use crate::state::use_workspace;

#[component]
pub fn Settings() -> Element {
    let workspace = use_workspace();

    rsx! {
        document::Title { "Settings · Modeloid" }
        div { class: "page",
            h1 { "Settings" }

            section { class: "settings-section", aria_labelledby: "settings-appearance",
                h2 { id: "settings-appearance", class: "settings-section-title", "Appearance" }
                p { class: "settings-section-lead", "How Modeloid looks on this computer." }
                AppearanceSettings {}
            }

            section { class: "settings-section", aria_labelledby: "settings-workspace",
                h2 { id: "settings-workspace", class: "settings-section-title", "Workspace" }
                if let Some(info) = workspace() {
                    p { class: "settings-section-lead",
                        "Options of the open workspace, "
                        strong { "{info.folder}" }
                        ". Stored in its workspace.toml."
                    }
                }
                match workspace() {
                    // Keyed by folder: another workspace starts from its own values.
                    Some(info) => rsx! { WorkspaceSettings { key: "{info.path}", info } },
                    None => rsx! {
                        p { class: "settings-section-lead", "No workspace loaded. Create or load one to change its options." }
                    },
                }
            }

            section { class: "settings-section", aria_labelledby: "settings-api-keys",
                h2 { id: "settings-api-keys", class: "settings-section-title", "API Keys" }
                p { class: "settings-section-lead",
                    "Keys for the services Modeloid downloads from. Stored on this computer in your user config folder, shared with the command line tool."
                }
                div { class: "settings-card",
                    for service in ServiceId::ALL {
                        CredentialField { key: "{service.slug()}", service }
                    }
                }
            }
        }
    }
}
