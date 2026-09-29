//! Bundled web fonts (all SIL OFL 1.1, see assets/fonts/OFL*.txt), shipped
//! with the app so they work offline. The `[data-font]` blocks in themes.css
//! pick one and list lookalike fallbacks. Browsers only download a face once
//! text uses it, so registering all of them is cheap.

use dioxus::prelude::*;

const JETBRAINS_MONO: Asset = asset!("/assets/fonts/JetBrainsMono-Variable.woff2");
const JETBRAINS_MONO_ITALIC: Asset = asset!("/assets/fonts/JetBrainsMono-Variable-Italic.woff2");
const INTER: Asset = asset!("/assets/fonts/Inter-Variable-latin.woff2");
const INTER_EXT: Asset = asset!("/assets/fonts/Inter-Variable-latin-ext.woff2");
const PLEX_SANS: Asset = asset!("/assets/fonts/IBMPlexSans-Variable-latin.woff2");
const PLEX_SANS_EXT: Asset = asset!("/assets/fonts/IBMPlexSans-Variable-latin-ext.woff2");
const ATKINSON: Asset = asset!("/assets/fonts/AtkinsonHyperlegibleNext-Variable-latin.woff2");
const ATKINSON_EXT: Asset = asset!("/assets/fonts/AtkinsonHyperlegibleNext-Variable-latin-ext.woff2");

/// Unicode ranges of the split (Latin / Latin Extended) files, as Google Fonts cuts them.
const LATIN: &str = "U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+02C6, U+02DA, U+02DC, U+0304, U+0308, U+0329, U+2000-206F, U+20AC, U+2122, U+2191, U+2193, U+2212, U+2215, U+FEFF, U+FFFD";
const LATIN_EXT: &str = "U+0100-02BA, U+02BD-02C5, U+02C7-02CC, U+02CE-02D7, U+02DD-02FF, U+0304, U+0308, U+0329, U+1D00-1DBF, U+1E00-1E9F, U+1EF2-1EFF, U+2020, U+20A0-20AB, U+20AD-20C0, U+2113, U+2C60-2C7F, U+A720-A7FF";

/// The file to preload for a `data-font` id (the one the first paint needs).
fn preload(font: &str) -> Asset {
    match font {
        "inter" => INTER,
        "ibm-plex-sans" => PLEX_SANS,
        "atkinson" => ATKINSON,
        _ => JETBRAINS_MONO,
    }
}

/// Registers the font faces and preloads the `active` one. Render once at the root.
#[component]
pub fn Fonts(active: String) -> Element {
    rsx! {
        document::Link {
            rel: "preload",
            href: preload(&active),
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
            }}
            @font-face {{
                font-family: 'Inter';
                font-weight: 100 900;
                font-display: swap;
                src: url('{INTER}') format('woff2');
                unicode-range: {LATIN};
            }}
            @font-face {{
                font-family: 'Inter';
                font-weight: 100 900;
                font-display: swap;
                src: url('{INTER_EXT}') format('woff2');
                unicode-range: {LATIN_EXT};
            }}
            @font-face {{
                font-family: 'IBM Plex Sans';
                font-weight: 100 700;
                font-display: swap;
                src: url('{PLEX_SANS}') format('woff2');
                unicode-range: {LATIN};
            }}
            @font-face {{
                font-family: 'IBM Plex Sans';
                font-weight: 100 700;
                font-display: swap;
                src: url('{PLEX_SANS_EXT}') format('woff2');
                unicode-range: {LATIN_EXT};
            }}
            @font-face {{
                font-family: 'Atkinson Hyperlegible Next';
                font-weight: 200 800;
                font-display: swap;
                src: url('{ATKINSON}') format('woff2');
                unicode-range: {LATIN};
            }}
            @font-face {{
                font-family: 'Atkinson Hyperlegible Next';
                font-weight: 200 800;
                font-display: swap;
                src: url('{ATKINSON_EXT}') format('woff2');
                unicode-range: {LATIN_EXT};
            }}"
        }
    }
}
