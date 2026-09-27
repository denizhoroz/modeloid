//! Bundled web fonts. JetBrains Mono (SIL OFL 1.1, see assets/fonts/OFL.txt)
//! ships with the app so it works offline. `--font-sans` in main.css lists
//! the lookalike fallbacks.

use dioxus::prelude::*;

const JETBRAINS_MONO: Asset = asset!("/assets/fonts/JetBrainsMono-Variable.woff2");
const JETBRAINS_MONO_ITALIC: Asset = asset!("/assets/fonts/JetBrainsMono-Variable-Italic.woff2");

/// Registers the font faces and preloads the regular one. Render once at the root.
#[component]
pub fn Fonts() -> Element {
    rsx! {
        document::Link {
            rel: "preload",
            href: JETBRAINS_MONO,
            r#as: "font",
            r#type: "font/woff2",
            crossorigin: "anonymous",
        }
        document::Style {
            "@font-face {{
                font-family: 'JetBrains Mono';
                font-style: normal;
                font-weight: 100 800;
                font-display: swap;
                src: url('{JETBRAINS_MONO}') format('woff2');
            }}
            @font-face {{
                font-family: 'JetBrains Mono';
                font-style: italic;
                font-weight: 100 800;
                font-display: swap;
                src: url('{JETBRAINS_MONO_ITALIC}') format('woff2');
            }}"
        }
    }
}
