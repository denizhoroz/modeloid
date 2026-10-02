use dioxus::prelude::*;

use crate::api::appearance::{save_appearance, Appearance, FONTS, SCALES, THEMES};
use crate::api::error_message;
use crate::state::use_appearance;

/// Theme, font and text size pickers. A choice applies at once and is saved
/// on the server; if saving fails it still holds until the app reloads.
#[component]
pub fn AppearanceSettings() -> Element {
    let mut appearance = use_appearance();
    let mut error = use_signal(|| None::<String>);

    let mut choose = move |change: fn(&mut Appearance, &'static str), value: &'static str| {
        change(&mut appearance.write(), value);
        let chosen = appearance();
        spawn(async move {
            match save_appearance(chosen).await {
                Ok(()) => error.set(None),
                Err(e) => error.set(Some(format!("Couldn't save your choice: {}", error_message(&e)))),
            }
        });
    };

    let current = appearance();

    rsx! {
        div { class: "settings-card",
            fieldset { class: "field appearance-field",
                legend { class: "field-label", "Theme" }
                for (dark, heading) in [(true, "Dark"), (false, "Light")] {
                    div { key: "{heading}", class: "theme-group", role: "group", aria_labelledby: "theme-group-{heading}",
                        h3 { id: "theme-group-{heading}", class: "theme-group-title", "{heading}" }
                        div { class: "theme-grid",
                            for theme in THEMES.iter().filter(|t| t.dark == dark) {
                                label { key: "{theme.id}", class: "theme-card",
                                    input {
                                        class: "visually-hidden",
                                        r#type: "radio",
                                        name: "appearance-theme",
                                        value: theme.id,
                                        checked: current.theme == theme.id,
                                        onchange: move |_| choose(|a, v| a.theme = v.into(), theme.id),
                                    }
                                    span { class: "theme-preview", "data-theme": theme.id, aria_hidden: "true",
                                        span { class: "theme-preview-side" }
                                        span { class: "theme-preview-main",
                                            span { class: "theme-preview-text", "Aa" }
                                            span { class: "theme-preview-line" }
                                            span { class: "theme-preview-accent" }
                                        }
                                    }
                                    span { class: "theme-card-name", "{theme.label}" }
                                }
                            }
                        }
                    }
                }
            }

            fieldset { class: "field appearance-field",
                legend { class: "field-label", "Font" }
                div { class: "font-list",
                    for font in FONTS {
                        label { key: "{font.id}", class: "font-option",
                            input {
                                r#type: "radio",
                                name: "appearance-font",
                                value: font.id,
                                checked: current.font == font.id,
                                onchange: move |_| choose(|a, v| a.font = v.into(), font.id),
                            }
                            span { class: "font-option-name", "data-font": font.id, "{font.label}" }
                            span { class: "font-option-note", "{font.note}" }
                        }
                    }
                }
            }

            fieldset { class: "field appearance-field",
                legend { class: "field-label", "Text size" }
                div { class: "font-list",
                    for (id, label) in SCALES {
                        label { key: "{id}", class: "font-option",
                            input {
                                r#type: "radio",
                                name: "appearance-scale",
                                value: id,
                                checked: current.scale == id,
                                onchange: move |_| choose(|a, v| a.scale = v.into(), id),
                            }
                            span { class: "font-option-name", "{label}" }
                            // Shown in its own size, whatever size is chosen now.
                            span { class: "scale-sample", "data-scale": id, "The quick brown fox jumps over the lazy dog" }
                        }
                    }
                }
            }

            if let Some(message) = error() {
                p { class: "field-error", role: "alert", "{message}" }
            }
        }
    }
}
