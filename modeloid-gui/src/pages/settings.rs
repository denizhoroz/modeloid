use dioxus::prelude::*;

use crate::api::settings::ServiceId;
use crate::components::{AppearanceSettings, CredentialField};

#[component]
pub fn Settings() -> Element {
    rsx! {
        document::Title { "Settings · Modeloid" }
        div { class: "page",
            h1 { "Settings" }

            section { class: "settings-section", aria_labelledby: "settings-appearance",
                h2 { id: "settings-appearance", class: "settings-section-title", "Appearance" }
                p { class: "settings-section-lead", "How Modeloid looks on this computer." }
                AppearanceSettings {}
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
