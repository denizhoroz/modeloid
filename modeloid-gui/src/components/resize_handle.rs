use dioxus::prelude::*;

/// Vertical bar on the left edge of a right-hand panel. Dragging it left makes
/// the panel wider. Also works from the keyboard: arrows, Home, End.
#[component]
pub fn ResizeHandle(
    /// Panel width in px, owned by the parent.
    width: Signal<f64>,
    #[props(default = 200.0)] min: f64,
    #[props(default = 900.0)] max: f64,
    /// Accessible name, e.g. "Resize datasets panel".
    #[props(into)]
    label: String,
    /// Id of the panel being resized.
    #[props(into)]
    controls: String,
) -> Element {
    let mut width = width;
    // (pointer x, width) when the drag started; `Some` while dragging.
    let mut drag = use_signal(|| None::<(f64, f64)>);
    const STEP: f64 = 16.0;

    let mut set = move |w: f64| width.set(w.clamp(min, max).round());

    rsx! {
        div {
            class: if drag().is_some() { "resize-handle is-dragging" } else { "resize-handle" },
            role: "separator",
            tabindex: "0",
            aria_label: "{label}",
            aria_controls: "{controls}",
            aria_orientation: "vertical",
            aria_valuemin: "{min}",
            aria_valuemax: "{max}",
            aria_valuenow: "{width}",
            onpointerdown: move |e| {
                e.prevent_default();
                drag.set(Some((e.client_coordinates().x, width())));
            },
            onkeydown: move |e| match e.key() {
                Key::ArrowLeft => set(width() + STEP),
                Key::ArrowRight => set(width() - STEP),
                Key::Home => set(max),
                Key::End => set(min),
                _ => {}
            },
            ondoubleclick: move |_| set(420.0),
        }

        // While dragging, a full-screen layer catches the pointer so the drag
        // keeps working over iframes/text and shows the resize cursor everywhere.
        if let Some((start_x, start_width)) = drag() {
            div {
                class: "resize-overlay",
                onpointermove: move |e| set(start_width - (e.client_coordinates().x - start_x)),
                onpointerup: move |_| drag.set(None),
                onpointerleave: move |_| drag.set(None),
            }
        }
    }
}
