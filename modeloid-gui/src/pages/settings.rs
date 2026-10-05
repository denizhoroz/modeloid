use dioxus::prelude::*;

use crate::api::settings::ServiceId;
use crate::components::{IconName, TitleIcon};
use crate::components::{AppearanceSettings, CacheSettings, CredentialField, PythonSetupCard, SystemSettings};

#[component]
pub fn Settings() -> Element {
    rsx! {
        document::Title { "Settings · Modeloid" }
        div { class: "page",
            h1 { "Settings" }

            section { class: "settings-section", aria_labelledby: "settings-appearance",
                h2 { id: "settings-appearance", class: "settings-section-title", TitleIcon { name: IconName::Palette } "Appearance" }
                p { class: "settings-section-lead", "How Modeloid looks on this computer." }
                AppearanceSettings {}
            }

            section { class: "settings-section", aria_labelledby: "settings-system",
                h2 { id: "settings-system", class: "settings-section-title", TitleIcon { name: IconName::Cpu } "System" }
                p { class: "settings-section-lead",
                    "How much of this computer Modeloid uses. Stored on this computer, for every workspace."
                }
                SystemSettings {}
            }

            section { class: "settings-section", aria_labelledby: "settings-cache",
                h2 { id: "settings-cache", class: "settings-section-title", TitleIcon { name: IconName::Database } "Cache" }
                p { class: "settings-section-lead",
                    "Image previews and statistics of the open workspace, kept so pages open fast. Stored in its .cache folder."
                }
                CacheSettings {}
            }

            section { class: "settings-section", aria_labelledby: "settings-python",
                h2 { id: "settings-python", class: "settings-section-title", TitleIcon { name: IconName::Code } "Python environment" }
                p { class: "settings-section-lead",
                    "The Python, PyTorch and Ultralytics that training runs on. Modeloid keeps its own copy next to the app."
                }
                PythonSetupCard {}
            }

            section { class: "settings-section", aria_labelledby: "settings-api-keys",
                h2 { id: "settings-api-keys", class: "settings-section-title", TitleIcon { name: IconName::Key } "API Keys" }
                p { class: "settings-section-lead",
                    "Keys for the services Modeloid downloads from. Stored on this computer in your user config folder."
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
