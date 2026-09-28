use dioxus::prelude::*;

use crate::components::{Icon, IconName};
use crate::nav;
use crate::pages::classes::ClassesMerge;
use crate::pages::download::Download;
use crate::pages::explore::Explore;
use crate::pages::health::HealthCheck;

/// Placeholder page for every sidebar tool until each one is implemented.
#[component]
pub fn Tool(section: String, tool: String) -> Element {
    let Some((category, item)) = nav::find(&section, &tool) else {
        return rsx! {
            div { class: "page",
                h1 { "Tool not found" }
                p { class: "page-lead", "There is no tool at /{section}/{tool}." }
            }
        };
    };

    // Implemented tools; everything else shows the placeholder below.
    match (category.slug, item.slug) {
        ("dataset", "download") => return rsx! { Download {} },
        ("dataset", "explore") => return rsx! { Explore {} },
        ("dataset", "health-check") => return rsx! { HealthCheck {} },
        ("dataset", "classes-merge") => return rsx! { ClassesMerge {} },
        _ => {}
    }

    rsx! {
        document::Title { "{item.label} · Modeloid" }
        div { class: "page",
            p { class: "page-eyebrow", "{category.label}" }
            h1 { "{item.label}" }
            div { class: "empty-state",
                Icon { name: IconName::Wrench, size: 22 }
                p { "This tool isn't built yet." }
            }
        }
    }
}
