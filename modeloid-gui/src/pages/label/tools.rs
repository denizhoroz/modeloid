//! The vertical tools bar left of the image.

use dioxus::prelude::*;

use super::{use_editor, Tool};
use crate::components::{Icon, IconName};

#[component]
pub fn ToolsPanel() -> Element {
    let editor = use_editor();
    let mut tool = editor.tool;
    let mut show_labels = editor.show_labels;
    let current = tool();
    let read_only = editor.read_only();
    let has_selection = !editor.selected.read().is_empty();

    rsx! {
        div { class: "label-tools", role: "toolbar", aria_label: "Labelling tools", aria_orientation: "vertical",
            for (value, icon, label, key) in [
                (Tool::Select, IconName::Pointer, "Select and move", "V"),
                (Tool::Box, IconName::Square, "Draw boxes", "B"),
                (Tool::Pan, IconName::Hand, "Pan (or hold Space)", "H"),
            ] {
                button {
                    key: "{key}",
                    class: if current == value { "icon-button label-tool is-active" } else { "icon-button label-tool" },
                    r#type: "button",
                    aria_label: "{label}",
                    aria_pressed: if current == value { "true" } else { "false" },
                    title: "{label} ({key})",
                    onclick: move |_| tool.set(value),
                    Icon { name: icon, size: 18 }
                }
            }
            button {
                class: "icon-button label-tool",
                r#type: "button",
                aria_label: "Delete selected labels",
                title: "Delete selected labels (Delete)",
                disabled: read_only || !has_selection,
                onclick: move |_| editor.delete_selected(),
                Icon { name: IconName::Trash, size: 18 }
            }
            hr { class: "label-tools-rule" }
            button {
                class: "icon-button label-tool",
                r#type: "button",
                aria_label: "Undo",
                title: "Undo (Ctrl+Z)",
                disabled: read_only || !editor.can_undo(),
                onclick: move |_| editor.undo(),
                Icon { name: IconName::Undo, size: 18 }
            }
            button {
                class: "icon-button label-tool",
                r#type: "button",
                aria_label: "Redo",
                title: "Redo (Ctrl+Y)",
                disabled: read_only || !editor.can_redo(),
                onclick: move |_| editor.redo(),
                Icon { name: IconName::Redo, size: 18 }
            }
            hr { class: "label-tools-rule" }
            button {
                class: "icon-button label-tool",
                r#type: "button",
                aria_label: "Zoom in",
                title: "Zoom in (+ or mouse wheel)",
                onclick: move |_| editor.zoom(1.25, None),
                Icon { name: IconName::ZoomIn, size: 18 }
            }
            button {
                class: "icon-button label-tool",
                r#type: "button",
                aria_label: "Zoom out",
                title: "Zoom out (−)",
                onclick: move |_| editor.zoom(1.0 / 1.25, None),
                Icon { name: IconName::ZoomOut, size: 18 }
            }
            button {
                class: "icon-button label-tool",
                r#type: "button",
                aria_label: "Fit the image",
                title: "Fit the image (0)",
                onclick: move |_| editor.fit(),
                Icon { name: IconName::Fit, size: 18 }
            }
            button {
                class: "icon-button label-tool",
                r#type: "button",
                aria_label: if show_labels() { "Hide labels" } else { "Show labels" },
                aria_pressed: if show_labels() { "false" } else { "true" },
                title: if show_labels() { "Hide labels (L)" } else { "Show labels (L)" },
                onclick: move |_| show_labels.toggle(),
                Icon { name: if show_labels() { IconName::Eye } else { IconName::EyeOff }, size: 18 }
            }
        }
    }
}
