use dioxus::prelude::*;

/// Number input for decimals (learning rates, probabilities). It keeps what
/// the user typed ("0.", "1e-") while it isn't a number yet, sends each valid
/// number, and follows `value` when it changes from outside (another set
/// loaded, "Reset to defaults").
#[component]
pub fn DecimalInput(
    value: ReadSignal<f64>,
    #[props(into)] label: String,
    #[props(default = 0.0)] min: f64,
    #[props(default = 1.0)] max: f64,
    #[props(default = 0.01)] step: f64,
    #[props(into, default)] id: String,
    onchange: EventHandler<f64>,
) -> Element {
    let mut text = use_signal(|| format_value(value()));

    use_effect(move || {
        let outside = value();
        if text.peek().trim().parse::<f64>().ok() != Some(outside) {
            text.set(format_value(outside));
        }
    });

    rsx! {
        input {
            class: "input input-narrow",
            r#type: "number",
            id: if !id.is_empty() { "{id}" },
            inputmode: "decimal",
            min: "{min}",
            max: "{max}",
            step: "{step}",
            value: "{text}",
            aria_label: "{label}",
            oninput: move |e| {
                let typed = e.value();
                if let Ok(n) = typed.trim().parse::<f64>() {
                    if n.is_finite() {
                        onchange.call(n);
                    }
                }
                text.set(typed);
            },
        }
    }
}

/// Shortest text that reads back as `value`, e.g. "0.0005", not "5e-4".
fn format_value(value: f64) -> String {
    let text = format!("{value}");
    if text.contains('e') { format!("{value:.10}").trim_end_matches('0').trim_end_matches('.').to_string() } else { text }
}

#[cfg(test)]
mod tests {
    use super::format_value;

    #[test]
    fn formats_plainly() {
        assert_eq!(format_value(0.0005), "0.0005");
        assert_eq!(format_value(0.937), "0.937");
        assert_eq!(format_value(3.0), "3");
        assert_eq!(format_value(1e-7), "0.0000001");
    }
}
