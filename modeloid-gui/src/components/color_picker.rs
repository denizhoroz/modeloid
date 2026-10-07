use dioxus::prelude::*;
use modeloid_types::colors;

/// A colour swatch button that opens a popup: the 16 preset colours (every
/// fourth of the 64-colour class palette) and, under them, "Custom colour…"
/// with the system picker. Escape or a click outside closes it.
///
/// ```ignore
/// ColorPicker { value: color, label: "Colour of car", onchange: move |c| set(c) }
/// ```
#[component]
pub fn ColorPicker(
    /// `#rrggbb`.
    #[props(into)]
    value: String,
    /// What the colour is for, e.g. "Colour of car" (button name, popup title).
    #[props(into)]
    label: String,
    onchange: EventHandler<String>,
) -> Element {
    let mut open = use_signal(|| false);
    let current = colors::normalize(&value);

    rsx! {
        div {
            class: "color-picker",
            onkeydown: move |e| {
                if e.key() == Key::Escape {
                    open.set(false);
                }
            },
            button {
                class: "color-picker-button",
                r#type: "button",
                style: "--swatch: {value}",
                title: "{label}",
                aria_label: "{label}",
                aria_haspopup: "dialog",
                aria_expanded: if open() { "true" } else { "false" },
                onclick: move |_| open.toggle(),
            }
            if open() {
                div { class: "color-picker-backdrop", onclick: move |_| open.set(false) }
                div { class: "color-picker-popup", role: "dialog", aria_label: "{label}",
                    div { class: "color-picker-grid", role: "group", aria_label: "Colours",
                        for color in colors::presets() {
                            button {
                                key: "{color}",
                                class: if current.as_deref() == Some(color.as_str()) { "color-picker-swatch is-selected" } else { "color-picker-swatch" },
                                r#type: "button",
                                style: "--swatch: {color}",
                                title: "{color}",
                                aria_label: "{color}",
                                aria_pressed: if current.as_deref() == Some(color.as_str()) { "true" } else { "false" },
                                onclick: {
                                    move |_| {
                                        onchange.call(color.clone());
                                        open.set(false);
                                    }
                                },
                            }
                        }
                    }
                    label { class: "color-picker-custom",
                        input {
                            class: "color-input",
                            r#type: "color",
                            value: "{value}",
                            onchange: move |e| {
                                onchange.call(e.value());
                                open.set(false);
                            },
                        }
                        "Custom colour…"
                    }
                }
            }
        }
    }
}
