use dioxus::prelude::*;

/// Whole-number input. While the text isn't a number, the last one stays.
#[component]
pub fn NumberInput(value: u32, #[props(into)] label: String, #[props(default = 100)] max: u32, onchange: EventHandler<u32>) -> Element {
    rsx! {
        input {
            class: "input input-narrow",
            r#type: "number",
            min: "0",
            max: "{max}",
            value: "{value}",
            aria_label: "{label}",
            oninput: move |e| {
                if let Ok(n) = e.value().trim().parse() {
                    onchange.call(n);
                }
            },
        }
    }
}
