//! Inline SVG icons (paths from Lucide, https://lucide.dev, ISC license).
//! To add one: add a variant to `IconName` and its shapes to `shapes`.

use dioxus::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IconName {
    Logo,
    FolderOpen,
    Plus,
    Database,
    Cpu,
    ChevronDown,
    Menu,
    Wrench,
    Settings,
    Eye,
    Refresh,
    EyeOff,
}

/// Decorative icon, hidden from screen readers. Pair it with visible text
/// or put an `aria_label` on the surrounding button.
#[component]
pub fn Icon(
    name: IconName,
    #[props(default = 20)] size: u32,
    #[props(into, default)] class: String,
) -> Element {
    rsx! {
        svg {
            class: "icon {class}",
            width: "{size}",
            height: "{size}",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            "focusable": "false",
            {shapes(name)}
        }
    }
}

fn shapes(name: IconName) -> Element {
    match name {
        IconName::Logo => rsx! {
            path { d: "M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" }
            path { d: "m3.3 7 8.7 5 8.7-5" }
            path { d: "M12 22V12" }
        },
        IconName::FolderOpen => rsx! {
            path { d: "m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2" }
        },
        IconName::Plus => rsx! {
            path { d: "M5 12h14M12 5v14" }
        },
        IconName::Database => rsx! {
            ellipse { cx: "12", cy: "5", rx: "9", ry: "3" }
            path { d: "M3 5V19A9 3 0 0 0 21 19V5" }
            path { d: "M3 12A9 3 0 0 0 21 12" }
        },
        IconName::Cpu => rsx! {
            rect { width: "16", height: "16", x: "4", y: "4", rx: "2" }
            rect { width: "6", height: "6", x: "9", y: "9", rx: "1" }
            path { d: "M15 2v2M15 20v2M2 15h2M2 9h2M20 15h2M20 9h2M9 2v2M9 20v2" }
        },
        IconName::ChevronDown => rsx! {
            path { d: "m6 9 6 6 6-6" }
        },
        IconName::Menu => rsx! {
            path { d: "M4 6h16M4 12h16M4 18h16" }
        },
        IconName::Settings => rsx! {
            path { d: "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" }
            circle { cx: "12", cy: "12", r: "3" }
        },
        IconName::Eye => rsx! {
            path { d: "M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0" }
            circle { cx: "12", cy: "12", r: "3" }
        },
        IconName::EyeOff => rsx! {
            path { d: "M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49" }
            path { d: "M14.084 14.158a3 3 0 0 1-4.242-4.242" }
            path { d: "M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143" }
            path { d: "m2 2 20 20" }
        },
        IconName::Refresh => rsx! {
            path { d: "M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8" }
            path { d: "M21 3v5h-5" }
        },
        IconName::Wrench => rsx! {
            path { d: "M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" }
        },
    }
}
