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
pub use backend::{edit_labels, image_file, labels as cached_labels};

#[cfg(feature = "server")]
mod backend {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use modeloid::{core::stats, LabelSet, Progress, Silent, Workspace};

    use super::{ExploreStats, GalleryFilter, GalleryPage};
    use crate::api::activity;

    /// Workspace folder, dataset id and its dataset.toml's stamp.
    type Key = (PathBuf, u32, String);

    /// Labels of the datasets last looked at, most recent last, so paging,
    /// previews and going back and forth between a few datasets don't re-read
    /// every label file.
    static LABELS: Mutex<Vec<(Key, Arc<LabelSet>)>> = Mutex::new(Vec::new());
    /// Label sets kept: a big dataset's labels take a lot of memory.
    const KEPT: usize = 3;

    use crate::api::workspace::open;

    /// Label edits one at a time, so two can't change the cached set at once.
    static EDITS: Mutex<()> = Mutex::new(());

    /// Cache key of dataset `id` in folder `dir`: its dataset.toml's stamp.
    fn cache_key(ws: &Workspace, dir: &std::path::Path, id: u32) -> Result<Key, String> {
        let meta = std::fs::metadata(dir.join("dataset.toml")).map_err(|e| e.to_string())?;
        Ok((ws.path().to_path_buf(), id, format!("{}|{:?}", meta.len(), meta.modified().ok())))
    }

    /// Drops what's cached for the dataset of `key`, whatever its stamp.
    fn forget(cached: &mut Vec<(Key, Arc<LabelSet>)>, key: &Key) {
        cached.retain(|((ws, id, _), _)| (ws, id) != (&key.0, &key.1));
    }

    fn remember(key: Key, set: Arc<LabelSet>) {
        let mut cached = LABELS.lock().unwrap();
        forget(&mut cached, &key);
        cached.push((key, set));
        if cached.len() > KEPT {
            cached.remove(0);
        }
    }

    /// Dataset `id`'s folder and labels, read only when it isn't the cached one.
    pub fn labels(ws: &Workspace, id: u32, progress: &dyn Progress) -> Result<(PathBuf, Arc<LabelSet>), String> {
        let (dir, _) = ws.find_dataset(id).map_err(|e| e.to_string())?;
        let key = cache_key(ws, &dir, id)?;

        {
            let mut cached = LABELS.lock().unwrap();
            if let Some(at) = cached.iter().position(|(k, _)| *k == key) {
                let hit = cached.remove(at);
                let set = hit.1.clone();
                cached.push(hit);
                return Ok((dir, set));
            }
        }
        let (_, _, set) = ws.read_labels(id, progress).map_err(|e| e.to_string())?;
        let set = Arc::new(set);
        remember(key, set.clone());
        Ok((dir, set))
    }

    /// Runs `change` (a labelling edit, which rewrites dataset.toml) on dataset
    /// `id`'s labels and keeps the changed set cached, so the next request
    /// doesn't read every label file again. A failed change drops the cache:
    /// the files may be half changed.
    pub fn edit_labels<T>(ws: &Workspace, id: u32, change: impl FnOnce(&mut LabelSet) -> Result<T, String>) -> Result<T, String> {
        let _one_at_a_time = EDITS.lock().unwrap_or_else(|e| e.into_inner());
        let (dir, set) = labels(ws, id, &Silent)?;
        forget(&mut LABELS.lock().unwrap(), &cache_key(ws, &dir, id)?);
        // Usually the only reference now, so nothing is copied.
        let mut set = Arc::try_unwrap(set).unwrap_or_else(|shared| (*shared).clone());
        let result = change(&mut set);
        if result.is_ok() {
            remember(cache_key(ws, &dir, id)?, Arc::new(set));
        }
        result
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
            let ws = open_for(ctx, "Loading gallery", id)?;
            let (_, set) = labels(&ws, id, &ctx.progress())?;
            let colors = ws.find_dataset(id).map_err(|e| e.to_string())?.1.colors_for(&set.classes);
            Ok(GalleryPage { colors, ..stats::gallery_page(&set, &filter, offset, limit) })
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
