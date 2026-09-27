use dioxus::prelude::*;

use crate::components::RoboflowKeyField;

#[component]
pub fn Settings() -> Element {
    rsx! {
        document::Title { "Settings · Modeloid" }
        div { class: "page",
            h1 { "Settings" }

            section { class: "settings-section", aria_labelledby: "settings-api-keys",
                h2 { id: "settings-api-keys", class: "settings-section-title", "API Keys" }
                p { class: "settings-section-lead", "Keys for the services Modeloid downloads from." }
                div { class: "settings-card",
                    RoboflowKeyField {}
                }
            }
        }
    }
}
