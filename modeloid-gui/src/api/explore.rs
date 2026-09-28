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
pub use backend::image_file;

#[cfg(feature = "server")]
mod backend {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use modeloid::{core::stats, LabelSet, Silent, Workspace};

    use super::{ExploreStats, GalleryFilter, GalleryPage};
    use crate::api::workspace;

    /// Labels of the dataset last looked at, so paging and previews don't
    /// re-read every label file. Keyed by workspace, dataset and its toml stamp.
    static LABELS: Mutex<Option<(String, Arc<LabelSet>)>> = Mutex::new(None);

    fn open() -> Result<Workspace, String> {
        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))
    }

    fn labels(ws: &Workspace, id: u32) -> Result<(PathBuf, Arc<LabelSet>), String> {
        let (dir, _) = ws.find_dataset(id).map_err(|e| e.to_string())?;
        let meta = std::fs::metadata(dir.join("dataset.toml")).map_err(|e| e.to_string())?;
        let key = format!("{}|{id}|{}|{:?}", ws.path().display(), meta.len(), meta.modified().ok());

        if let Some((cached_key, set)) = LABELS.lock().unwrap().as_ref() {
            if *cached_key == key {
                return Ok((dir, set.clone()));
            }
        }
        let (_, _, set) = ws.read_labels(id, &Silent).map_err(|e| e.to_string())?;
        let set = Arc::new(set);
        *LABELS.lock().unwrap() = Some((key, set.clone()));
        Ok((dir, set))
    }

    async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
        tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?
    }

    pub async fn stats(id: u32, refresh: bool) -> Result<ExploreStats, String> {
        blocking(move || open()?.explore_stats(id, refresh, &Silent).map_err(|e| e.to_string())).await
    }

    pub async fn gallery(id: u32, filter: GalleryFilter, offset: usize, limit: usize) -> Result<GalleryPage, String> {
        blocking(move || {
            let (_, set) = labels(&open()?, id)?;
            Ok(stats::gallery_page(&set, &filter, offset, limit))
        })
        .await
    }

    /// The file of image `index` in dataset `id` and, with `thumb_size`, a
    /// cached preview of it. Only files inside the dataset folder are served.
    pub fn image_file(id: u32, index: usize, thumb_size: Option<u32>) -> Result<PathBuf, String> {
        let ws = open()?;
        let (dir, set) = labels(&ws, id)?;
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
