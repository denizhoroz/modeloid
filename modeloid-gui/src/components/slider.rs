use dioxus::prelude::*;

/// Whole-number slider with its value shown next to it.
#[component]
pub fn Slider(
    value: u32,
    /// Accessible name, e.g. "Subset in percent".
    #[props(into)]
    label: String,
    #[props(default = 0)] min: u32,
    #[props(default = 100)] max: u32,
    /// Shown after the number, e.g. "%" or " px".
    #[props(into, default)]
    unit: String,
    /// Shown instead of the number, e.g. "Automatic" for 0.
    #[props(default)]
    text: Option<String>,
    onchange: EventHandler<u32>,
) -> Element {
    let shown = text.unwrap_or_else(|| format!("{value}{unit}"));
    rsx! {
        span { class: "slider",
            input {
                class: "slider-input",
                r#type: "range",
                min: "{min}",
                max: "{max}",
                step: "1",
                value: "{value}",
                aria_label: "{label}",
                aria_valuetext: "{shown}",
                oninput: move |e| {
                    if let Ok(n) = e.value().trim().parse() {
                        onchange.call(n);
                    }
                },
            }
            output { class: "slider-value", aria_hidden: "true", "{shown}" }
        }
    }
}
