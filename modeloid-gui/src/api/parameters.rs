//! Training parameter sets, saved in the workspace as
//! `models/parameters/<name>.toml` (see the core's `Workspace::parameter_sets`).

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

pub use modeloid_types::parameters::*;

/// The saved sets, and a warning for each file that couldn't be read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParameterList {
    pub sets: Vec<ParameterSet>,
    pub warnings: Vec<String>,
}

#[post("/api/parameters")]
pub async fn list_parameters() -> Result<ParameterList, ServerFnError> {
    backend::blocking(|ws| {
        let (sets, warnings) = ws.parameter_sets().map_err(|e| e.to_string())?;
        Ok(ParameterList { sets, warnings })
    })
    .await
}

/// Adds `set` (`replacing = None`) or saves it over the set named
/// `replacing`, renaming it if the name changed. Returns it as saved.
#[post("/api/parameters/save")]
pub async fn save_parameters(set: ParameterSet, replacing: Option<String>) -> Result<ParameterSet, ServerFnError> {
    backend::blocking(move |ws| ws.save_parameter_set(&set, replacing.as_deref()).map_err(|e| e.to_string())).await
}

#[post("/api/parameters/delete")]
pub async fn delete_parameters(name: String) -> Result<(), ServerFnError> {
    backend::blocking(move |ws| ws.delete_parameter_set(&name).map_err(|e| e.to_string())).await
}

#[cfg(feature = "server")]
mod backend {
    use dioxus::prelude::ServerFnError;
    use modeloid::Workspace;

    use crate::api::workspace;

    fn open() -> Result<Workspace, String> {
        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))
    }

    /// Runs `f` with the open workspace on a blocking thread.
    pub async fn blocking<T: Send + 'static>(
        f: impl FnOnce(&Workspace) -> Result<T, String> + Send + 'static,
    ) -> Result<T, ServerFnError> {
        tokio::task::spawn_blocking(move || f(&open()?))
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?
            .map_err(ServerFnError::new)
    }
}
