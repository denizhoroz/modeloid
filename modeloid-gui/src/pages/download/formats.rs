//! Export format of the Download page: the Roboflow formats of the shared
//! format list (`FormatSelect`); "Other" lets the user type any name Roboflow accepts.

use dioxus::prelude::*;

use crate::components::{FormatSelect, FormatUse, InfoTip, OTHER_FORMAT};
use crate::components::{IconName, TitleIcon};

pub use modeloid_types::formats::DEFAULT_DOWNLOAD_FORMAT as DEFAULT_FORMAT;
/// Select value that reveals the free-text format input.
pub const OTHER: &str = OTHER_FORMAT;

/// Format picker: a list of known formats plus "Other" with a text input.
#[component]
pub fn FormatSection(selected: Signal<String>, custom: Signal<String>, error: Option<String>) -> Element {
    let mut selected = selected;
    let mut custom = custom;
    let is_other = selected() == OTHER;

    rsx! {
        section { class: "section-card", aria_labelledby: "download-options-title",
            h2 { id: "download-options-title", class: "section-card-title", TitleIcon { name: IconName::Download } "Download options" }

            div { class: "field",
                label { class: "field-label", r#for: "download-format",
                    "Roboflow export format "
                    InfoTip {
                        label: "About the export format",
                        text: "Annotation format Roboflow exports the dataset in; other sources download as they are. Roboflow offers different formats per project type. If the project doesn't offer the one chosen here, Modeloid switches to one it does and tells you which. Formats marked \"download only\" download fine, but Modeloid's tools can't read them yet.",
                    }
                }
                FormatSelect { id: "download-format", value: selected(), use_for: FormatUse::Download, onchange: move |v| selected.set(v) }
            }

            if is_other {
                div { class: "field",
                    label { class: "field-label", r#for: "download-format-custom",
                        "Format name "
                        InfoTip { label: "About the format name", text: "The export format name as Roboflow calls it." }
                    }
                    input {
                        id: "download-format-custom",
                        class: "input",
                        r#type: "text",
                        spellcheck: "false",
                        autocomplete: "off",
                        placeholder: "e.g. coco-mmdetection",
                        aria_invalid: if error.is_some() { "true" },
                        value: "{custom}",
                        oninput: move |e| custom.set(e.value()),
                    }
                    match &error {
                        Some(message) => rsx! { p { class: "field-error", role: "alert", "{message}" } },
                        None => rsx! {},
                    }
                }
            }
        }
    }
}
