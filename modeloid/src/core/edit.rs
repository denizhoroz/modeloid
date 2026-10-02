//! Datasets > Create and Edit: empty datasets, details, adding images and
//! labelling progress.

use std::{
    fs,
    path::{Path, PathBuf},
};

pub use modeloid_types::edit::{AddedImages, CREATE_FORMATS, CREATE_TYPES, DatasetDetails, LabelStatus, NewDataset};
use modeloid_types::edit::is_image_name;

use crate::core::{
    io as fsio,
    labels::LabelSet,
    progress::{Cancelled, Reporter, Unit},
    scan::{DatasetStats, scan_dataset},
};
use crate::models::{
    dataset::{Dataset, Lineage},
    workspace::{ImportMode, Workspace, validate_dataset_name},
};

/// Labelling progress from a dataset's labels.
pub fn label_status(set: &LabelSet) -> LabelStatus {
    LabelStatus {
        images: set.images.len() as u32,
        unlabelled: set.images.iter().filter(|i| i.objects.is_empty()).count() as u32,
        labels: set.images.iter().map(|i| i.objects.len() as u32).sum(),
        classes: set.classes.len() as u32,
    }
}

/// The folder new images of the dataset in `dir` go into: the train images
/// folder of the layouts Modeloid writes, else `images/` (made if missing).
pub fn images_target(dir: &Path) -> PathBuf {
    ["images/train", "train/images", "images", "train"]
        .iter()
        .map(|rel| dir.join(rel))
        .find(|path| path.is_dir())
        .unwrap_or_else(|| dir.join("images"))
}

/// A name for `name` that's free in `dir`: the name itself, or `stem-2.ext`, `stem-3.ext`...
fn free_name(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let (stem, ext) = name.rsplit_once('.').map_or((name, ""), |(s, e)| (s, e));
    (2..)
        .map(|n| if ext.is_empty() { format!("{stem}-{n}") } else { format!("{stem}-{n}.{ext}") })
        .find(|candidate| !dir.join(candidate).exists())
        .expect("some number is free")
}

/// Prefix of the `.cache/tmp` folders images are uploaded into before they're added.
const UPLOAD_PREFIX: &str = "upload-";

impl Workspace {
    /// Starts an upload of images from the browser: a fresh folder in
    /// `.cache/tmp`, named by the returned token.
    pub fn new_upload(&self) -> Result<String, crate::Error> {
        let dir = self.staging_dir("upload")?;
        Ok(dir.file_name().ok_or("upload folder has no name")?.to_string_lossy().into_owned())
    }

    /// The folder of upload `token` (`new_upload`).
    pub fn upload_dir(&self, token: &str) -> Result<PathBuf, crate::Error> {
        let well_formed = token
            .strip_prefix(UPLOAD_PREFIX)
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '-'));
        let dir = self.path().join(".cache").join("tmp").join(token);
        if !well_formed || !dir.is_dir() {
            return Err("that upload is gone; add the images again".into());
        }
        Ok(dir)
    }

    /// Where an uploaded file called `name` (as the browser gave it) is
    /// written in upload `token`: only its last path part, under a free name.
    pub fn upload_path(&self, token: &str, name: &str) -> Result<PathBuf, crate::Error> {
        let dir = self.upload_dir(token)?;
        let name = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
        if !is_image_name(name) {
            return Err(format!("'{name}' isn't an image").into());
        }
        Ok(dir.join(free_name(&dir, name)))
    }

    /// Moves the images of upload `token` into dataset `id` (`add_images`)
    /// and deletes the upload folder.
    pub fn add_uploaded_images(&self, id: u32, token: &str, reporter: &dyn Reporter) -> Result<AddedImages, crate::Error> {
        let dir = self.upload_dir(token)?;
        let mut files: Vec<PathBuf> = fs::read_dir(&dir)?.filter_map(Result::ok).map(|e| e.path()).collect();
        files.sort();
        let result = self.add_images(id, &files, ImportMode::Move, reporter);
        let _ = fs::remove_dir_all(&dir);
        result
    }

    /// Creates an empty dataset (an `images/` folder and its dataset.toml),
    /// ready for images to be added.
    pub fn create_dataset(&self, new: &NewDataset) -> Result<(PathBuf, Dataset), crate::Error> {
        validate_dataset_name(&new.name)?;
        if !CREATE_TYPES.iter().any(|(t, _)| *t == new.dataset_type) {
            return Err(format!("'{}' isn't a dataset type that can be created", new.dataset_type).into());
        }
        if !CREATE_FORMATS.iter().any(|(f, _)| *f == new.annotation_format) {
            return Err(format!("'{}' isn't a label format that can be created", new.annotation_format).into());
        }
        let dir = self.datasets_dir().join(&new.name);
        if dir.exists() {
            return Err(format!("a dataset folder named '{}' already exists", new.name).into());
        }

        let stats = DatasetStats {
            dataset_type: new.dataset_type.clone(),
            annotation_format: new.annotation_format.clone(),
            ..DatasetStats::default()
        };
        let mut dataset = Dataset::from_stats(self.next_dataset_id()?, new.name.clone(), stats);
        dataset.set_description(new.description.trim().to_string());
        dataset.set_lineage(Some(Lineage::new("create", &[])));

        // Made in staging, so the folder never shows in datasets/ without its toml.
        let staging = self.staging_dir("create")?;
        let made = (|| {
            fs::create_dir_all(staging.join("images"))?;
            dataset.save_to_dir(&staging)?;
            fs::create_dir_all(self.datasets_dir())?;
            fsio::rename(&staging, &dir)?;
            Ok::<_, crate::Error>(())
        })();
        if let Err(e) = made {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
        Ok((dir, dataset))
    }

    /// The details Edit shows for dataset `id`.
    pub fn dataset_details(&self, id: u32) -> Result<DatasetDetails, crate::Error> {
        let (_, dataset) = self.find_dataset(id)?;
        Ok(DatasetDetails {
            name: dataset.name().to_string(),
            description: dataset.description().to_string(),
            link: dataset.dataset_link().to_string(),
            license: dataset.license().to_string(),
            tags: dataset.tags().to_vec(),
        })
    }

    /// Saves new details for dataset `id`. A new name renames its folder too.
    pub fn edit_dataset(&self, id: u32, details: &DatasetDetails) -> Result<(PathBuf, Dataset), crate::Error> {
        validate_dataset_name(&details.name)?;
        let (mut dir, mut dataset) = self.find_dataset(id)?;

        if details.name != dataset.name() {
            let target = self.datasets_dir().join(&details.name);
            // On Windows a name that differs only in case is the same folder.
            let same_folder = target.exists() && target.canonicalize()? == dir.canonicalize()?;
            if target.exists() && !same_folder {
                return Err(format!("a dataset folder named '{}' already exists", details.name).into());
            }
            fsio::rename(&dir, &target)?;
            dir = target;
            dataset.set_name(details.name.clone());
        }
        dataset.set_description(details.description.trim().to_string());
        dataset.set_dataset_link(details.link.trim().to_string());
        dataset.set_license(details.license.trim().to_string());
        dataset.set_tags(details.tags.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect());
        dataset.touch();
        dataset.save_to_dir(&dir)?;
        Ok((dir, dataset))
    }

    /// Copies (or moves) the image `files` into dataset `id` (`images_target`),
    /// renaming ones whose name is taken, then updates its dataset.toml. Files
    /// that aren't images are skipped. A cancel keeps the images added so far.
    pub fn add_images(
        &self,
        id: u32,
        files: &[PathBuf],
        mode: ImportMode,
        reporter: &dyn Reporter,
    ) -> Result<AddedImages, crate::Error> {
        let (dir, mut dataset) = self.find_dataset(id)?;
        let target = images_target(&dir);
        fs::create_dir_all(&target)?;

        let mut added = AddedImages::default();
        let progress = reporter.start("Adding images", Unit::Files);
        progress.set_total(files.len() as u64);
        let mut cancelled = false;
        for file in files {
            if progress.is_cancelled() {
                cancelled = true;
                break;
            }
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !is_image_name(&name) || !file.is_file() {
                added.skipped.push(name);
                progress.add(1);
                continue;
            }
            let free = free_name(&target, &name);
            let to = target.join(&free);
            match mode {
                ImportMode::Copy => fs::copy(file, &to).map(|_| ())?,
                // Another drive can't rename; copy and delete instead.
                ImportMode::Move => fs::rename(file, &to).or_else(|_| fs::copy(file, &to).and_then(|_| fs::remove_file(file)))?,
            }
            added.added += 1;
            added.renamed += u32::from(free != name);
            progress.add(1);
        }
        progress.finish();

        let progress = reporter.start("Scanning", Unit::Files);
        let stats = scan_dataset(&dir, progress.as_ref());
        progress.finish();
        dataset.refresh(stats?);
        dataset.save_to_dir(&dir)?;

        if cancelled {
            return Err(Cancelled.into());
        }
        Ok(added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::progress::Silent;

    fn workspace(name: &str) -> Workspace {
        let dir = std::env::temp_dir().join(format!("modeloid-edit-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let ws = Workspace::new(name.into(), dir);
        ws.make_to_dir().unwrap();
        ws
    }

    fn new(name: &str) -> NewDataset {
        NewDataset {
            name: name.into(),
            description: " parked cars ".into(),
            dataset_type: "object-detection".into(),
            annotation_format: "yolo".into(),
        }
    }

    #[test]
    fn creates_edits_and_fills_a_dataset() {
        let ws = workspace("create");
        let (dir, dataset) = ws.create_dataset(&new("cars")).unwrap();
        assert_eq!(dir, ws.datasets_dir().join("cars"));
        assert!(dir.join("images").is_dir());
        assert_eq!((dataset.id(), dataset.image_count(), dataset.description()), (1, 0, "parked cars"));
        assert!(ws.create_dataset(&new("cars")).is_err(), "name taken");
        assert!(ws.create_dataset(&NewDataset { dataset_type: "x".into(), ..new("b") }).is_err());

        let details = DatasetDetails {
            name: "vehicles".into(),
            description: "cars and vans".into(),
            link: " https://example.com ".into(),
            license: "CC BY 4.0".into(),
            tags: vec!["road".into(), " ".into()],
        };
        let (dir, _) = ws.edit_dataset(1, &details).unwrap();
        assert!(!ws.datasets_dir().join("cars").exists());
        let saved = ws.dataset_details(1).unwrap();
        assert_eq!(saved, DatasetDetails { link: "https://example.com".into(), tags: vec!["road".into()], ..details });

        let source = ws.path().join("source");
        fs::create_dir_all(&source).unwrap();
        for name in ["a.jpg", "b.png", "notes.txt"] {
            fs::write(source.join(name), "x").unwrap();
        }
        fs::write(dir.join("images").join("a.jpg"), "x").unwrap();
        let files: Vec<PathBuf> = ["a.jpg", "b.png", "notes.txt"].iter().map(|n| source.join(n)).collect();
        let added = ws.add_images(1, &files, ImportMode::Copy, &Silent).unwrap();
        assert_eq!(added, AddedImages { added: 2, renamed: 1, skipped: vec!["notes.txt".into()] });
        assert!(dir.join("images").join("a-2.jpg").is_file());
        assert!(source.join("b.png").is_file(), "copied, not moved");

        let (_, dataset) = ws.find_dataset(1).unwrap();
        assert_eq!(dataset.image_count(), 3);
        assert_eq!((dataset.dataset_type(), dataset.annotation_format()), ("object-detection", "yolo"));

        let _ = fs::remove_dir_all(ws.path());
    }

    #[test]
    fn adds_uploaded_images() {
        let ws = workspace("upload");
        ws.create_dataset(&new("cars")).unwrap();
        let token = ws.new_upload().unwrap();
        assert!(ws.upload_dir("../x").is_err());
        assert!(ws.upload_path(&token, "notes.txt").is_err());
        let first = ws.upload_path(&token, r"C:\x\a.jpg").unwrap();
        fs::write(&first, "x").unwrap();
        let second = ws.upload_path(&token, "a.jpg").unwrap();
        assert_eq!(second.file_name().unwrap(), "a-2.jpg");
        fs::write(&second, "x").unwrap();

        let added = ws.add_uploaded_images(1, &token, &Silent).unwrap();
        assert_eq!(added.added, 2);
        assert!(ws.upload_dir(&token).is_err(), "upload folder is gone");
        assert_eq!(ws.find_dataset(1).unwrap().1.image_count(), 2);
        let _ = fs::remove_dir_all(ws.path());
    }

    #[test]
    fn finds_where_images_go() {
        let ws = workspace("target");
        let dir = ws.path().join("d");
        assert_eq!(images_target(&dir), dir.join("images"));
        fs::create_dir_all(dir.join("train/images")).unwrap();
        assert_eq!(images_target(&dir), dir.join("train/images"));
        let _ = fs::remove_dir_all(ws.path());
    }
}
