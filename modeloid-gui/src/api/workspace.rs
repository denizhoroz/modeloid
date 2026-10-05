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
    pub description: String,
    /// Seed of random choices (splits, subsets).
    pub seed: u64,
    /// Label format tools start on: "yolo", "coco" or "voc"; empty = follow the dataset.
    pub default_format: String,
}

/// A seed typed into a form: a whole number from 0 up.
pub fn parse_seed(text: &str) -> Result<u64, String> {
    text.trim().parse().map_err(|_| "Enter a whole number, 0 or more.".to_string())
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

/// Creates `<parent>/<name>/` as a new workspace and opens it.
/// `default_format` is "yolo", "coco", "voc" or empty (follow the dataset).
#[post("/api/workspace/create")]
pub async fn create_workspace(
    name: String,
    description: String,
    parent: String,
    seed: u64,
    default_format: String,
) -> Result<WorkspaceInfo, ServerFnError> {
    backend::create(&name, &description, parent.as_ref(), seed, &default_format).map_err(ServerFnError::new)
}

/// Changes the open workspace's description, seed and default label format.
#[post("/api/workspace/update")]
pub async fn update_workspace(description: String, seed: u64, default_format: String) -> Result<WorkspaceInfo, ServerFnError> {
    backend::update(&description, seed, &default_format).map_err(ServerFnError::new)
}

/// Opens a folder picker and loads the chosen workspace.
/// Returns `None` if the user cancels the dialog.
#[post("/api/workspace/load")]
pub async fn load_workspace() -> Result<Option<WorkspaceInfo>, ServerFnError> {
    use crate::api::dialogs::{pick_folder_path, DialogPurpose};
    let Some(dir) = pick_folder_path("Choose a workspace folder".into(), DialogPurpose::Workspace).await? else {
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

    pub fn create(name: &str, description: &str, parent: &Path, seed: u64, default_format: &str) -> Result<WorkspaceInfo, String> {
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
        workspace.set_seed(seed);
        workspace.set_default_format(Some(default_format)).map_err(|e| e.to_string())?;
        workspace
            .save_to_dir()
            .map_err(|e| format!("Couldn't create the workspace in {}: {e}", root.display()))?;

        load(&root)
    }

    pub fn update(description: &str, seed: u64, default_format: &str) -> Result<WorkspaceInfo, String> {
        let dir = current().ok_or("Load or create a workspace first.")?;
        let mut workspace = Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't load the workspace: {e}"))?;
        workspace.set_description(description.trim().to_string());
        workspace.set_seed(seed);
        workspace.set_default_format(Some(default_format)).map_err(|e| e.to_string())?;
        workspace.save().map_err(|e| format!("Couldn't save the workspace: {e}"))?;
        crate::api::history::record("Settings", "Workspace", "Save settings", workspace.name(), None);
        Ok(info(&workspace))
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
        // Ids with gaps, e.g. from datasets deleted outside the app.
        crate::api::jobs::renumber_when_idle();
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
            description: workspace.description().to_string(),
            seed: workspace.seed(),
            default_format: workspace.default_format_name().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_seed, validate_workspace_name};

    #[test]
    fn validates_names() {
        assert!(validate_workspace_name("my-workspace").is_ok());
        assert!(validate_workspace_name("  padded  ").is_ok());
        assert!(validate_workspace_name("   ").is_err());
        assert!(validate_workspace_name("a/b").is_err());
        assert!(validate_workspace_name("what?").is_err());
        assert!(validate_workspace_name("trailing.").is_err());
    }

    #[test]
    fn parses_seeds() {
        assert_eq!(parse_seed(" 42 "), Ok(42));
        assert_eq!(parse_seed("0"), Ok(0));
        assert!(parse_seed("").is_err() && parse_seed("-1").is_err() && parse_seed("1.5").is_err());
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

        let created = backend::create(" cats ", "pet detection", &parent, 7, "coco").unwrap();
        let root = parent.join("cats");
        assert!(root.join("workspace.toml").is_file());
        assert!(root.join("datasets").is_dir());
        assert_eq!(created.name, "cats");
        assert_eq!(created.folder, "cats");
        assert_eq!((created.seed, created.default_format.as_str(), created.description.as_str()), (7, "coco", "pet detection"));
        assert_eq!(backend::current().as_deref(), Some(root.as_path()));
        assert!(backend::create("cats", "", &parent, 42, "").is_err(), "creating twice should fail");

        let loaded = backend::load(&root).unwrap();
        assert_eq!(loaded, created);
        let toml = std::fs::read_to_string(root.join("workspace.toml")).unwrap();
        assert!(toml.contains("pet detection"));

        // Settings change the open workspace in place.
        let updated = backend::update(" more pets ", 9, "").unwrap();
        assert_eq!((updated.seed, updated.default_format.as_str(), updated.description.as_str()), (9, "", "more pets"));
        assert_eq!(backend::load(&root).unwrap(), updated);
        assert!(backend::update("x", 1, "parquet").is_err());

        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn create_rejects_bad_input() {
        let parent = temp_dir("bad");
        assert!(backend::create("a/b", "", &parent, 42, "").is_err());
        assert!(backend::create("ok", "", &parent.join("missing"), 42, "").is_err());
        assert!(backend::create("ok", "", &parent, 42, "parquet").is_err());
        assert!(!parent.join("ok").exists(), "nothing is created for a bad format");
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn load_rejects_non_workspace() {
        let dir = temp_dir("plain");
        assert!(backend::load(&dir).unwrap_err().contains("has no workspace.toml"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
