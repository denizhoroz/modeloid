use dioxus::prelude::*;

#[component]
pub fn Home() -> Element {
    rsx! {
        div { class: "page",
            h1 { "Modeloid" }
            p { class: "page-lead", "Pick a tool from the sidebar to get started." }
        }
    }
}
