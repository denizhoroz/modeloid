use dioxus::prelude::*;

use crate::components::{provide_toasts, HistoryLayer, HistoryOpen, Icon, IconName, PanelTable, PanelTableState, PythonAutoSetup, ResizeHandle, SidePanel, Sidebar, StatusBar, ToastHost};
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
    let history_open = use_signal(|| false);
    use_context_provider(|| HistoryOpen(history_open));
    // Lives here so the panel keeps its width across dataset pages.
    let panel_width = use_signal(|| 420.0);
    use_context_provider(|| PanelWidth(panel_width));
    let (section, tool) = match use_route::<Route>() {
        Route::Tool { section, tool } => (section, tool),
        _ => (String::new(), String::new()),
    };
    let show_panel = PanelTable::for_section(&section).is_some();
    // Lives here too, so the chosen table survives page changes within a
    // category; a page of another category switches to that category's table,
    // and a page with a table of its own (Performance → Models) to that one.
    let mut panel_table = use_signal(|| PanelTable::for_tool(&section, &tool).unwrap_or(PanelTable::Datasets));
    use_context_provider(|| PanelTableState(panel_table));
    // The page shown before: its category and the table it opened on.
    let mut last = use_signal(|| (String::new(), None::<PanelTable>));
    use_effect(use_reactive((&section, &tool), move |(section, tool)| {
        let page = PanelTable::for_tool(&section, &tool);
        let (last_section, last_page) = last.peek().clone();
        let new_category = last_section != section;
        let own_table = page != PanelTable::for_section(&section);
        let left_own_table = last_page != PanelTable::for_section(&last_section);
        if new_category || own_table || left_own_table {
            if let Some(table) = page {
                panel_table.set(table);
            }
        }
        last.set((section, page));
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

            HistoryLayer {}
            StatusBar {}
            ToastHost {}
            PythonAutoSetup {}
        }
    }
}

