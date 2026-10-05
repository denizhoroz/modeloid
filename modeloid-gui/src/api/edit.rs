//! Datasets > Create and Edit: make an empty dataset, change a dataset's
//! details, add images to it (uploaded from the browser, then moved in by a
//! `JobKind::AddImages` job) and show how far its labelling is.

use dioxus::fullstack::FileStream;
use dioxus::prelude::*;

pub use modeloid_types::edit::{
    is_image_name, parse_tags, DatasetDetails, LabelStatus, NewDataset, CREATE_TYPES,
};

use crate::api::jobs::Job;

/// Creates an empty dataset in the open workspace; returns its id.
#[post("/api/datasets/create")]
pub async fn create_dataset(new: NewDataset) -> Result<u32, ServerFnError> {
    backend::create(new).map_err(ServerFnError::new)
}

/// The details Edit shows for dataset `id`.
#[post("/api/datasets/details")]
pub async fn dataset_details(id: u32) -> Result<DatasetDetails, ServerFnError> {
    backend::details(id).map_err(ServerFnError::new)
}

/// Saves `details` for dataset `id`, renaming its folder for a new name.
/// `expected` is the name the page showed: ids are renumbered after jobs, so
/// a page that hasn't caught up is refused rather than changing another dataset.
#[post("/api/datasets/save-details")]
pub async fn save_dataset_details(id: u32, expected: String, details: DatasetDetails) -> Result<(), ServerFnError> {
    backend::save(id, expected, details).map_err(ServerFnError::new)
}

/// Starts an upload of images; pass the token to `upload_image` and `start_add_images`.
#[post("/api/datasets/upload/start")]
pub async fn start_upload() -> Result<String, ServerFnError> {
    backend::start_upload().map_err(ServerFnError::new)
}

/// Stores one image of upload `token`.
#[post("/api/datasets/upload/image?token")]
pub async fn upload_image(token: String, file: FileStream) -> Result<(), ServerFnError> {
    backend::upload(token, file).await.map_err(ServerFnError::new)
}

/// Moves the images of upload `token` into dataset `id`, as a job.
#[post("/api/datasets/upload/add")]
pub async fn start_add_images(id: u32, token: String) -> Result<Job, ServerFnError> {
    backend::add(id, token).map_err(ServerFnError::new)
}

/// How many of dataset `id`'s images have no labels, and how many labels it has.
#[post("/api/datasets/label-status")]
pub async fn label_status(id: u32) -> Result<LabelStatus, ServerFnError> {
    backend::label_status(id).await.map_err(ServerFnError::new)
}

#[cfg(feature = "server")]
mod backend {
    use dioxus::fullstack::FileStream;
    use futures_util::StreamExt;
    use modeloid::{core::edit, Workspace};
    use tokio::io::AsyncWriteExt;

    use super::{DatasetDetails, LabelStatus, NewDataset, CREATE_TYPES};
    use crate::api::jobs::{self, Job, JobKind};
    use crate::api::{activity, explore, history, workspace};
    use crate::util::plural;

    fn open() -> Result<Workspace, String> {
        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))
    }

    pub fn create(new: NewDataset) -> Result<u32, String> {
        let new = NewDataset { name: new.name.trim().to_string(), ..new };
        let (_, dataset) = open()?.create_dataset(&new).map_err(|e| e.to_string())?;
        let label = |list: &[(&str, &'static str)], key: &str| list.iter().find(|(k, _)| *k == key).map_or(key.to_string(), |(_, l)| l.to_string());
        let detail = format!("{} · {}", label(CREATE_TYPES, &new.dataset_type), modeloid_types::formats::label(&new.annotation_format));
        history::record("Dataset Manager", "Datasets", "Create dataset", dataset.name(), Some(detail));
        Ok(dataset.id())
    }

    pub fn details(id: u32) -> Result<DatasetDetails, String> {
        open()?.dataset_details(id).map_err(|e| e.to_string())
    }

    pub fn save(id: u32, expected: String, details: DatasetDetails) -> Result<(), String> {
        let ws = open()?;
        let (_, dataset) = ws.find_dataset(id).map_err(|e| e.to_string())?;
        if dataset.name() != expected {
            return Err("The dataset has changed meanwhile. Check the selection and try again.".into());
        }
        jobs::busy(id)?;
        ws.edit_dataset(id, &details).map_err(|e| format!("Couldn't save: {e}"))?;
        let detail = (details.name.trim() != expected).then(|| format!("Renamed to {}", details.name.trim()));
        history::record("Dataset Manager", "Datasets", "Edit dataset", expected, detail);
        Ok(())
    }

    pub fn start_upload() -> Result<String, String> {
        open()?.new_upload().map_err(|e| e.to_string())
    }

    pub async fn upload(token: String, mut file: FileStream) -> Result<(), String> {
        let path = open()?.upload_path(&token, file.file_name()).map_err(|e| e.to_string())?;
        let written = async {
            let mut out = tokio::fs::File::create(&path).await.map_err(|e| e.to_string())?;
            while let Some(chunk) = file.next().await {
                let chunk = chunk.map_err(|e| format!("The upload broke off: {e}"))?;
                out.write_all(&chunk).await.map_err(|e| e.to_string())?;
            }
            out.flush().await.map_err(|e| e.to_string())
        }
        .await;
        if written.is_err() {
            let _ = tokio::fs::remove_file(&path).await;
        }
        written
    }

    pub fn add(id: u32, token: String) -> Result<Job, String> {
        let ws = open()?;
        ws.upload_dir(&token).map_err(|e| e.to_string())?;
        let (_, dataset) = ws.find_dataset(id).map_err(|e| e.to_string())?;
        jobs::start(JobKind::AddImages, vec![dataset.name().to_string()], jobs::Touches::Datasets(vec![id]), move |ctx| {
            let added = ws.add_uploaded_images(id, &token, ctx.reporter().as_ref()).map_err(|e| e.to_string())?;
            let mut note = format!("Added {}", plural(added.added as usize, "image"));
            if added.renamed > 0 {
                note += &format!(", {} under a new name (the name was taken)", added.renamed);
            }
            if !added.skipped.is_empty() {
                note += &format!("; skipped {} that aren't images", plural(added.skipped.len(), "file"));
            }
            Ok(Some(note))
        })
    }

    pub async fn label_status(id: u32) -> Result<LabelStatus, String> {
        activity::track("label-status", "Reading labels", move |ctx| {
            let ws = open()?;
            let (_, dataset) = ws.find_dataset(id).map_err(|e| e.to_string())?;
            // Nothing to read in a dataset without labels (e.g. just created).
            if dataset.label_count() == 0 {
                return Ok(LabelStatus {
                    images: dataset.image_count(),
                    unlabelled: dataset.image_count(),
                    labels: 0,
                    classes: dataset.class_names().len() as u32,
                });
            }
            ctx.set_label(format!("Reading labels · {}", dataset.name()));
            let (_, set) = explore::cached_labels(&ws, id, &ctx.progress())?;
            Ok(edit::label_status(&set))
        })
        .await
    }
}
