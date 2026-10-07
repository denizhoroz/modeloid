//! App settings, stored by the core (`modeloid::Settings`) on the server machine
//! in the user's config folder (e.g. `%APPDATA%\modeloid\settings.toml`), shared
//! by every workspace; the credentials themselves in the system's credential
//! store. Secrets never go back to the browser in full; the UI only gets a `KeyStatus`.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// A service with saved credentials (mirrors the core's `Service`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceId {
    Roboflow,
    Kaggle,
    HuggingFace,
}

impl ServiceId {
    pub const ALL: [ServiceId; 3] = [ServiceId::Roboflow, ServiceId::Kaggle, ServiceId::HuggingFace];

    pub fn label(self) -> &'static str {
        match self {
            ServiceId::Roboflow => "Roboflow",
            ServiceId::Kaggle => "Kaggle",
            ServiceId::HuggingFace => "Hugging Face",
        }
    }

    /// Name of the credential, e.g. "Roboflow API key".
    pub fn credential(self) -> &'static str {
        match self {
            ServiceId::Roboflow => "Roboflow API key",
            ServiceId::Kaggle => "Kaggle API credentials",
            ServiceId::HuggingFace => "Hugging Face token",
        }
    }

    /// Short, URL-safe id for element ids.
    pub fn slug(self) -> &'static str {
        match self {
            ServiceId::Roboflow => "roboflow",
            ServiceId::Kaggle => "kaggle",
            ServiceId::HuggingFace => "huggingface",
        }
    }
}

/// What the UI may know about stored credentials.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyStatus {
    /// Last 4 characters of a saved key, or the saved Kaggle username.
    pub saved_hint: Option<String>,
    /// Nothing saved, but this is used instead, e.g. "ROBOFLOW_API_KEY env var".
    pub fallback: Option<String>,
    /// Where saved credentials are kept, e.g. "Windows Credential Manager", or
    /// the settings file's path when there's no credential store.
    pub stored_in: String,
}

pub use modeloid_types::validate_api_key;

/// Quick form check; the server parses Kaggle credentials fully.
pub fn validate_credential(service: ServiceId, value: &str) -> Result<(), String> {
    let value = value.trim();
    match service {
        ServiceId::Kaggle if value.starts_with('{') => {
            if value.contains("\"username\"") && value.contains("\"key\"") {
                Ok(())
            } else {
                Err("That isn't a valid kaggle.json. It needs \"username\" and \"key\".".into())
            }
        }
        ServiceId::Kaggle if value.starts_with("KGAT_") => validate_api_key(value),
        ServiceId::Kaggle => match value.split_once(':') {
            Some((user, key)) if !user.trim().is_empty() => validate_api_key(key),
            _ => Err("Kaggle API tokens start with KGAT_. A legacy key needs your username too: username:key, or the contents of kaggle.json.".into()),
        },
        _ => validate_api_key(value),
    }
}

#[post("/api/settings/credentials/status")]
pub async fn credential_status(service: ServiceId) -> Result<KeyStatus, ServerFnError> {
    crate::api::blocking(move || backend::status(&backend::settings_path()?, service)).await
}

#[post("/api/settings/credentials/save")]
pub async fn save_credential(service: ServiceId, value: String) -> Result<KeyStatus, ServerFnError> {
    crate::api::blocking(move || backend::set(&backend::settings_path()?, service, Some(value))).await
}

#[post("/api/settings/credentials/remove")]
pub async fn remove_credential(service: ServiceId) -> Result<KeyStatus, ServerFnError> {
    crate::api::blocking(move || backend::set(&backend::settings_path()?, service, None)).await
}

#[cfg(feature = "server")]
pub use backend::settings_path;

#[cfg(feature = "server")]
mod backend {
    use std::path::{Path, PathBuf};

    
    use modeloid::{Service, Settings};

    use super::{KeyStatus, ServiceId};

    fn core(service: ServiceId) -> Service {
        match service {
            ServiceId::Roboflow => Service::Roboflow,
            ServiceId::Kaggle => Service::Kaggle,
            ServiceId::HuggingFace => Service::HuggingFace,
        }
    }

    /// Where the settings live (`Settings::default_path`).
    pub fn settings_path() -> Result<PathBuf, String> {
        Settings::default_path().ok_or_else(|| "Couldn't find a config folder on this computer.".into())
    }

    fn load(path: &Path) -> Result<Settings, String> {
        Settings::load(path).map_err(|e| format!("Couldn't read the settings: {e}"))
    }

    pub fn status(path: &Path, service: ServiceId) -> Result<KeyStatus, String> {
        let settings = load(path)?;
        let saved_hint = settings.credential_hint(core(service));
        Ok(KeyStatus {
            fallback: if saved_hint.is_none() { core(service).fallback() } else { None },
            saved_hint,
            stored_in: settings.credential_store().map_or_else(|| path.display().to_string(), str::to_string),
        })
    }

    pub fn set(path: &Path, service: ServiceId, value: Option<String>) -> Result<KeyStatus, String> {
        let mut settings = load(path)?;
        settings.set_credential(core(service), value.as_deref())?;
        settings.save(path).map_err(|e| format!("Couldn't save the settings: {e}"))?;
        status(path, service)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_kaggle_credentials() {
        assert!(validate_credential(ServiceId::Kaggle, "someone:0123456789abcdef").is_ok());
        assert!(validate_credential(ServiceId::Kaggle, r#"{"username":"someone","key":"0123456789abcdef"}"#).is_ok());
        assert!(validate_credential(ServiceId::Kaggle, "0123456789abcdef").is_err());
        assert!(validate_credential(ServiceId::Kaggle, r#"{"username":"someone"}"#).is_err());
        assert!(validate_credential(ServiceId::HuggingFace, "hf_0123456789abcdef").is_ok());
    }
}

#[cfg(all(test, feature = "server"))]
mod server_tests {
    use super::{backend, ServiceId};

    #[test]
    fn save_status_remove() {
        let dir = std::env::temp_dir().join(format!("modeloid-gui-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("settings.toml");

        assert_eq!(backend::status(&path, ServiceId::Roboflow).unwrap().saved_hint, None);

        let status = backend::set(&path, ServiceId::Roboflow, Some(" abcdefgh1234 ".into())).unwrap();
        assert_eq!(status.saved_hint.as_deref(), Some("1234"));
        assert_eq!(status.fallback, None);

        let status = backend::set(&path, ServiceId::Kaggle, Some("someone:0123456789abcdef".into())).unwrap();
        assert_eq!(status.saved_hint.as_deref(), Some("someone"));

        assert!(backend::set(&path, ServiceId::HuggingFace, Some("bad key".into())).is_err());

        let status = backend::set(&path, ServiceId::Roboflow, None).unwrap();
        assert_eq!(status.saved_hint, None);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("roboflow") && text.contains("someone"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
