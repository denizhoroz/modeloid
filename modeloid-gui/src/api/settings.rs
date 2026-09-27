//! App settings, stored on the server machine in the user's config folder
//! (e.g. `%APPDATA%\modeloid\settings.toml`). Secrets never go back to the
//! browser in full; the UI only gets a `KeyStatus`.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// What the UI may know about a stored API key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyStatus {
    /// Last 4 characters of the saved key, if one is saved.
    pub saved_hint: Option<String>,
    /// No key saved, but the matching environment variable is set.
    pub from_env: bool,
    /// Where the settings file lives, shown to the user.
    pub settings_path: String,
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

#[get("/api/settings/roboflow-key")]
pub async fn roboflow_key_status() -> Result<KeyStatus, ServerFnError> {
    backend::roboflow_status(&backend::settings_path()?).map_err(ServerFnError::new)
}

#[post("/api/settings/roboflow-key")]
pub async fn save_roboflow_key(key: String) -> Result<KeyStatus, ServerFnError> {
    let path = backend::settings_path()?;
    backend::set_roboflow_key(&path, Some(key)).map_err(ServerFnError::new)
}

#[post("/api/settings/roboflow-key/remove")]
pub async fn remove_roboflow_key() -> Result<KeyStatus, ServerFnError> {
    let path = backend::settings_path()?;
    backend::set_roboflow_key(&path, None).map_err(ServerFnError::new)
}

#[cfg(feature = "server")]
pub use backend::roboflow_api_key;

#[cfg(feature = "server")]
mod backend {
    use std::fs;
    use std::path::{Path, PathBuf};

    use dioxus::prelude::ServerFnError;
    use serde::{Deserialize, Serialize};

    use super::{validate_api_key, KeyStatus};

    const ROBOFLOW_KEY_ENV: &str = "ROBOFLOW_API_KEY";

    #[derive(Debug, Default, Serialize, Deserialize)]
    struct Settings {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        roboflow_api_key: Option<String>,
    }

    pub fn settings_path() -> Result<PathBuf, ServerFnError> {
        dirs::config_dir()
            .map(|dir| dir.join("modeloid").join("settings.toml"))
            .ok_or_else(|| ServerFnError::new("Couldn't find a config folder on this computer."))
    }

    fn read(path: &Path) -> Result<Settings, String> {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{} is invalid: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(format!("Couldn't read {}: {e}", path.display())),
        }
    }

    fn write(path: &Path, settings: &Settings) -> Result<(), String> {
        let save = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            let text = toml::to_string(settings).map_err(std::io::Error::other)?;
            fs::write(path, text)
        };
        save().map_err(|e| format!("Couldn't save {}: {e}", path.display()))
    }

    pub fn roboflow_status(path: &Path) -> Result<KeyStatus, String> {
        let key = read(path)?.roboflow_api_key;
        Ok(KeyStatus {
            saved_hint: key.as_deref().map(|k| {
                let tail: Vec<char> = k.chars().rev().take(4).collect();
                tail.into_iter().rev().collect()
            }),
            from_env: key.is_none() && std::env::var_os(ROBOFLOW_KEY_ENV).is_some(),
            settings_path: path.display().to_string(),
        })
    }

    pub fn set_roboflow_key(path: &Path, key: Option<String>) -> Result<KeyStatus, String> {
        let key = match key {
            Some(key) => {
                validate_api_key(&key)?;
                Some(key.trim().to_string())
            }
            None => None,
        };
        let mut settings = read(path)?;
        settings.roboflow_api_key = key;
        write(path, &settings)?;
        roboflow_status(path)
    }

    /// The saved Roboflow key, for `DownloadOptions.api_key`. `None` lets the
    /// core fall back to the `ROBOFLOW_API_KEY` environment variable.
    pub fn roboflow_api_key() -> Option<String> {
        let path = settings_path().ok()?;
        read(&path).ok()?.roboflow_api_key
    }
}

#[cfg(test)]
mod tests {
    use super::validate_api_key;

    #[test]
    fn validates_keys() {
        assert!(validate_api_key("abcDEF1234567890").is_ok());
        assert!(validate_api_key("  abcDEF1234567890  ").is_ok());
        assert!(validate_api_key("").is_err());
        assert!(validate_api_key("abc def 1234").is_err());
        assert!(validate_api_key("short").is_err());
    }
}

#[cfg(all(test, feature = "server"))]
mod server_tests {
    use super::backend;

    #[test]
    fn save_status_remove() {
        let dir = std::env::temp_dir().join(format!("modeloid-gui-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("settings.toml");

        assert_eq!(backend::roboflow_status(&path).unwrap().saved_hint, None);

        let status = backend::set_roboflow_key(&path, Some(" abcdefgh1234 ".into())).unwrap();
        assert_eq!(status.saved_hint.as_deref(), Some("1234"));
        assert!(!status.from_env);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("roboflow_api_key = \"abcdefgh1234\""));

        assert!(backend::set_roboflow_key(&path, Some("bad key".into())).is_err());

        let status = backend::set_roboflow_key(&path, None).unwrap();
        assert_eq!(status.saved_hint, None);
        assert!(!std::fs::read_to_string(&path).unwrap().contains("roboflow"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
