//! Importing a local dataset folder or zip into the workspace, as a job
//! (`JobKind::Import`) so it shows progress and can be cancelled.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::api::jobs::Job;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportKind {
    /// A dataset folder, copied.
    Copy,
    /// A dataset folder, moved (the original is gone afterwards).
    Move,
    Zip,
}

/// Dataset name suggested for a path: the folder name, or the zip's name without `.zip`.
pub fn name_from_path(path: &str) -> String {
    let last = path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or_default();
    let lower = last.to_ascii_lowercase();
    if lower.ends_with(".zip") { last[..last.len() - 4].to_string() } else { last.to_string() }
}

/// Imports `path` as a new dataset called `name` in the open workspace.
#[post("/api/import/start")]
pub async fn start_import(path: String, kind: ImportKind, name: String) -> Result<Job, ServerFnError> {
    crate::api::blocking(move || backend::start(path, kind, name)).await
}

#[cfg(feature = "server")]
mod backend {
    use std::path::PathBuf;

    use modeloid::{validate_dataset_name, ImportMode};

    use super::ImportKind;
    use crate::api::jobs::{self, Job, JobKind};
    use crate::api::workspace;

    pub fn start(path: String, kind: ImportKind, name: String) -> Result<Job, String> {
        let ws = workspace::open()?;
        let name = name.trim().to_string();
        validate_dataset_name(&name)?;
        let source = PathBuf::from(path.trim());
        match kind {
            ImportKind::Zip if !source.is_file() => return Err(format!("{} isn't a file.", source.display())),
            ImportKind::Copy | ImportKind::Move if !source.is_dir() => {
                return Err(format!("{} isn't a folder.", source.display()));
            }
            _ => {}
        }
        if ws.datasets_dir().join(&name).exists() {
            return Err(format!("A dataset named '{name}' already exists. Choose another name."));
        }

        jobs::start(JobKind::Import, vec![source.display().to_string()], jobs::Touches::Nothing, move |ctx| {
            let reporter = ctx.reporter();
            let result = match kind {
                ImportKind::Copy => ws.import_dir(&source, &name, ImportMode::Copy, reporter.as_ref()),
                ImportKind::Move => ws.import_dir(&source, &name, ImportMode::Move, reporter.as_ref()),
                ImportKind::Zip => ws.import_zip(&source, &name, reporter.as_ref()),
            };
            result.map(|dir| Some(dir.display().to_string())).map_err(|e| e.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::name_from_path;

    #[test]
    fn names_from_paths() {
        assert_eq!(name_from_path(r"C:\data\cats"), "cats");
        assert_eq!(name_from_path("/home/me/cats/"), "cats");
        assert_eq!(name_from_path(r"D:\dl\Cats-v2.ZIP"), "Cats-v2");
        assert_eq!(name_from_path(""), "");
    }
}
