//! One labelled input each, with an "i" explanation, for the Parameters form.

use dioxus::prelude::*;

use crate::components::{DecimalInput, InfoTip, NumberInput};

/// Whole number.
#[component]
pub fn IntField(
    #[props(into)] label: String,
    #[props(into)] tip: String,
    value: u32,
    #[props(default = 100_000)] max: u32,
    onchange: EventHandler<u32>,
) -> Element {
    rsx! {
        div { class: "field param-field",
            span { class: "field-label", "{label}" InfoTip { text: tip, label: "About {label}" } }
            NumberInput { value, label: label.clone(), max, onchange }
        }
    }
}

/// Decimal number.
#[component]
pub fn FloatField(
    #[props(into)] label: String,
    #[props(into)] tip: String,
    value: f64,
    #[props(default = 0.0)] min: f64,
    #[props(default = 1.0)] max: f64,
    #[props(default = 0.01)] step: f64,
    onchange: EventHandler<f64>,
) -> Element {
    rsx! {
        div { class: "field param-field",
            span { class: "field-label", "{label}" InfoTip { text: tip, label: "About {label}" } }
            DecimalInput { value, label: label.clone(), min, max, step, onchange }
        }
    }
}

/// On / off.
#[component]
pub fn BoolField(#[props(into)] label: String, #[props(into)] tip: String, value: bool, onchange: EventHandler<bool>) -> Element {
    rsx! {
        div { class: "field param-field param-check",
            label { class: "check",
                input { r#type: "checkbox", checked: value, onchange: move |e| onchange.call(e.checked()) }
                span { "{label}" }
            }
            InfoTip { text: tip, label: "About {label}" }
        }
    }
}

/// One of a few choices, as `(id, label)` pairs.
#[component]
pub fn ChoiceField(
    #[props(into)] label: String,
    #[props(into)] tip: String,
    options: Vec<(String, String)>,
    #[props(into)] value: String,
    onchange: EventHandler<String>,
) -> Element {
    rsx! {
        div { class: "field param-field",
            span { class: "field-label", "{label}" InfoTip { text: tip, label: "About {label}" } }
            select {
                class: "input select",
                aria_label: "{label}",
                value: "{value}",
                onchange: move |e| onchange.call(e.value()),
                for (id, text) in options {
                    option { key: "{id}", value: "{id}", selected: id == value, "{text}" }
                }
            }
        }
    }
}

/// A whole number that can be turned off (`None`), e.g. frozen layers.
#[component]
pub fn OptionalIntField(
    #[props(into)] label: String,
    #[props(into)] tip: String,
    value: Option<u32>,
    /// Used when it's switched on.
    on_value: u32,
    #[props(default = 100_000)] max: u32,
    onchange: EventHandler<Option<u32>>,
) -> Element {
    rsx! {
        div { class: "field param-field",
            span { class: "field-label", "{label}" InfoTip { text: tip, label: "About {label}" } }
            div { class: "param-optional",
                label { class: "check",
                    input {
                        r#type: "checkbox",
                        checked: value.is_some(),
                        aria_label: "Use {label}",
                        onchange: move |e| onchange.call(if e.checked() { Some(on_value) } else { None }),
                    }
                    if value.is_none() { span { class: "muted", "Off" } }
                }
                if let Some(n) = value {
                    NumberInput { value: n, label: label.clone(), max, onchange: move |n| onchange.call(Some(n)) }
                }
            }
        }
    }
}

/// A decimal that can be turned off (`None`), e.g. a time limit.
#[component]
pub fn OptionalFloatField(
    #[props(into)] label: String,
    #[props(into)] tip: String,
    value: Option<f64>,
    on_value: f64,
    #[props(default = 0.0)] min: f64,
    #[props(default = 1000.0)] max: f64,
    #[props(default = 0.5)] step: f64,
    onchange: EventHandler<Option<f64>>,
) -> Element {
    rsx! {
        div { class: "field param-field",
            span { class: "field-label", "{label}" InfoTip { text: tip, label: "About {label}" } }
            div { class: "param-optional",
                label { class: "check",
                    input {
                        r#type: "checkbox",
                        checked: value.is_some(),
                        aria_label: "Use {label}",
                        onchange: move |e| onchange.call(if e.checked() { Some(on_value) } else { None }),
                    }
                    if value.is_none() { span { class: "muted", "Off" } }
                }
                if let Some(n) = value {
                    DecimalInput { value: n, label: label.clone(), min, max, step, onchange: move |n| onchange.call(Some(n)) }
                }
            }
        }
    }
}

/// `(id, label)` options from a list of choices.
pub fn options<T: Copy>(all: &[T], id: impl Fn(T) -> &'static str, label: impl Fn(T) -> &'static str) -> Vec<(String, String)> {
    all.iter().map(|&t| (id(t).to_string(), label(t).to_string())).collect()
}
