//! The labelling interface: a dataset's classes and per-image labels, edited
//! in place. Every change works on the cached `LabelSet` (`explore::edit_labels`)
//! and writes only the files it touches.

use dioxus::prelude::*;

pub use modeloid_types::edit::{validate_class_name, LabelClasses, LabelImage, LabelObject, LabelSession, SavedLabels};

/// Opens dataset `id` for labelling: its classes, label counts and which images have labels.
#[post("/api/label/session")]
pub async fn label_session(id: u32) -> Result<LabelSession, ServerFnError> {
    backend::session(id).await.map_err(ServerFnError::new)
}

/// Image `index` of dataset `id` with its labels.
#[post("/api/label/image")]
pub async fn label_image(id: u32, index: usize) -> Result<LabelImage, ServerFnError> {
    backend::image(id, index).await.map_err(ServerFnError::new)
}

/// Saves the labels of image `index` (called `image`) of dataset `id` (called `dataset`).
#[post("/api/label/save")]
pub async fn save_labels(id: u32, dataset: String, index: usize, image: String, objects: Vec<LabelObject>) -> Result<SavedLabels, ServerFnError> {
    backend::save(id, dataset, index, image, objects).await.map_err(ServerFnError::new)
}

/// Adds a class; returns the new class list.
#[post("/api/label/add-class")]
pub async fn add_label_class(id: u32, dataset: String, name: String) -> Result<LabelClasses, ServerFnError> {
    backend::add_class(id, dataset, name).await.map_err(ServerFnError::new)
}

/// Renames class `class`; returns the new class list.
#[post("/api/label/rename-class")]
pub async fn rename_label_class(id: u32, dataset: String, class: usize, name: String) -> Result<LabelClasses, ServerFnError> {
    backend::rename_class(id, dataset, class, name).await.map_err(ServerFnError::new)
}

/// Removes `classes` and all their labels; returns the session afresh (ids moved).
#[post("/api/label/remove-classes")]
pub async fn remove_label_classes(id: u32, dataset: String, classes: Vec<usize>) -> Result<LabelSession, ServerFnError> {
    backend::remove_classes(id, dataset, classes).await.map_err(ServerFnError::new)
}

#[cfg(feature = "server")]
mod backend {
    use modeloid::{core::annotate::class_totals, BBox, Dataset, LabelSet, Object, Workspace};

    use super::{LabelClasses, LabelImage, LabelObject, LabelSession, SavedLabels};
    use crate::api::explore::{cached_labels, edit_labels};
    use crate::api::{activity, history, jobs, workspace};

    fn open() -> Result<Workspace, String> {
        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))
    }

    fn session_of(dataset: &Dataset, set: &LabelSet, id: u32) -> LabelSession {
        LabelSession {
            dataset: dataset.name().to_string(),
            format: dataset.annotation_format().to_string(),
            classes: set.classes.clone(),
            colors: dataset.colors_for(&set.classes),
            totals: class_totals(set),
            labelled: set.images.iter().map(|i| !i.objects.is_empty()).collect(),
            busy: jobs::busy(id).err(),
        }
    }

    pub async fn session(id: u32) -> Result<LabelSession, String> {
        activity::track("label-session", "Reading labels", move |ctx| {
            let ws = open()?;
            let (_, dataset) = ws.find_dataset(id).map_err(|e| e.to_string())?;
            ctx.set_label(format!("Reading labels · {}", dataset.name()));
            let (_, set) = cached_labels(&ws, id, &ctx.progress())?;
            Ok(session_of(&dataset, &set, id))
        })
        .await
    }

    pub async fn image(id: u32, index: usize) -> Result<LabelImage, String> {
        activity::track("label-image", "Loading image", move |ctx| {
            let (_, set) = cached_labels(&open()?, id, &ctx.progress())?;
            let image = set.images.get(index).ok_or("That image isn't in the dataset any more.")?;
            Ok(LabelImage {
                index,
                name: image.name.clone(),
                split: image.split,
                width: image.width,
                height: image.height,
                objects: image
                    .objects
                    .iter()
                    .map(|o| LabelObject { class: o.class, bbox: [o.bbox.x, o.bbox.y, o.bbox.w, o.bbox.h], polygon: o.polygon.clone() })
                    .collect(),
            })
        })
        .await
    }

    /// Runs `change` on dataset `id`'s labels on a blocking thread, after
    /// checking that it's still the dataset the page opened and no job works on it.
    async fn change<T: Send + 'static>(
        id: u32,
        dataset: String,
        change: impl FnOnce(&Workspace, &mut LabelSet) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        tokio::task::spawn_blocking(move || {
            let ws = open()?;
            let (_, found) = ws.find_dataset(id).map_err(|e| e.to_string())?;
            if found.name() != dataset {
                return Err("The dataset has changed meanwhile. Open labelling again from Datasets > Edit.".into());
            }
            jobs::busy(id)?;
            edit_labels(&ws, id, |set| change(&ws, set))
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn save(id: u32, dataset: String, index: usize, image: String, objects: Vec<LabelObject>) -> Result<SavedLabels, String> {
        change(id, dataset, move |ws, set| {
            let current = set.images.get(index).ok_or("That image isn't in the dataset any more.")?;
            if current.name != image {
                return Err("The dataset's images have changed meanwhile. Reload the page.".into());
            }
            let (w, h) = (current.width as f64, current.height as f64);
            let objects: Vec<Object> = objects
                .into_iter()
                .filter(|o| o.bbox.iter().all(|v| v.is_finite()))
                .map(|o| {
                    // Kept inside the image.
                    let [x, y, bw, bh] = o.bbox;
                    let (x0, y0) = (x.clamp(0.0, w), y.clamp(0.0, h));
                    let (x1, y1) = ((x + bw).clamp(0.0, w), (y + bh).clamp(0.0, h));
                    Object { class: o.class, bbox: BBox { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }, polygon: o.polygon }
                })
                .filter(|o| o.bbox.w > 0.0 && o.bbox.h > 0.0)
                .collect();
            ws.save_image_labels(id, set, index, objects).map_err(|e| format!("Couldn't save: {e}"))?;
            Ok(SavedLabels { totals: class_totals(set), labelled: !set.images[index].objects.is_empty() })
        })
        .await
    }

    /// The classes of `set` with dataset `id`'s colours for them.
    fn classes_of(ws: &Workspace, id: u32, set: &LabelSet) -> Result<LabelClasses, String> {
        let (_, dataset) = ws.find_dataset(id).map_err(|e| e.to_string())?;
        Ok(LabelClasses { classes: set.classes.clone(), colors: dataset.colors_for(&set.classes) })
    }

    pub async fn add_class(id: u32, dataset: String, name: String) -> Result<LabelClasses, String> {
        let note = format!("In {dataset}");
        change(id, dataset, move |ws, set| {
            ws.add_label_class(id, set, &name).map_err(|e| e.to_string())?;
            history::record("Dataset Manager", "Labelling", "Add class", name.clone(), Some(note));
            classes_of(ws, id, set)
        })
        .await
    }

    pub async fn rename_class(id: u32, dataset: String, class: usize, name: String) -> Result<LabelClasses, String> {
        let note = format!("In {dataset}");
        change(id, dataset, move |ws, set| {
            let old = set.classes.get(class).cloned().unwrap_or_default();
            ws.rename_label_class(id, set, class, &name).map_err(|e| e.to_string())?;
            history::record("Dataset Manager", "Labelling", "Rename class", format!("{old} → {name}"), Some(note));
            classes_of(ws, id, set)
        })
        .await
    }

    pub async fn remove_classes(id: u32, dataset: String, classes: Vec<usize>) -> Result<LabelSession, String> {
        let note = format!("In {dataset}");
        change(id, dataset.clone(), move |ws, set| {
            let names: Vec<String> = classes.iter().filter_map(|&c| set.classes.get(c).cloned()).collect();
            ws.remove_label_classes(id, set, &classes).map_err(|e| e.to_string())?;
            history::record("Dataset Manager", "Labelling", "Remove classes", names.join(", "), Some(note));
            let (_, found) = ws.find_dataset(id).map_err(|e| e.to_string())?;
            Ok(session_of(&found, set, id))
        })
        .await
    }
}
