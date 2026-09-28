//! User settings shared by every frontend, stored in the user's config folder
//! (e.g. `%APPDATA%\modeloid\settings.toml` on Windows), not in a workspace.

use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub use crate::core::download::ROBOFLOW_KEY_ENV;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    roboflow_api_key: Option<String>,
}

impl Settings {
    /// `<config dir>/modeloid/settings.toml`; `None` if the OS has no config folder.
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("modeloid").join("settings.toml"))
    }

    /// Reads `path`; a missing file gives default settings.
    pub fn load(path: &Path) -> Result<Self, crate::Error> {
        match fs::read_to_string(path) {
            Ok(text) => Ok(toml::from_str(&text).map_err(|e| format!("{} is invalid: {e}", path.display()))?),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(format!("cannot read {}: {e}", path.display()).into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), crate::Error> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, toml::to_string(self)?).map_err(|e| format!("cannot save {}: {e}", path.display()))?;
        Ok(())
    }

    pub fn roboflow_api_key(&self) -> Option<&str> {
        self.roboflow_api_key.as_deref()
    }

    /// Saves a trimmed, validated key, or removes it with `None`.
    pub fn set_roboflow_api_key(&mut self, key: Option<&str>) -> Result<(), String> {
        self.roboflow_api_key = match key {
            Some(key) => {
                validate_api_key(key)?;
                Some(key.trim().to_string())
            }
            None => None,
        };
        Ok(())
    }
}

/// The Roboflow key saved in the default settings file, if any. For
/// `DownloadOptions.api_key`; with `None` the core falls back to `ROBOFLOW_API_KEY`.
pub fn saved_roboflow_api_key() -> Option<String> {
    let settings = Settings::load(&Settings::default_path()?).ok()?;
    settings.roboflow_api_key
}

/// Checks the shape of an API key before saving it.
pub fn validate_api_key(key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("Paste your API key.".into());
    }
    if key.chars().any(char::is_whitespace) {
        return Err("The key can't contain spaces.".into());
    }
    if !(8..=128).contains(&key.len()) {
        return Err("That doesn't look like an API key. Check you copied all of it.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_keys() {
        assert!(validate_api_key("abcDEF1234567890").is_ok());
        assert!(validate_api_key("  abcDEF1234567890  ").is_ok());
        assert!(validate_api_key("").is_err());
        assert!(validate_api_key("abc def 1234").is_err());
        assert!(validate_api_key("short").is_err());
    }

    #[test]
    fn saves_loads_and_removes_key() {
        let dir = std::env::temp_dir().join(format!("modeloid-settings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("settings.toml");

        let mut settings = Settings::load(&path).unwrap();
        assert_eq!(settings.roboflow_api_key(), None);

        settings.set_roboflow_api_key(Some(" abcdefgh1234 ")).unwrap();
        settings.save(&path).unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("roboflow_api_key = \"abcdefgh1234\""));
        assert_eq!(Settings::load(&path).unwrap().roboflow_api_key(), Some("abcdefgh1234"));

        assert!(settings.set_roboflow_api_key(Some("bad key")).is_err());

        settings.set_roboflow_api_key(None).unwrap();
        settings.save(&path).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("roboflow"));

        let _ = fs::remove_dir_all(&dir);
    }
}
