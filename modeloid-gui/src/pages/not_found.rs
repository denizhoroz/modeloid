use dioxus::prelude::*;

use crate::routes::Route;

#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    let path = segments.join("/");

    rsx! {
        main { class: "page page-standalone",
            h1 { "Page not found" }
            p { class: "page-lead", "Nothing lives at /{path}." }
            Link { class: "button", to: Route::Home {}, "Back to home" }
        }
    }
}
