//! User settings shared by every frontend, stored in the user's config folder
//! (e.g. `%APPDATA%\modeloid\settings.toml` on Windows), not in a workspace.
//! Holds credentials for the services datasets are downloaded from.

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
};

use crate::core::download::DownloadOptions;
use crate::core::limits::{self, SystemPrefs};

pub const ROBOFLOW_KEY_ENV: &str = "ROBOFLOW_API_KEY";
pub const KAGGLE_USERNAME_ENV: &str = "KAGGLE_USERNAME";
pub const KAGGLE_KEY_ENV: &str = "KAGGLE_KEY";
pub const HF_TOKEN_ENV: &str = "HF_TOKEN";

/// A service that needs credentials to download from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Service {
    Roboflow,
    Kaggle,
    HuggingFace,
}

impl Service {
    pub const ALL: [Service; 3] = [Service::Roboflow, Service::Kaggle, Service::HuggingFace];

    pub fn label(self) -> &'static str {
        match self {
            Service::Roboflow => "Roboflow",
            Service::Kaggle => "Kaggle",
            Service::HuggingFace => "Hugging Face",
        }
    }

    /// Where credentials come from when none are saved, if that source is set
    /// on this machine, e.g. "ROBOFLOW_API_KEY env var".
    pub fn fallback(self) -> Option<String> {
        let set = |name: &str| env::var_os(name).is_some_and(|v| !v.is_empty());
        match self {
            Service::Roboflow => set(ROBOFLOW_KEY_ENV).then(|| format!("{ROBOFLOW_KEY_ENV} env var")),
            Service::Kaggle if set(KAGGLE_USERNAME_ENV) && set(KAGGLE_KEY_ENV) => {
                Some(format!("{KAGGLE_USERNAME_ENV}/{KAGGLE_KEY_ENV} env vars"))
            }
            Service::Kaggle => KaggleCredentials::from_kaggle_json().map(|(_, path)| path.display().to_string()),
            Service::HuggingFace => set(HF_TOKEN_ENV).then(|| format!("{HF_TOKEN_ENV} env var")),
        }
    }
}

/// Kaggle API credentials, as in `kaggle.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KaggleCredentials {
    pub username: String,
    pub key: String,
}

impl KaggleCredentials {
    /// Accepts the contents of `kaggle.json` or `username:key`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let creds = if text.starts_with('{') {
            serde_json::from_str::<KaggleCredentials>(text)
                .map_err(|_| "That isn't a valid kaggle.json. It needs \"username\" and \"key\".".to_string())?
        } else {
            let (username, key) = text
                .split_once(':')
                .ok_or("Paste the contents of kaggle.json, or username:key.")?;
            KaggleCredentials { username: username.trim().into(), key: key.trim().into() }
        };
        if creds.username.is_empty() || creds.username.chars().any(char::is_whitespace) {
            return Err("The Kaggle username is missing or has spaces.".into());
        }
        validate_api_key(&creds.key)?;
        Ok(creds)
    }

    pub fn from_env() -> Option<Self> {
        let username = env::var(KAGGLE_USERNAME_ENV).ok().filter(|v| !v.is_empty())?;
        let key = env::var(KAGGLE_KEY_ENV).ok().filter(|v| !v.is_empty())?;
        Some(KaggleCredentials { username, key })
    }

    /// The Kaggle CLI's `kaggle.json` (`$KAGGLE_CONFIG_DIR` or `~/.kaggle`) and its path.
    pub fn from_kaggle_json() -> Option<(Self, PathBuf)> {
        let dir = env::var_os("KAGGLE_CONFIG_DIR").map(PathBuf::from).or_else(|| dirs::home_dir().map(|h| h.join(".kaggle")))?;
        let path = dir.join("kaggle.json");
        let creds = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
        Some((creds, path))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    roboflow_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    huggingface_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kaggle: Option<KaggleCredentials>,
    /// Folder each kind of file dialog opened in last (e.g. "workspace"), so
    /// the next one starts there.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    last_folders: BTreeMap<String, PathBuf>,
    /// How the GUI looks. The ids mean nothing to the core; the GUI falls back
    /// to its defaults for ones it doesn't know.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    appearance: Option<AppearancePrefs>,
    /// What training runs on, chosen in Settings → Python environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    train_device: Option<modeloid_types::pyenv::TrainDevice>,
    /// How much of this computer Modeloid uses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    system: Option<SystemPrefs>,
}

/// GUI look: theme, font family and text size, as ids.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppearancePrefs {
    #[serde(default)]
    pub theme: String,
    #[serde(default)]
    pub font: String,
    #[serde(default)]
    pub scale: String,
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

    /// Settings from the default file; defaults if it's missing or unreadable.
    pub fn load_default() -> Self {
        Self::default_path().and_then(|p| Self::load(&p).ok()).unwrap_or_default()
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

    pub fn huggingface_token(&self) -> Option<&str> {
        self.huggingface_token.as_deref()
    }

    pub fn kaggle(&self) -> Option<&KaggleCredentials> {
        self.kaggle.as_ref()
    }

    /// Where the `purpose` dialog should start, if that folder still exists.
    pub fn last_folder(&self, purpose: &str) -> Option<&Path> {
        self.last_folders.get(purpose).map(PathBuf::as_path).filter(|p| p.is_dir())
    }

    pub fn set_last_folder(&mut self, purpose: &str, dir: PathBuf) {
        self.last_folders.insert(purpose.to_string(), dir);
    }

    pub fn appearance(&self) -> Option<&AppearancePrefs> {
        self.appearance.as_ref()
    }

    pub fn set_appearance(&mut self, prefs: AppearancePrefs) {
        self.appearance = Some(prefs);
    }

    pub fn train_device(&self) -> Option<modeloid_types::pyenv::TrainDevice> {
        self.train_device
    }

    pub fn set_train_device(&mut self, device: modeloid_types::pyenv::TrainDevice) {
        self.train_device = Some(device);
    }

    /// System settings: the saved ones, or the defaults.
    pub fn system(&self) -> SystemPrefs {
        self.system.unwrap_or_default().normalized(limits::cores())
    }

    pub fn set_system(&mut self, prefs: SystemPrefs) {
        self.system = Some(prefs.normalized(limits::cores()));
    }

    /// Puts the system settings to use in this process (`core::limits`).
    /// Frontends call it at start, and again after saving a change.
    pub fn apply_system(&self) {
        limits::apply(self.system());
    }

    /// Saves validated credentials for `service`, or removes them with `None`.
    /// Kaggle takes `kaggle.json` contents or `username:key`.
    pub fn set_credential(&mut self, service: Service, value: Option<&str>) -> Result<(), String> {
        match (service, value) {
            (Service::Roboflow, value) => self.roboflow_api_key = value.map(checked_key).transpose()?,
            (Service::HuggingFace, value) => self.huggingface_token = value.map(checked_key).transpose()?,
            (Service::Kaggle, Some(text)) => self.kaggle = Some(KaggleCredentials::parse(text)?),
            (Service::Kaggle, None) => self.kaggle = None,
        }
        Ok(())
    }

    /// Safe to show: the last 4 characters of a key, or the Kaggle username.
    pub fn credential_hint(&self, service: Service) -> Option<String> {
        let tail = |k: &str| k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<String>();
        match service {
            Service::Roboflow => self.roboflow_api_key.as_deref().map(tail),
            Service::HuggingFace => self.huggingface_token.as_deref().map(tail),
            Service::Kaggle => self.kaggle.as_ref().map(|k| k.username.clone()),
        }
    }

    /// Download options using these saved credentials. Missing ones fall back
    /// to env vars (and `kaggle.json`) when the download runs.
    pub fn download_options(&self, format: String) -> DownloadOptions {
        DownloadOptions {
            format,
            api_key: self.roboflow_api_key.clone(),
            kaggle: self.kaggle.clone(),
            hf_token: self.huggingface_token.clone(),
            fork_unversioned: false,
        }
    }
}

fn checked_key(key: &str) -> Result<String, String> {
    validate_api_key(key)?;
    Ok(key.trim().to_string())
}

pub use modeloid_types::validate_api_key;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_system_settings() {
        let mut settings = Settings::default();
        assert_eq!(settings.system(), SystemPrefs::default());
        assert!(!toml::to_string(&settings).unwrap().contains("system"), "defaults aren't written");

        settings.set_system(SystemPrefs { threads: 1, download_connections: 99, max_jobs: 2, thumb_size: 240 });
        let loaded: Settings = toml::from_str(&toml::to_string(&settings).unwrap()).unwrap();
        assert_eq!(loaded.system(), SystemPrefs { threads: 1, download_connections: 16, max_jobs: 2, thumb_size: 240 });

        // A file that names only one setting keeps the defaults for the rest.
        let partial: Settings = toml::from_str("[system]\nmax_jobs = 3\n").unwrap();
        assert_eq!(partial.system(), SystemPrefs { max_jobs: 3, ..Default::default() });
    }

    #[test]
    fn parses_kaggle_credentials() {
        let json = KaggleCredentials::parse(r#"{"username":"deniz","key":"0123456789abcdef"}"#).unwrap();
        assert_eq!(json, KaggleCredentials { username: "deniz".into(), key: "0123456789abcdef".into() });
        assert_eq!(KaggleCredentials::parse(" deniz:0123456789abcdef ").unwrap(), json);
        assert!(KaggleCredentials::parse("0123456789abcdef").is_err());
        assert!(KaggleCredentials::parse(":0123456789abcdef").is_err());
        assert!(KaggleCredentials::parse(r#"{"username":"deniz"}"#).is_err());
    }

    #[test]
    fn saves_loads_and_removes_credentials() {
        let dir = std::env::temp_dir().join(format!("modeloid-settings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("settings.toml");

        let mut settings = Settings::load(&path).unwrap();
        assert_eq!(settings.credential_hint(Service::Roboflow), None);

        settings.set_credential(Service::Roboflow, Some(" abcdefgh1234 ")).unwrap();
        settings.set_credential(Service::Kaggle, Some("deniz:0123456789abcdef")).unwrap();
        settings.save(&path).unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("roboflow_api_key = \"abcdefgh1234\""));

        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.roboflow_api_key(), Some("abcdefgh1234"));
        assert_eq!(loaded.credential_hint(Service::Roboflow).as_deref(), Some("1234"));
        assert_eq!(loaded.credential_hint(Service::Kaggle).as_deref(), Some("deniz"));
        assert_eq!(loaded.download_options("coco".into()).kaggle.unwrap().key, "0123456789abcdef");

        assert!(settings.set_credential(Service::HuggingFace, Some("bad key")).is_err());

        settings.set_credential(Service::Roboflow, None).unwrap();
        settings.set_credential(Service::Kaggle, None).unwrap();
        settings.save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("roboflow") && !text.contains("kaggle"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remembers_existing_folders_only() {
        let mut settings = Settings::default();
        settings.set_last_folder("import", env::temp_dir());
        settings.set_last_folder("gone", env::temp_dir().join("modeloid-surely-missing-dir"));
        assert_eq!(settings.last_folder("import"), Some(env::temp_dir().as_path()));
        assert_eq!(settings.last_folder("gone"), None);
        assert_eq!(settings.last_folder("other"), None);

        let text = toml::to_string(&settings).unwrap();
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), settings);
    }
    #[test]
    fn appearance_round_trips_and_is_optional() {
        let old: Settings = toml::from_str("roboflow_api_key = \"abcdefgh1234\"
").unwrap();
        assert_eq!(old.appearance(), None);

        let mut settings = old.clone();
        let prefs = AppearancePrefs { theme: "dracula".into(), font: "inter".into(), scale: "large".into() };
        settings.set_appearance(prefs.clone());
        let text = toml::to_string(&settings).unwrap();
        assert!(text.contains("[appearance]"));
        let loaded = toml::from_str::<Settings>(&text).unwrap();
        assert_eq!(loaded.appearance(), Some(&prefs));
        assert_eq!(loaded.roboflow_api_key(), Some("abcdefgh1234"));
    }
}
