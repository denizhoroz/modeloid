use dioxus::prelude::*;

/// A small "i" button that explains the thing it sits next to. The
/// explanation shows on hover, and on focus or a click it stays until
/// Escape or until focus moves on.
///
/// ```ignore
/// label { class: "field-label", "Seed" InfoTip { text: "The same seed gives the same split again." } }
/// ```
#[component]
pub fn InfoTip(
    /// The explanation.
    #[props(into)]
    text: String,
    /// What the explanation is about, for screen readers, e.g. "About the seed".
    #[props(into, default = "More information".to_string())]
    label: String,
) -> Element {
    let mut open = use_signal(|| false);

    rsx! {
        span { class: if open() { "info-tip is-open" } else { "info-tip" },
            button {
                class: "info-tip-button",
                r#type: "button",
                // Screen readers get the explanation with the button itself.
                aria_label: "{label}: {text}",
                aria_expanded: if open() { "true" } else { "false" },
                onclick: move |e| {
                    // Inside a <label>, a click would also reach the label's input.
                    e.prevent_default();
                    e.stop_propagation();
                    open.set(true);
                },
                onfocus: move |_| open.set(true),
                onblur: move |_| open.set(false),
                onkeydown: move |e| {
                    if e.key() == Key::Escape {
                        open.set(false);
                    }
                },
                "i"
            }
            span { class: "info-tip-popup", aria_hidden: "true", "{text}" }
        }
    }
}
