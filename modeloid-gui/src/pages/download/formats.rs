//! Export formats offered on the Download page. Values are Roboflow export
//! format names; "Other" lets the user type any name Roboflow accepts.

use dioxus::prelude::*;

use crate::components::InfoTip;

pub const DEFAULT_FORMAT: &str = "coco";
/// Select value that reveals the free-text format input.
pub const OTHER: &str = "__other";

pub struct FormatGroup {
    pub label: &'static str,
    pub formats: &'static [(&'static str, &'static str)], // (value, label)
}

pub const FORMAT_GROUPS: &[FormatGroup] = &[
    FormatGroup {
        label: "Object detection / segmentation",
        formats: &[
            ("coco", "COCO JSON"),
            ("coco-segmentation", "COCO Segmentation"),
            ("yolov11", "YOLOv11 TXT"),
            ("yolov8", "YOLOv8 TXT"),
            ("yolov5pytorch", "YOLOv5 PyTorch TXT"),
            ("voc", "Pascal VOC XML"),
            ("createml", "CreateML JSON"),
            ("tfrecord", "TFRecord"),
        ],
    },
    FormatGroup {
        label: "Classification",
        formats: &[("folder", "Folder structure"), ("multiclass", "Multi-label CSV")],
    },
];

/// Format picker: a list of known formats plus "Other" with a text input.
#[component]
pub fn FormatSection(selected: Signal<String>, custom: Signal<String>, error: Option<String>) -> Element {
    let mut selected = selected;
    let mut custom = custom;
    let is_other = selected() == OTHER;

    rsx! {
        section { class: "section-card", aria_labelledby: "download-options-title",
            h2 { id: "download-options-title", class: "section-card-title", "Download options" }

            div { class: "field",
                label { class: "field-label", r#for: "download-format",
                    "Roboflow export format "
                    InfoTip {
                        label: "About the export format",
                        text: "Annotation format Roboflow exports the dataset in; other sources download as they are. Roboflow offers different formats per project type. If the project doesn't offer the one chosen here, Modeloid switches to one it does and tells you which.",
                    }
                }
                select {
                    id: "download-format",
                    class: "input select",
                    value: "{selected}",
                    onchange: move |e| selected.set(e.value()),
                    for group in FORMAT_GROUPS {
                        optgroup { key: "{group.label}", label: group.label,
                            for (value, label) in group.formats {
                                option { key: "{value}", value: *value, selected: selected() == *value, "{label}" }
                            }
                        }
                    }
                    option { value: OTHER, selected: is_other, "Other…" }
                }
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
                        placeholder: "e.g. yolov8-obb",
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
