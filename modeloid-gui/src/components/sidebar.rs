use dioxus::prelude::*;

use crate::components::{BrandMark, HistoryButton, Icon, IconName, WorkspaceCard};
use crate::nav::{NavCategory, NAV};
use crate::routes::Route;

/// Vertical navigation: current workspace box, one collapsible group per `NAV`
/// category, then History and Settings pinned to the bottom.
#[component]
pub fn Sidebar(open: bool, on_navigate: EventHandler<()>) -> Element {
    let current = use_route::<Route>();
    let on_settings = current == Route::Settings {};

    rsx! {
        nav {
            id: "sidebar",
            class: "sidebar",
            class: if open { "is-open" },
            aria_label: "Main",

            Link { class: "sidebar-brand", to: Route::Home {},
                onclick: move |_| on_navigate.call(()),
                BrandMark { size: 18 }
                span { "Modeloid" }
            }

            WorkspaceCard { on_navigate }

            ul { class: "nav-tree",
                for category in NAV {
                    NavGroup { key: "{category.slug}", category, on_navigate }
                }
            }

            HistoryButton {}

            div { class: "sidebar-footer",
                Link {
                    class: if on_settings { "sidebar-link is-active" } else { "sidebar-link" },
                    aria_current: if on_settings { "page" },
                    to: Route::Settings {},
                    onclick: move |_| on_navigate.call(()),
                    Icon { name: IconName::Settings, size: 16 }
                    span { "Settings" }
                }
            }
        }
    }
}

#[component]
fn NavGroup(category: &'static NavCategory, on_navigate: EventHandler<()>) -> Element {
    let mut expanded = use_signal(|| true);
    let current = use_route::<Route>();
    let list_id = format!("nav-{}", category.slug);

    rsx! {
        li { class: "nav-group",
            button {
                class: "nav-group-toggle",
                r#type: "button",
                aria_expanded: if expanded() { "true" } else { "false" },
                aria_controls: "{list_id}",
                onclick: move |_| expanded.toggle(),
                Icon { name: category.icon, size: 16 }
                span { class: "nav-group-label", "{category.label}" }
                Icon { name: IconName::ChevronDown, size: 14, class: "nav-chevron" }
            }

            ul { id: "{list_id}", class: "nav-items", hidden: !expanded(),
                for item in category.items {
                    li { key: "{item.slug}",
                        {
                            let target = category.route(item);
                            let active = current == target;
                            rsx! {
                                Link {
                                    class: if active { "nav-link is-active" } else { "nav-link" },
                                    aria_current: if active { "page" },
                                    to: target,
                                    onclick: move |_| on_navigate.call(()),
                                    "{item.label}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
