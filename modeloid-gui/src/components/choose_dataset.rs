use dioxus::prelude::*;

/// Shown while a tool has no input: datasets are chosen in the datasets panel.
#[component]
pub fn SelectDatasetHint(#[props(into)] text: String) -> Element {
    rsx! {
        div { class: "notice select-hint", role: "status",
            p { "{text}" }
            span { class: "select-hint-arrow", aria_hidden: "true", "→" }
        }
    }
}

/// For single-dataset tools when several datasets are selected.
#[component]
pub fn PrimaryNote(count: usize) -> Element {
    rsx! {
        if count > 1 {
            p { class: "field-hint primary-note",
                "{count} datasets are selected; this tool uses the one selected last."
            }
        }
    }
}
