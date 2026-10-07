//! System settings (threads, download connections, jobs at once, preview
//! size) and the open workspace's cache. Stored by the core in
//! `settings.toml` (`[system]`), applied to this server as soon as saved.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

pub use modeloid_types::system::{Machine, SystemPrefs, MAX_DOWNLOAD_CONNECTIONS, THUMB_SIZES};

/// The settings in use and what this computer has.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SystemView {
    pub prefs: SystemPrefs,
    /// The computer the server runs on.
    pub machine: Machine,
}

#[get("/api/settings/system")]
pub async fn get_system() -> Result<SystemView, ServerFnError> {
    crate::api::blocking(|| Ok(backend::view())).await
}

/// Saves the settings (pulled into range) and puts them to use at once;
/// running jobs pick up thread and connection changes at their next step.
#[post("/api/settings/system/save")]
pub async fn save_system(prefs: SystemPrefs) -> Result<SystemView, ServerFnError> {
    crate::api::blocking(move || backend::save(prefs)).await
}

/// Bytes of cached previews and statistics in the open workspace; `None` when none is open.
#[get("/api/settings/cache")]
pub async fn cache_size() -> Result<Option<u64>, ServerFnError> {
    backend::cache_size().await.map_err(ServerFnError::new)
}

/// Deletes the open workspace's cached previews and statistics. Returns the bytes freed.
#[post("/api/settings/cache/clear")]
pub async fn clear_cache() -> Result<u64, ServerFnError> {
    backend::clear_cache().await.map_err(ServerFnError::new)
}

#[cfg(feature = "server")]
mod backend {
    use modeloid::core::limits;
    use modeloid::{Settings, Workspace};

    use super::{SystemPrefs, SystemView};
    use crate::api::workspace;

    pub fn view() -> SystemView {
        SystemView { prefs: limits::current(), machine: limits::machine() }
    }

    pub fn save(prefs: SystemPrefs) -> Result<SystemView, String> {
        let path = crate::api::settings::settings_path()?;
        let mut settings = Settings::load(&path).map_err(|e| e.to_string())?;
        settings.set_system(prefs);
        settings.save(&path).map_err(|e| e.to_string())?;
        settings.apply_system();
        Ok(view())
    }

    fn open() -> Option<Result<Workspace, String>> {
        let dir = workspace::current_dir()?;
        Some(Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}")))
    }

    pub async fn cache_size() -> Result<Option<u64>, String> {
        tokio::task::spawn_blocking(|| open().map(|ws| ws.map(|ws| ws.cache_size())).transpose()).await.map_err(|e| e.to_string())?
    }

    pub async fn clear_cache() -> Result<u64, String> {
        tokio::task::spawn_blocking(|| {
            let ws = open().ok_or("Load or create a workspace first.")??;
            let freed = ws.clear_cache().map_err(|e| e.to_string())?;
            // The history went with the cache; this starts the new one.
            crate::api::history::record("Settings", "System", "Clear cache", crate::util::format_bytes(freed), Some("Previews, statistics and history removed".into()));
            Ok(freed)
        })
        .await
        .map_err(|e| e.to_string())?
    }
}
