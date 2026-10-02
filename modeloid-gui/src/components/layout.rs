use dioxus::prelude::*;

use crate::components::{provide_toasts, Icon, IconName, PanelTable, PanelTableState, PythonAutoSetup, ResizeHandle, SidePanel, Sidebar, StatusBar, ToastHost};
use crate::routes::Route;
use crate::state::{provide_activity_state, provide_workspace_state};

/// Width of the right-hand panel, shared by the tool pages' side panel and
/// the labelling page's Classes panel. Provided by `AppLayout`.
#[derive(Clone, Copy)]
pub struct PanelWidth(pub Signal<f64>);

/// App shell: sidebar on the left, current page on the right.
/// On narrow screens the sidebar becomes a drawer opened from the top bar.
#[component]
pub fn AppLayout() -> Element {
    provide_workspace_state();
    provide_activity_state();
    provide_toasts();
    let mut drawer_open = use_signal(|| false);
    // Lives here so the panel keeps its width across dataset pages.
    let panel_width = use_signal(|| 420.0);
    use_context_provider(|| PanelWidth(panel_width));
    let section = match use_route::<Route>() {
        Route::Tool { section, .. } => section,
        _ => String::new(),
    };
    let show_panel = PanelTable::for_section(&section).is_some();
    // Lives here too, so the chosen table survives page changes within a
    // category; a page of another category switches to that category's table.
    let mut panel_table = use_signal(|| PanelTable::for_section(&section).unwrap_or(PanelTable::Datasets));
    use_context_provider(|| PanelTableState(panel_table));
    use_effect(use_reactive((&section,), move |(section,)| {
        if let Some(table) = PanelTable::for_section(&section) {
            panel_table.set(table);
        }
    }));

    rsx! {
        a { class: "skip-link", href: "#main", "Skip to content" }
        div {
            class: "app-shell",
            onkeydown: move |e| {
                if e.key() == Key::Escape {
                    drawer_open.set(false);
                }
            },

            header { class: "topbar",
                button {
                    class: "icon-button",
                    r#type: "button",
                    aria_label: "Open navigation",
                    aria_controls: "sidebar",
                    aria_expanded: if drawer_open() { "true" } else { "false" },
                    onclick: move |_| drawer_open.toggle(),
                    Icon { name: IconName::Menu }
                }
                span { class: "topbar-title", "Modeloid" }
            }

            Sidebar {
                open: drawer_open(),
                on_navigate: move |_| drawer_open.set(false),
            }

            if drawer_open() {
                div {
                    class: "scrim",
                    aria_hidden: "true",
                    onclick: move |_| drawer_open.set(false),
                }
            }

            if show_panel {
                div { class: "content content-split", style: "--panel-width: {panel_width}px",
                    main { id: "main", class: "content-main", tabindex: "-1",
                        Outlet::<Route> {}
                    }
                    ResizeHandle {
                        width: panel_width,
                        min: 260.0,
                        max: 900.0,
                        label: "Resize side panel",
                        controls: "side-panel",
                    }
                    SidePanel { table: panel_table }
                }
            } else {
                main { id: "main", class: "content", tabindex: "-1",
                    Outlet::<Route> {}
                }
            }

            StatusBar {}
            ToastHost {}
            PythonAutoSetup {}
        }
    }
}

