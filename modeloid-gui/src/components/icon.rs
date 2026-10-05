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
    Warning,
    Pointer,
    Square,
    Hand,
    Trash,
    Undo,
    Redo,
    ZoomIn,
    ZoomOut,
    Fit,
    ChevronLeft,
    ChevronRight,
    History,
    // Section and table titles (`TitleIcon`)
    Chart,
    Save,
    Sliders,
    Tag,
    Swap,
    Split,
    Scale,
    Filter,
    FileText,
    Image,
    Pencil,
    Download,
    Upload,
    Link,
    Activity,
    Play,
    Terminal,
    TrendingUp,
    Package,
    Wand,
    Target,
    Palette,
    Key,
    Code,
}

/// Icon at the start of a section or table title; hidden with the rest
/// when Appearance > Icons is off (`[data-icons="off"] .title-icon`).
#[component]
pub fn TitleIcon(name: IconName) -> Element {
    rsx! { Icon { name, size: 15, class: "title-icon" } }
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
        IconName::Warning => rsx! {
            path { d: "m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3" }
            path { d: "M12 9v4" }
            path { d: "M12 17h.01" }
        },
        IconName::Refresh => rsx! {
            path { d: "M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8" }
            path { d: "M21 3v5h-5" }
        },
        IconName::Pointer => rsx! {
            path { d: "M4.037 4.688a.495.495 0 0 1 .651-.651l16 6.5a.5.5 0 0 1-.063.947l-6.124 1.58a2 2 0 0 0-1.438 1.435l-1.579 6.126a.5.5 0 0 1-.947.063z" }
        },
        IconName::Square => rsx! {
            rect { width: "18", height: "18", x: "3", y: "3", rx: "2" }
        },
        IconName::Hand => rsx! {
            path { d: "M18 11V6a2 2 0 0 0-2-2a2 2 0 0 0-2 2" }
            path { d: "M14 10V4a2 2 0 0 0-2-2a2 2 0 0 0-2 2v2" }
            path { d: "M10 10.5V6a2 2 0 0 0-2-2a2 2 0 0 0-2 2v8" }
            path { d: "M18 8a2 2 0 1 1 4 0v6a8 8 0 0 1-8 8h-2c-2.8 0-4.5-.86-5.99-2.34l-3.6-3.6a2 2 0 0 1 2.83-2.82L7 15" }
        },
        IconName::History => rsx! {
            path { d: "M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" }
            path { d: "M3 3v5h5" }
            path { d: "M12 7v5l4 2" }
        },
        IconName::Trash => rsx! {
            path { d: "M3 6h18" }
            path { d: "M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6" }
            path { d: "M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" }
        },
        IconName::Undo => rsx! {
            path { d: "M9 14 4 9l5-5" }
            path { d: "M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11" }
        },
        IconName::Redo => rsx! {
            path { d: "m15 14 5-5-5-5" }
            path { d: "M20 9H9.5A5.5 5.5 0 0 0 4 14.5A5.5 5.5 0 0 0 9.5 20H13" }
        },
        IconName::ZoomIn => rsx! {
            circle { cx: "11", cy: "11", r: "8" }
            path { d: "m21 21-4.3-4.3M11 8v6M8 11h6" }
        },
        IconName::ZoomOut => rsx! {
            circle { cx: "11", cy: "11", r: "8" }
            path { d: "m21 21-4.3-4.3M8 11h6" }
        },
        IconName::Fit => rsx! {
            path { d: "M8 3H5a2 2 0 0 0-2 2v3M21 8V5a2 2 0 0 0-2-2h-3M3 16v3a2 2 0 0 0 2 2h3M16 21h3a2 2 0 0 0 2-2v-3" }
        },
        IconName::ChevronLeft => rsx! {
            path { d: "m15 18-6-6 6-6" }
        },
        IconName::ChevronRight => rsx! {
            path { d: "m9 18 6-6-6-6" }
        },
        IconName::Chart => rsx! {
            path { d: "M3 3v16a2 2 0 0 0 2 2h16M18 17V9M13 17V5M8 17v-3" }
        },
        IconName::Save => rsx! {
            path { d: "M15.2 3a2 2 0 0 1 1.4.6l3.8 3.8a2 2 0 0 1 .6 1.4V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z" }
            path { d: "M17 21v-7a1 1 0 0 0-1-1H8a1 1 0 0 0-1 1v7M7 3v4a1 1 0 0 0 1 1h7" }
        },
        IconName::Sliders => rsx! {
            path { d: "M10 5H3M12 19H3M14 3v4M16 17v4M21 12h-9M21 19h-5M21 5h-7M8 10v4M8 12H3" }
        },
        IconName::Tag => rsx! {
            path { d: "M12.586 2.586A2 2 0 0 0 11.172 2H4a2 2 0 0 0-2 2v7.172a2 2 0 0 0 .586 1.414l8.704 8.704a2.426 2.426 0 0 0 3.42 0l6.58-6.58a2.426 2.426 0 0 0 0-3.42z" }
            circle { cx: "7.5", cy: "7.5", r: ".5", fill: "currentColor" }
        },
        IconName::Swap => rsx! {
            path { d: "m16 3 4 4-4 4M20 7H4M8 21l-4-4 4-4M4 17h16" }
        },
        IconName::Split => rsx! {
            path { d: "M16 3h5v5M8 3H3v5M12 22v-8.3a4 4 0 0 0-1.172-2.872L3 3M15 9l6-6" }
        },
        IconName::Scale => rsx! {
            path { d: "m16 16 3-8 3 8c-.87.65-1.92 1-3 1s-2.13-.35-3-1ZM2 16l3-8 3 8c-.87.65-1.92 1-3 1s-2.13-.35-3-1Z" }
            path { d: "M7 21h10M12 3v18M3 7h2c2 0 5-1 7-2 2 1 5 2 7 2h2" }
        },
        IconName::Filter => rsx! {
            path { d: "M22 3H2l8 9.46V19l4 2v-8.54z" }
        },
        IconName::FileText => rsx! {
            path { d: "M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" }
            path { d: "M14 2v4a2 2 0 0 0 2 2h4M10 9H8M16 13H8M16 17H8" }
        },
        IconName::Image => rsx! {
            rect { width: "18", height: "18", x: "3", y: "3", rx: "2" }
            circle { cx: "9", cy: "9", r: "2" }
            path { d: "m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21" }
        },
        IconName::Pencil => rsx! {
            path { d: "M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z" }
            path { d: "m15 5 4 4" }
        },
        IconName::Download => rsx! {
            path { d: "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3" }
        },
        IconName::Upload => rsx! {
            path { d: "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M17 8l-5-5-5 5M12 3v12" }
        },
        IconName::Link => rsx! {
            path { d: "M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" }
            path { d: "M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" }
        },
        IconName::Activity => rsx! {
            path { d: "M22 12h-4l-3 9L9 3l-3 9H2" }
        },
        IconName::Play => rsx! {
            path { d: "M6 3l14 9-14 9z" }
        },
        IconName::Terminal => rsx! {
            path { d: "m4 17 6-6-6-6M12 19h8" }
        },
        IconName::TrendingUp => rsx! {
            path { d: "M22 7l-8.5 8.5-5-5L2 17M16 7h6v6" }
        },
        IconName::Package => rsx! {
            path { d: "M11 21.73a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73z" }
            path { d: "M12 22V12M3.29 7 12 12l8.71-5M7.5 4.27l9 5.15" }
        },
        IconName::Wand => rsx! {
            path { d: "M15 4V2M15 16v-2M8 9h2M20 9h2M17.8 11.8 19 13M15 9h.01M17.8 6.2 19 5M3 21l9-9M12.2 6.2 11 5" }
        },
        IconName::Target => rsx! {
            circle { cx: "12", cy: "12", r: "10" }
            circle { cx: "12", cy: "12", r: "6" }
            circle { cx: "12", cy: "12", r: "2" }
        },
        IconName::Palette => rsx! {
            path { d: "M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.926 0 1.648-.746 1.648-1.688 0-.437-.18-.835-.437-1.125-.29-.289-.438-.652-.438-1.125a1.64 1.64 0 0 1 1.668-1.668h1.996c3.051 0 5.555-2.503 5.555-5.554C21.965 6.012 17.461 2 11.996 2z" }
            circle { cx: "13.5", cy: "6.5", r: ".5", fill: "currentColor" }
            circle { cx: "17.5", cy: "10.5", r: ".5", fill: "currentColor" }
            circle { cx: "8.5", cy: "7.5", r: ".5", fill: "currentColor" }
            circle { cx: "6.5", cy: "12.5", r: ".5", fill: "currentColor" }
        },
        IconName::Key => rsx! {
            path { d: "M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z" }
            circle { cx: "16.5", cy: "7.5", r: ".5", fill: "currentColor" }
        },
        IconName::Code => rsx! {
            path { d: "m16 18 6-6-6-6M8 6l-6 6 6 6" }
        },
        IconName::Wrench => rsx! {
            path { d: "M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" }
        },
    }
}
