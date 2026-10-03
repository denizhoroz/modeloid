use std::rc::Rc;

use dioxus::prelude::*;

/// A small "i" button that explains the thing it sits next to. The
/// explanation shows on hover, and on focus or a click it stays until
/// Escape or until focus moves on.
///
/// The explanation is placed against the window (`position: fixed`), so a
/// scrolling table or card around the button can't cut it off or cover it;
/// near the bottom of the window it opens above the button.
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
    let mut button = use_signal(|| None::<Rc<MountedData>>);
    // Where the popup goes, as a style; `None` until measured.
    let mut place = use_signal(|| None::<String>);

    let measure = move || async move {
        let Some(el) = button() else { return };
        let Ok(rect) = el.get_client_rect().await else { return };
        let height = document::eval("return window.innerHeight;").await.ok().and_then(|v| v.as_f64()).unwrap_or(f64::MAX);
        let (left, top, bottom) = (rect.origin.x, rect.origin.y, rect.origin.y + rect.size.height);
        // Below the button, or above it in the lower part of the window.
        let vertical = if bottom > height * 0.6 { format!("bottom: {:.0}px", height - top + 6.0) } else { format!("top: {:.0}px", bottom + 6.0) };
        // Kept inside the window on the right.
        place.set(Some(format!("left: min({:.0}px, calc(100vw - min(320px, 70vw) - 32px)); {vertical}", left - 8.0)));
    };

    rsx! {
        span {
            class: if open() { "info-tip is-open" } else { "info-tip" },
            onmouseenter: move |_| async move { measure().await },
            button {
                class: "info-tip-button",
                r#type: "button",
                // Screen readers get the explanation with the button itself.
                aria_label: "{label}: {text}",
                aria_expanded: if open() { "true" } else { "false" },
                onmounted: move |e| button.set(Some(e.data())),
                onclick: move |e| {
                    // Inside a <label>, a click would also reach the label's input.
                    e.prevent_default();
                    e.stop_propagation();
                    open.set(true);
                },
                onfocus: move |_| async move {
                    measure().await;
                    open.set(true);
                },
                onblur: move |_| open.set(false),
                onkeydown: move |e| {
                    if e.key() == Key::Escape {
                        open.set(false);
                    }
                },
                "i"
            }
            span {
                class: if place().is_some() { "info-tip-popup is-placed" } else { "info-tip-popup" },
                style: place().unwrap_or_default(),
                aria_hidden: "true",
                "{text}"
            }
        }
    }
}
