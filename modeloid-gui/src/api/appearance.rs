//! How the app looks: theme, font, text size and whether titles show icons. Stored by the core in
//! `settings.toml` (`[appearance]`) so it survives browser changes, and read
//! during server rendering so the first paint already uses it.
//!
//! Adding a theme: one `[data-theme="…"]` block in `assets/themes.css` plus
//! one entry in `THEMES`. Adding a font: files + `@font-face` in
//! `components/fonts.rs`, a `[data-font="…"]` block in themes.css, and an
//! entry in `FONTS`.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

pub struct ThemeInfo {
    /// Value of `data-theme`, also what's saved.
    pub id: &'static str,
    pub label: &'static str,
    pub dark: bool,
}

pub const THEMES: &[ThemeInfo] = &[
    ThemeInfo { id: "lime", label: "Lime", dark: true },
    ThemeInfo { id: "catppuccin", label: "Catppuccin Mocha", dark: true },
    ThemeInfo { id: "rose-pine", label: "Rosé Pine", dark: true },
    ThemeInfo { id: "tokyo-night", label: "Tokyo Night", dark: true },
    ThemeInfo { id: "gruvbox", label: "Gruvbox Dark", dark: true },
    ThemeInfo { id: "dracula", label: "Dracula", dark: true },
    ThemeInfo { id: "nord", label: "Nord", dark: true },
    ThemeInfo { id: "everforest", label: "Everforest Dark", dark: true },
    ThemeInfo { id: "lime-light", label: "Lime Light", dark: false },
    ThemeInfo { id: "catppuccin-latte", label: "Catppuccin Latte", dark: false },
    ThemeInfo { id: "rose-pine-dawn", label: "Rosé Pine Dawn", dark: false },
    ThemeInfo { id: "tokyo-night-day", label: "Tokyo Night Day", dark: false },
    ThemeInfo { id: "gruvbox-light", label: "Gruvbox Light", dark: false },
    ThemeInfo { id: "nord-light", label: "Nord Light", dark: false },
    ThemeInfo { id: "everforest-light", label: "Everforest Light", dark: false },
    ThemeInfo { id: "solarized-light", label: "Solarized Light", dark: false },
];

pub struct FontInfo {
    /// Value of `data-font`, also what's saved.
    pub id: &'static str,
    pub label: &'static str,
    pub note: &'static str,
}

pub const FONTS: &[FontInfo] = &[
    FontInfo { id: "jetbrains-mono", label: "JetBrains Mono", note: "Monospace" },
    FontInfo { id: "inter", label: "Inter", note: "Sans-serif" },
    FontInfo { id: "ibm-plex-sans", label: "IBM Plex Sans", note: "Sans-serif" },
    FontInfo { id: "atkinson", label: "Atkinson Hyperlegible", note: "Made for low vision" },
];

/// Text size; `data-scale` sets `--font-scale`.
pub const SCALES: [(&str, &str); 3] = [("small", "Small"), ("default", "Default"), ("large", "Large")];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Appearance {
    pub theme: String,
    pub font: String,
    pub scale: String,
    /// Section and table titles without their icons (`data-icons="off"`).
    #[serde(default)]
    pub hide_icons: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance { theme: THEMES[0].id.into(), font: FONTS[0].id.into(), scale: "default".into(), hide_icons: false }
    }
}

impl Appearance {
    /// Unknown ids (a removed theme, a hand-edited file) become the defaults.
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    pub fn normalized(self) -> Self {
        let default = Appearance::default();
        Appearance {
            theme: if THEMES.iter().any(|t| t.id == self.theme) { self.theme } else { default.theme },
            font: if FONTS.iter().any(|f| f.id == self.font) { self.font } else { default.font },
            scale: if SCALES.iter().any(|(id, _)| *id == self.scale) { self.scale } else { default.scale },
            hide_icons: self.hide_icons,
        }
    }
}

#[post("/api/settings/appearance")]
pub async fn get_appearance() -> Result<Appearance, ServerFnError> {
    crate::api::blocking(|| backend::get(&backend::settings_path()?)).await
}

#[post("/api/settings/appearance/save")]
pub async fn save_appearance(appearance: Appearance) -> Result<(), ServerFnError> {
    crate::api::blocking(move || backend::save(&backend::settings_path()?, appearance)).await
}

#[cfg(feature = "server")]
mod backend {
    use std::path::Path;

    
    use modeloid::{AppearancePrefs, Settings};

    use super::Appearance;

    pub use crate::api::settings::settings_path;

    pub fn get(path: &Path) -> Result<Appearance, String> {
        let settings = Settings::load(path).map_err(|e| format!("Couldn't read the settings: {e}"))?;
        Ok(settings
            .appearance()
            .map(|p| Appearance { theme: p.theme.clone(), font: p.font.clone(), scale: p.scale.clone(), hide_icons: p.hide_icons }.normalized())
            .unwrap_or_default())
    }

    pub fn save(path: &Path, appearance: Appearance) -> Result<(), String> {
        let Appearance { theme, font, scale, hide_icons } = appearance.normalized();
        let mut settings = Settings::load(path).map_err(|e| format!("Couldn't read the settings: {e}"))?;
        settings.set_appearance(AppearancePrefs { theme, font, scale, hide_icons });
        settings.save(path).map_err(|e| format!("Couldn't save the settings: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_ids_fall_back_to_defaults() {
        let odd = Appearance { theme: "gone".into(), font: "inter".into(), scale: "huge".into(), hide_icons: false };
        let fixed = odd.normalized();
        assert_eq!(fixed.theme, "lime");
        assert_eq!(fixed.font, "inter");
        assert_eq!(fixed.scale, "default");
    }
}

#[cfg(all(test, feature = "server"))]
mod server_tests {
    use super::{backend, Appearance};

    #[test]
    fn save_and_get() {
        let dir = std::env::temp_dir().join(format!("modeloid-gui-appearance-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.toml");

        assert_eq!(backend::get(&path).unwrap(), Appearance::default());
        let chosen = Appearance { theme: "dracula".into(), font: "inter".into(), scale: "large".into(), hide_icons: true };
        backend::save(&path, chosen.clone()).unwrap();
        assert_eq!(backend::get(&path).unwrap(), chosen);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
