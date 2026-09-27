use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// What the UI needs to know about a workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub name: String,
    /// Name of the workspace's folder on disk.
    pub folder: String,
    pub path: String,
    /// Default place for downloaded datasets.
    pub datasets_path: String,
}

/// Checks a workspace name, which is also used as its folder name.
/// Shared by the form (instant feedback) and the server (final check).
pub fn validate_workspace_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a name.".into());
    }
    if name.chars().count() > 100 {
        return Err("Keep the name under 100 characters.".into());
    }
    if let Some(c) = name.chars().find(|c| r#"<>:"/\|?*"#.contains(*c) || c.is_control()) {
        return Err(format!("The name can't contain '{c}', it's used as a folder name."));
    }
    if name.ends_with('.') {
        return Err("The name can't end with a dot.".into());
    }
    Ok(())
}

/// The workspace the server currently has open, if any.
#[get("/api/workspace/current")]
pub async fn current_workspace() -> Result<Option<WorkspaceInfo>, ServerFnError> {
    // Re-read it so a moved or deleted workspace isn't shown as open.
    Ok(backend::current().and_then(|dir| backend::load(&dir).ok()))
}

/// Opens a native folder picker. Returns `None` if the user cancels.
#[post("/api/fs/pick-folder")]
pub async fn pick_folder(title: String) -> Result<Option<String>, ServerFnError> {
    let dir = backend::pick_folder(title).await?;
    Ok(dir.map(|d| d.display().to_string()))
}

/// Creates `<parent>/<name>/` as a new workspace and opens it.
#[post("/api/workspace/create")]
pub async fn create_workspace(
    name: String,
    description: String,
    parent: String,
) -> Result<WorkspaceInfo, ServerFnError> {
    backend::create(&name, &description, parent.as_ref()).map_err(ServerFnError::new)
}

/// Opens a folder picker and loads the chosen workspace.
/// Returns `None` if the user cancels the dialog.
#[post("/api/workspace/load")]
pub async fn load_workspace() -> Result<Option<WorkspaceInfo>, ServerFnError> {
    let Some(dir) = backend::pick_folder("Choose a workspace folder".into()).await? else {
        return Ok(None);
    };
    backend::load(&dir).map(Some).map_err(ServerFnError::new)
}

/// Folder of the open workspace, for other server functions.
#[cfg(feature = "server")]
pub use backend::current as current_dir;

#[cfg(feature = "server")]
mod backend {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use dioxus::prelude::ServerFnError;
    use modeloid::Workspace;

    use super::{validate_workspace_name, WorkspaceInfo};

    const WORKSPACE_FILE: &str = "workspace.toml";

    /// Single-user local app, so one open workspace for the whole server.
    static CURRENT: Mutex<Option<PathBuf>> = Mutex::new(None);

    pub fn current() -> Option<PathBuf> {
        CURRENT.lock().unwrap().clone()
    }

    fn set_current(dir: &Path) {
        *CURRENT.lock().unwrap() = Some(dir.to_path_buf());
    }

    /// Shows the native folder dialog. It blocks, so it runs on a blocking thread.
    pub async fn pick_folder(title: String) -> Result<Option<PathBuf>, ServerFnError> {
        tokio::task::spawn_blocking(move || rfd::FileDialog::new().set_title(title).pick_folder())
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))
    }

    pub fn create(name: &str, description: &str, parent: &Path) -> Result<WorkspaceInfo, String> {
        validate_workspace_name(name)?;
        let name = name.trim();
        if !parent.is_dir() {
            return Err(format!("{} isn't a folder. Choose another location.", parent.display()));
        }
        let root = parent.join(name);
        if root.exists() {
            return Err(format!("{} already exists. Pick another name or location.", root.display()));
        }

        let mut workspace = Workspace::new(name.to_string(), parent.to_path_buf());
        workspace.set_description(description.trim().to_string());
        workspace
            .save_to_dir()
            .map_err(|e| format!("Couldn't create the workspace in {}: {e}", root.display()))?;

        load(&root)
    }

    pub fn load(dir: &Path) -> Result<WorkspaceInfo, String> {
        if !dir.join(WORKSPACE_FILE).is_file() {
            return Err(format!(
                "{} has no {WORKSPACE_FILE}. Pick a workspace folder, or use Create.",
                dir.display()
            ));
        }
        let workspace = Workspace::load_from_dir(dir)
            .map_err(|e| format!("Couldn't load the workspace: {e}"))?;

        set_current(dir);
        Ok(info(&workspace))
    }

    fn info(workspace: &Workspace) -> WorkspaceInfo {
        let path = workspace.path();
        WorkspaceInfo {
            name: workspace.name().to_string(),
            folder: path
                .file_name()
                .map_or_else(|| path.display().to_string(), |f| f.to_string_lossy().into_owned()),
            path: path.display().to_string(),
            datasets_path: workspace.datasets_dir().display().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::validate_workspace_name;

    #[test]
    fn validates_names() {
        assert!(validate_workspace_name("my-workspace").is_ok());
        assert!(validate_workspace_name("  padded  ").is_ok());
        assert!(validate_workspace_name("   ").is_err());
        assert!(validate_workspace_name("a/b").is_err());
        assert!(validate_workspace_name("what?").is_err());
        assert!(validate_workspace_name("trailing.").is_err());
    }
}

#[cfg(all(test, feature = "server"))]
mod server_tests {
    use super::backend;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("modeloid-gui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_then_load() {
        let parent = temp_dir("create");

        let created = backend::create(" cats ", "pet detection", &parent).unwrap();
        let root = parent.join("cats");
        assert!(root.join("workspace.toml").is_file());
        assert!(root.join("datasets").is_dir());
        assert_eq!(created.name, "cats");
        assert_eq!(created.folder, "cats");
        assert_eq!(backend::current().as_deref(), Some(root.as_path()));
        assert!(backend::create("cats", "", &parent).is_err(), "creating twice should fail");

        let loaded = backend::load(&root).unwrap();
        assert_eq!(loaded, created);
        let toml = std::fs::read_to_string(root.join("workspace.toml")).unwrap();
        assert!(toml.contains("pet detection"));

        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn create_rejects_bad_input() {
        let parent = temp_dir("bad");
        assert!(backend::create("a/b", "", &parent).is_err());
        assert!(backend::create("ok", "", &parent.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn load_rejects_non_workspace() {
        let dir = temp_dir("plain");
        assert!(backend::load(&dir).unwrap_err().contains("has no workspace.toml"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
