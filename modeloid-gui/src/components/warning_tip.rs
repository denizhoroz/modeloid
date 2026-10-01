use dioxus::prelude::*;

use crate::components::{Icon, IconName};

/// A warning icon with a count that lists `errors` and `warnings` on hover.
/// Like `InfoTip`, focus or a click keeps the list open until Escape or until
/// focus moves on. Shows nothing when both lists are empty.
///
/// ```ignore
/// div { class: "section-card-header",
///     h2 { class: "section-card-title", "Result" }
///     WarningTip { errors: preview.problems.clone(), warnings: notes }
/// }
/// ```
#[component]
pub fn WarningTip(
    /// Problems that stop the run; shown first, in the danger colour.
    #[props(default)]
    errors: Vec<String>,
    /// Things worth knowing that don't stop the run.
    #[props(default)]
    warnings: Vec<String>,
) -> Element {
    let mut open = use_signal(|| false);
    let count = errors.len() + warnings.len();
    if count == 0 {
        return rsx! {};
    }

    let level = if errors.is_empty() { "is-warning" } else { "is-error" };
    let open_class = if open() { " is-open" } else { "" };
    let label = match (errors.len(), warnings.len()) {
        (0, w) => format!("{w} warning{}", if w == 1 { "" } else { "s" }),
        (e, 0) => format!("{e} problem{}", if e == 1 { "" } else { "s" }),
        (e, w) => format!("{e} problem{}, {w} warning{}", if e == 1 { "" } else { "s" }, if w == 1 { "" } else { "s" }),
    };
    let spoken = errors.iter().chain(warnings.iter()).cloned().collect::<Vec<_>>().join(" ");

    rsx! {
        span { class: "warning-tip {level}{open_class}",
            button {
                class: "warning-tip-button",
                r#type: "button",
                title: "{label}",
                // Screen readers get the whole list with the button itself.
                aria_label: "{label}: {spoken}",
                aria_expanded: if open() { "true" } else { "false" },
                onclick: move |e| {
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
                Icon { name: IconName::Warning, size: 16 }
                span { class: "warning-tip-count", "{count}" }
            }
            div { class: "warning-tip-popup", aria_hidden: "true",
                ul { class: "warning-list",
                    for error in errors.iter() {
                        li { key: "e-{error}", class: "is-error", "{error}" }
                    }
                    for warning in warnings.iter() {
                        li { key: "w-{warning}", "{warning}" }
                    }
                }
            }
        }
    }
}
