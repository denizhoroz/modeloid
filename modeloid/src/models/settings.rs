//! User settings shared by every frontend, stored in the user's config folder
//! (e.g. `%APPDATA%\modeloid\settings.toml` on Windows), not in a workspace.
//!
//! Credentials for the services datasets are downloaded from are kept in the
//! system's credential store (Windows Credential Manager, macOS Keychain, the
//! Secret Service on Linux), filed under "modeloid". Only when there is none,
//! or for a settings file other than the default one, do they go in the file.

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
/// Kaggle's newer API token (`KGAT_…`), used on its own.
pub const KAGGLE_TOKEN_ENV: &str = "KAGGLE_API_TOKEN";
/// How Kaggle's API tokens start.
pub const KAGGLE_TOKEN_PREFIX: &str = "KGAT_";
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

    /// Name of its entry in the credential store.
    fn entry(self) -> &'static str {
        match self {
            Service::Roboflow => "roboflow-api-key",
            Service::Kaggle => "kaggle",
            Service::HuggingFace => "huggingface-token",
        }
    }

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
            Service::Kaggle if set(KAGGLE_TOKEN_ENV) => Some(format!("{KAGGLE_TOKEN_ENV} env var")),
            Service::Kaggle if set(KAGGLE_USERNAME_ENV) && set(KAGGLE_KEY_ENV) => {
                Some(format!("{KAGGLE_USERNAME_ENV}/{KAGGLE_KEY_ENV} env vars"))
            }
            Service::Kaggle => KaggleCredentials::from_files().map(|(_, path)| path.display().to_string()),
            Service::HuggingFace => set(HF_TOKEN_ENV).then(|| format!("{HF_TOKEN_ENV} env var")),
        }
    }
}

/// Kaggle API credentials: an API token (`KGAT_…`, sent as a bearer token;
/// `username` is empty), or the legacy username and key of `kaggle.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KaggleCredentials {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub username: String,
    pub key: String,
}

impl KaggleCredentials {
    /// An API token from Kaggle's settings page.
    pub fn token(token: &str) -> Self {
        KaggleCredentials { username: String::new(), key: token.trim().to_string() }
    }

    /// An API token rather than a username and key.
    pub fn is_token(&self) -> bool {
        self.username.is_empty()
    }

    /// Accepts an API token (`KGAT_…`), the contents of `kaggle.json` or `username:key`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.starts_with(KAGGLE_TOKEN_PREFIX) {
            validate_api_key(text)?;
            return Ok(KaggleCredentials::token(text));
        }
        let creds = if text.starts_with('{') {
            serde_json::from_str::<KaggleCredentials>(text)
                .map_err(|_| "That isn't a valid kaggle.json. It needs \"username\" and \"key\".".to_string())?
        } else {
            let (username, key) = text.split_once(':').ok_or(
                "Kaggle API tokens start with KGAT_. A legacy key needs your username too: username:key, or the contents of kaggle.json.",
            )?;
            KaggleCredentials { username: username.trim().into(), key: key.trim().into() }
        };
        if creds.username.is_empty() || creds.username.chars().any(char::is_whitespace) {
            return Err("The Kaggle username is missing or has spaces.".into());
        }
        validate_api_key(&creds.key)?;
        Ok(creds)
    }

    /// `KAGGLE_API_TOKEN`, else `KAGGLE_USERNAME` with `KAGGLE_KEY`.
    pub fn from_env() -> Option<Self> {
        if let Some(token) = env::var(KAGGLE_TOKEN_ENV).ok().filter(|v| !v.trim().is_empty()) {
            return Some(KaggleCredentials::token(&token));
        }
        let username = env::var(KAGGLE_USERNAME_ENV).ok().filter(|v| !v.is_empty())?;
        let key = env::var(KAGGLE_KEY_ENV).ok().filter(|v| !v.is_empty())?;
        Some(KaggleCredentials { username, key })
    }

    /// The Kaggle CLI's files (`$KAGGLE_CONFIG_DIR` or `~/.kaggle`) and the
    /// path used: the API token in `access_token`, else `kaggle.json`.
    pub fn from_files() -> Option<(Self, PathBuf)> {
        let dir = env::var_os("KAGGLE_CONFIG_DIR").map(PathBuf::from).or_else(|| dirs::home_dir().map(|h| h.join(".kaggle")))?;
        let token = dir.join("access_token");
        if let Some(text) = fs::read_to_string(&token).ok().filter(|t| !t.trim().is_empty()) {
            return Some((KaggleCredentials::token(&text), token));
        }
        let path = dir.join("kaggle.json");
        let creds: KaggleCredentials = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
        (!creds.username.is_empty()).then_some((creds, path))
    }
}

/// Service name the credential store files Modeloid's entries under.
const STORE_SERVICE: &str = "modeloid";

/// Where saved credentials are kept.
#[derive(Debug, Clone, Default)]
enum Vault {
    /// In the settings file: there's no credential store on this computer,
    /// or the file isn't the default one.
    #[default]
    File,
    /// The system's credential store.
    System,
    /// Stands in for the system's store in tests.
    #[cfg(test)]
    Memory(std::sync::Arc<std::sync::Mutex<BTreeMap<String, String>>>),
}

impl PartialEq for Vault {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl Vault {
    /// The system's credential store, if this computer has one.
    fn system() -> Vault {
        if keyring::Entry::store_status().is_ok() { Vault::System } else { Vault::File }
    }

    fn label(&self) -> Option<&'static str> {
        match self {
            Vault::File => None,
            Vault::System if cfg!(windows) => Some("Windows Credential Manager"),
            Vault::System if cfg!(target_os = "macos") => Some("the macOS Keychain"),
            Vault::System => Some("the system keyring"),
            #[cfg(test)]
            Vault::Memory(_) => Some("the test store"),
        }
    }

    fn get(&self, service: Service) -> Result<Option<String>, String> {
        match self {
            Vault::File => Ok(None),
            Vault::System => match keyring::Entry::new(STORE_SERVICE, service.entry()).and_then(|e| e.get_password()) {
                Ok(value) => Ok(Some(value)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(e.to_string()),
            },
            #[cfg(test)]
            Vault::Memory(map) => Ok(map.lock().unwrap().get(service.entry()).cloned()),
        }
    }

    /// Stores `value` for `service`, or removes it with `None`.
    fn set(&self, service: Service, value: Option<&str>) -> Result<(), String> {
        match self {
            Vault::File => Ok(()),
            Vault::System => {
                let entry = keyring::Entry::new(STORE_SERVICE, service.entry()).map_err(|e| e.to_string())?;
                let done = match value {
                    Some(value) => entry.set_password(value),
                    None => entry.delete_credential().or_else(|e| if matches!(e, keyring::Error::NoEntry) { Ok(()) } else { Err(e) }),
                };
                done.map_err(|e| e.to_string())
            }
            #[cfg(test)]
            Vault::Memory(map) => {
                let mut map = map.lock().unwrap();
                match value {
                    Some(value) => map.insert(service.entry().to_string(), value.to_string()),
                    None => map.remove(service.entry()),
                };
                Ok(())
            }
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Where the credentials are; the three fields below hold them only when
    /// that's the file (or an older version wrote them there).
    #[serde(skip)]
    vault: Vault,
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

/// GUI look: theme, font family and text size, as ids, and whether title
/// icons are hidden.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppearancePrefs {
    #[serde(default)]
    pub theme: String,
    #[serde(default)]
    pub font: String,
    #[serde(default)]
    pub scale: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hide_icons: bool,
}

impl Settings {
    /// `<config dir>/modeloid/settings.toml`; `None` if the OS has no config folder.
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("modeloid").join("settings.toml"))
    }

    /// Reads `path`; a missing file gives default settings. The default file
    /// keeps its credentials in the system's credential store when there is
    /// one; other files (tests, a copied file) keep them inline.
    pub fn load(path: &Path) -> Result<Self, crate::Error> {
        let vault = if Self::default_path().as_deref() == Some(path) { Vault::system() } else { Vault::File };
        Self::load_with(path, vault)
    }

    /// Reads `path` with credentials kept in `vault`. Credentials found in the
    /// file (written by an older version) are moved there and the file saved
    /// without them; if the store refuses, they stay in the file.
    fn load_with(path: &Path, vault: Vault) -> Result<Self, crate::Error> {
        let mut settings: Settings = match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{} is invalid: {e}", path.display()))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Settings::default(),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display()).into()),
        };
        if vault == Vault::File {
            return Ok(settings);
        }
        let inline: Vec<(Service, String)> = Service::ALL.into_iter().filter_map(|s| settings.inline(s).map(|v| (s, v))).collect();
        if inline.iter().any(|(service, value)| vault.set(*service, Some(value)).is_err()) {
            return Ok(settings);
        }
        settings.vault = vault;
        if !inline.is_empty() {
            for service in Service::ALL {
                settings.put_inline(service, None);
            }
            settings.save(path)?;
        }
        Ok(settings)
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

    pub fn roboflow_api_key(&self) -> Option<String> {
        self.secret(Service::Roboflow)
    }

    pub fn huggingface_token(&self) -> Option<String> {
        self.secret(Service::HuggingFace)
    }

    pub fn kaggle(&self) -> Option<KaggleCredentials> {
        self.secret(Service::Kaggle).and_then(|json| serde_json::from_str(&json).ok())
    }

    /// Where saved credentials are kept, e.g. "Windows Credential Manager";
    /// `None` when that's the settings file.
    pub fn credential_store(&self) -> Option<&'static str> {
        self.vault.label()
    }

    /// The saved credential for `service` as stored (Kaggle's as JSON). One
    /// the store can't give (locked, access denied) counts as not saved.
    fn secret(&self, service: Service) -> Option<String> {
        self.inline(service).or_else(|| self.vault.get(service).ok().flatten())
    }

    fn inline(&self, service: Service) -> Option<String> {
        match service {
            Service::Roboflow => self.roboflow_api_key.clone(),
            Service::HuggingFace => self.huggingface_token.clone(),
            Service::Kaggle => self.kaggle.as_ref().map(|k| serde_json::to_string(k).expect("credentials serialize")),
        }
    }

    fn put_inline(&mut self, service: Service, value: Option<String>) {
        match service {
            Service::Roboflow => self.roboflow_api_key = value,
            Service::HuggingFace => self.huggingface_token = value,
            Service::Kaggle => self.kaggle = value.and_then(|json| serde_json::from_str(&json).ok()),
        }
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
        self.system.unwrap_or_default().normalized(limits::machine())
    }

    pub fn set_system(&mut self, prefs: SystemPrefs) {
        self.system = Some(prefs.normalized(limits::machine()));
    }

    /// Puts the system settings to use in this process (`core::limits`).
    /// Frontends call it at start, and again after saving a change.
    pub fn apply_system(&self) {
        limits::apply(self.system());
    }

    /// Saves validated credentials for `service`, or removes them with `None`.
    /// Kaggle takes an API token, `kaggle.json` contents or `username:key`.
    /// The credential store changes right away; the file on `save`.
    pub fn set_credential(&mut self, service: Service, value: Option<&str>) -> Result<(), String> {
        let value = match (service, value) {
            (_, None) => None,
            (Service::Kaggle, Some(text)) => Some(serde_json::to_string(&KaggleCredentials::parse(text)?).map_err(|e| e.to_string())?),
            (_, Some(text)) => Some(checked_key(text)?),
        };
        match self.vault.label() {
            None => self.put_inline(service, value),
            Some(store) => self.vault.set(service, value.as_deref()).map_err(|e| format!("Couldn't update {store}: {e}"))?,
        }
        Ok(())
    }

    /// Safe to show: the last 4 characters of a key, or the Kaggle username.
    pub fn credential_hint(&self, service: Service) -> Option<String> {
        let tail = |k: &str| k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<String>();
        match service {
            Service::Roboflow => self.roboflow_api_key().as_deref().map(tail),
            Service::HuggingFace => self.huggingface_token().as_deref().map(tail),
            Service::Kaggle => self.kaggle().map(|k| if k.is_token() { format!("API token …{}", tail(&k.key)) } else { k.username }),
        }
    }

    /// Download options using these saved credentials. Missing ones fall back
    /// to env vars (and `kaggle.json`) when the download runs.
    pub fn download_options(&self, format: String) -> DownloadOptions {
        DownloadOptions {
            format,
            api_key: self.roboflow_api_key(),
            kaggle: self.kaggle(),
            hf_token: self.huggingface_token(),
            fork_unversioned: false,
            python: None,
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
        assert_eq!(loaded.system(), SystemPrefs { threads: 1, download_connections: 32, max_jobs: 2, thumb_size: 240 });

        // A file that names only one setting keeps the defaults for the rest.
        let partial: Settings = toml::from_str("[system]\nmax_jobs = 2\n").unwrap();
        assert_eq!(partial.system(), SystemPrefs { max_jobs: 2, ..Default::default() });
    }

    #[test]
    fn parses_kaggle_credentials() {
        let json = KaggleCredentials::parse(r#"{"username":"someone","key":"0123456789abcdef"}"#).unwrap();
        assert_eq!(json, KaggleCredentials { username: "someone".into(), key: "0123456789abcdef".into() });
        assert_eq!(KaggleCredentials::parse(" someone:0123456789abcdef ").unwrap(), json);
        assert!(KaggleCredentials::parse("0123456789abcdef").is_err(), "a legacy key needs the username");
        let token = KaggleCredentials::parse(" KGAT_0123456789abcdef0123456789abcdef ").unwrap();
        assert!(token.is_token());
        assert_eq!(token.key, "KGAT_0123456789abcdef0123456789abcdef");
        assert!(!toml::to_string(&token).unwrap().contains("username"));
        assert_eq!(toml::from_str::<KaggleCredentials>("key = \"KGAT_0123456789abcdef\"").unwrap(), KaggleCredentials::token("KGAT_0123456789abcdef"));
        assert!(KaggleCredentials::parse(":0123456789abcdef").is_err());
        assert!(KaggleCredentials::parse(r#"{"username":"someone"}"#).is_err());
    }

    #[test]
    fn saves_loads_and_removes_credentials() {
        let dir = std::env::temp_dir().join(format!("modeloid-settings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("settings.toml");

        let mut settings = Settings::load(&path).unwrap();
        assert_eq!(settings.credential_hint(Service::Roboflow), None);

        settings.set_credential(Service::Roboflow, Some(" abcdefgh1234 ")).unwrap();
        settings.set_credential(Service::Kaggle, Some("someone:0123456789abcdef")).unwrap();
        settings.save(&path).unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("roboflow_api_key = \"abcdefgh1234\""));

        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.roboflow_api_key().as_deref(), Some("abcdefgh1234"));
        assert_eq!(loaded.credential_hint(Service::Roboflow).as_deref(), Some("1234"));
        assert_eq!(loaded.credential_hint(Service::Kaggle).as_deref(), Some("someone"));
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
        let prefs = AppearancePrefs { theme: "dracula".into(), font: "inter".into(), scale: "large".into(), hide_icons: true };
        settings.set_appearance(prefs.clone());
        let text = toml::to_string(&settings).unwrap();
        assert!(text.contains("[appearance]"));
        let loaded = toml::from_str::<Settings>(&text).unwrap();
        assert_eq!(loaded.appearance(), Some(&prefs));
        assert_eq!(loaded.roboflow_api_key().as_deref(), Some("abcdefgh1234"));
    }

    #[test]
    fn keeps_credentials_in_the_store() {
        let dir = std::env::temp_dir().join(format!("modeloid-settings-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("settings.toml");
        let entries = std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::new()));
        let store = Vault::Memory(entries.clone());

        // An older version's file: the keys move to the store and out of the file.
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, "roboflow_api_key = \"abcdefgh1234\"\n[kaggle]\nusername = \"someone\"\nkey = \"0123456789abcdef\"\n").unwrap();
        let mut settings = Settings::load_with(&path, store.clone()).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("abcdefgh1234") && !text.contains("someone"), "{text}");
        assert_eq!(entries.lock().unwrap().get("roboflow-api-key").map(String::as_str), Some("abcdefgh1234"));
        assert_eq!(settings.kaggle().unwrap().username, "someone");
        assert_eq!(settings.credential_store(), Some("the test store"));

        // New ones go straight to the store; saving never writes them.
        settings.set_credential(Service::HuggingFace, Some("hf_0123456789abcdef")).unwrap();
        settings.set_credential(Service::Roboflow, None).unwrap();
        settings.save(&path).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("hf_"));
        let loaded = Settings::load_with(&path, store).unwrap();
        assert_eq!(loaded.huggingface_token().as_deref(), Some("hf_0123456789abcdef"));
        assert_eq!(loaded.roboflow_api_key(), None);
        assert_eq!(loaded.credential_hint(Service::HuggingFace).as_deref(), Some("cdef"));
        assert!(Settings::load_with(&path, Vault::File).unwrap().huggingface_token().is_none(), "the file never had it");

        let _ = fs::remove_dir_all(&dir);
    }

    /// Touches this computer's real credential store, under a throwaway name.
    #[test]
    #[ignore = "uses the system's credential store"]
    fn system_store_round_trips() {
        keyring::Entry::store_status().as_ref().expect("no credential store");
        let entry = keyring::Entry::new(STORE_SERVICE, "self-test").unwrap();
        entry.set_password("abcdefgh1234").unwrap();
        assert_eq!(entry.get_password().unwrap(), "abcdefgh1234");
        entry.delete_credential().unwrap();
        assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
    }
}
