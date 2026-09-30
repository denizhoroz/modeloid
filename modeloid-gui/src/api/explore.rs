//! Explore: statistics and gallery pages for one dataset. Image files and
//! previews are served by plain GET routes (`api/files.rs`), so `<img>` can
//! point at them.

use dioxus::prelude::*;

pub use modeloid_types::stats::{ExploreStats, GalleryFilter, GalleryImage, GalleryPage};

/// Statistics of dataset `id` (cached by the core until the dataset changes;
/// `refresh` recomputes).
#[post("/api/explore/stats")]
pub async fn dataset_stats(id: u32, refresh: bool) -> Result<ExploreStats, ServerFnError> {
    backend::stats(id, refresh).await.map_err(ServerFnError::new)
}

/// Images `offset..offset + limit` of dataset `id` matching `filter`, with their objects.
#[post("/api/explore/gallery")]
pub async fn gallery_page(id: u32, filter: GalleryFilter, offset: usize, limit: usize) -> Result<GalleryPage, ServerFnError> {
    backend::gallery(id, filter, offset, limit.min(200)).await.map_err(ServerFnError::new)
}

/// URL of a preview of image `index` of dataset `id`.
pub fn thumb_url(id: u32, index: usize) -> String {
    format!("/files/thumb/{id}/{index}")
}

/// URL of image `index` of dataset `id` at full size.
pub fn image_url(id: u32, index: usize) -> String {
    format!("/files/image/{id}/{index}")
}

#[cfg(feature = "server")]
pub use backend::{image_file, labels as cached_labels};

#[cfg(feature = "server")]
mod backend {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use modeloid::{core::stats, LabelSet, Progress, Silent, Workspace};

    use super::{ExploreStats, GalleryFilter, GalleryPage};
    use crate::api::{activity, workspace};

    /// Labels of the dataset last looked at, so paging and previews don't
    /// re-read every label file. Keyed by workspace, dataset and its toml stamp.
    static LABELS: Mutex<Option<(String, Arc<LabelSet>)>> = Mutex::new(None);

    fn open() -> Result<Workspace, String> {
        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))
    }

    /// Dataset `id`'s folder and labels, read only when it isn't the cached one.
    pub fn labels(ws: &Workspace, id: u32, progress: &dyn Progress) -> Result<(PathBuf, Arc<LabelSet>), String> {
        let (dir, _) = ws.find_dataset(id).map_err(|e| e.to_string())?;
        let meta = std::fs::metadata(dir.join("dataset.toml")).map_err(|e| e.to_string())?;
        let key = format!("{}|{id}|{}|{:?}", ws.path().display(), meta.len(), meta.modified().ok());

        if let Some((cached_key, set)) = LABELS.lock().unwrap().as_ref() {
            if *cached_key == key {
                return Ok((dir, set.clone()));
            }
        }
        let (_, _, set) = ws.read_labels(id, progress).map_err(|e| e.to_string())?;
        let set = Arc::new(set);
        *LABELS.lock().unwrap() = Some((key, set.clone()));
        Ok((dir, set))
    }

    /// Opens the workspace and names the load after dataset `id`.
    fn open_for(ctx: &activity::TaskCtx, action: &str, id: u32) -> Result<Workspace, String> {
        let ws = open()?;
        if let Ok((_, dataset)) = ws.find_dataset(id) {
            ctx.set_label(format!("{action} · {}", dataset.name()));
        }
        Ok(ws)
    }

    pub async fn stats(id: u32, refresh: bool) -> Result<ExploreStats, String> {
        activity::track("explore-stats", "Reading labels", move |ctx| {
            let ws = open_for(ctx, "Reading labels", id)?;
            ws.explore_stats(id, refresh, &ctx.progress()).map_err(|e| e.to_string())
        })
        .await
    }

    pub async fn gallery(id: u32, filter: GalleryFilter, offset: usize, limit: usize) -> Result<GalleryPage, String> {
        activity::track("explore-gallery", "Loading gallery", move |ctx| {
            let (_, set) = labels(&open_for(ctx, "Loading gallery", id)?, id, &ctx.progress())?;
            Ok(stats::gallery_page(&set, &filter, offset, limit))
        })
        .await
    }

    /// The file of image `index` in dataset `id` and, with `thumb_size`, a
    /// cached preview of it. Only files inside the dataset folder are served.
    pub fn image_file(id: u32, index: usize, thumb_size: Option<u32>) -> Result<PathBuf, String> {
        let ws = open()?;
        let (dir, set) = labels(&ws, id, &Silent)?;
        let file = &set.images.get(index).ok_or("No such image.")?.file;
        let (dir, file) = (dir.canonicalize().map_err(|e| e.to_string())?, file.canonicalize().map_err(|e| e.to_string())?);
        if !file.starts_with(&dir) {
            return Err("That image is outside the dataset folder.".into());
        }
        match thumb_size {
            Some(size) => modeloid::core::thumbs::thumbnail(&file, &ws.thumbs_dir(id), size).map_err(|e| e.to_string()),
            None => Ok(file),
        }
    }
}
