
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}, sync::Arc};

use crate::core::{
    download::{DownloadOptions, Downloader, Source, SourceInfo},
    io,
    labels::{self, LabelFormat, LabelSet},
    progress::{Progress, Reporter, Unit, check_cancel},
    scan::scan_dataset,
    stats::{self, ExploreStats, STATS_VERSION},
};
use crate::models::dataset::{DATASET_FILE, Dataset, Lineage};

/// How `Workspace::import_dir` brings a folder in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    Copy,
    /// Moves the folder; fast on the same drive, and the original is gone.
    Move,
}

impl ImportMode {
    pub fn name(self) -> &'static str {
        match self {
            ImportMode::Copy => "copy",
            ImportMode::Move => "move",
        }
    }
}

/// Where a tool's result goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// A new dataset folder `datasets/<name>`.
    New { name: String },
    /// Replace the dataset with this id, keeping its id, name and description.
    Overwrite { id: u32 },
}

pub use modeloid_types::validate_dataset_name;


#[derive(Serialize, Deserialize)]
struct WorkspaceFile<T> {
    workspace: T,
}


fn default_version() -> u32 {
    1
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Workspace {
    name: String, 
    path: PathBuf,
    #[serde(default = "default_version")]
    format_version: u32,
    // created_at: DateTime<Utc>,
    seed: u64,
    description: String,
}
impl Workspace {
    pub fn new(name: String, path: PathBuf) -> Self{
        Workspace {
            name,
            path,
            format_version: default_version(),
            // created_at: Utc::now(),
            seed: 42,
            description: String::new(),
        }
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name; 
    }

    pub fn set_description(&mut self, description: String) {
        self.description = description;
    }

    pub fn save_to_dir(&self) -> Result<(), crate::Error> {
        // Create workspace folders
        let root_dir = &self.path.join(&self.name);
        fs::create_dir(&root_dir)?;
        fs::create_dir_all(root_dir.join(".cache").join("tmp"))?;
        fs::create_dir(root_dir.join("datasets"))?;
        fs::create_dir(root_dir.join("models"))?;
        fs::create_dir(root_dir.join("evals"))?;

        // Create workspace.toml
        self._create_workspace_file(root_dir)?;
        Ok(())
    }

    pub fn make_to_dir(&self) -> Result<(), crate::Error> {
        // Create workspace folders
        let root_dir = &self.path;
        fs::create_dir_all(root_dir.join(".cache").join("tmp"))?;
        fs::create_dir(root_dir.join("datasets"))?;
        fs::create_dir(root_dir.join("models"))?;
        fs::create_dir(root_dir.join("evals"))?;

        // Create workspace.toml
        self._create_workspace_file(root_dir)?;
        Ok(())
    }

    pub fn load_from_dir(dir: &Path) -> Result<Self, crate::Error> {
        // Read workspace.toml
        let contents = fs::read_to_string(dir.join("workspace.toml"))
            .map_err(|e| format!("cannot read workspace.toml in {}: {e}", dir.display()))?;

        let mut file: WorkspaceFile<Workspace> = toml::from_str(&contents)?;
        // The stored path can be relative (the CLI writes "."), so trust where it was loaded from.
        file.workspace.path = dir.to_path_buf();
        Ok(file.workspace)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn display_info(&self) -> Result<(), crate::Error> {
        let contents = toml::to_string(self)?;
        println!("{contents}");

        Ok(())
    }

    fn _create_workspace_file(&self, root_dir: &PathBuf) -> Result<(), crate::Error> {
        let file = &WorkspaceFile { workspace: self };
        let contents = toml::to_string(file)?;
        fs::write(root_dir.join("workspace.toml"), contents)?;  

        Ok(())
    }

    pub fn datasets_dir(&self) -> PathBuf {
        self.path.join("datasets")
    }

    /// Datasets that have a dataset.toml, ordered by id.
    pub fn list_datasets(&self) -> Result<Vec<(PathBuf, Dataset)>, crate::Error> {
        let mut datasets = Vec::new();
        for dir in self.dataset_dirs()? {
            if dir.join(DATASET_FILE).is_file() {
                let dataset = Dataset::load_from_dir(&dir)?;
                datasets.push((dir, dataset));
            }
        }
        datasets.sort_by_key(|(_, d)| d.id());
        Ok(datasets)
    }

    pub fn next_dataset_id(&self) -> Result<u32, crate::Error> {
        let max = self.list_datasets()?.iter().map(|(_, d)| d.id()).max();
        Ok(max.map_or(1, |id| id + 1))
    }

    /// Cheap summary of the workspace at `dir` on disk: changes when
    /// `workspace.toml` or a dataset folder / `dataset.toml` is added, removed,
    /// renamed or edited. Used to notice changes made outside Modeloid.
    pub fn fingerprint(dir: &Path) -> u64 {
        use std::hash::{Hash, Hasher};

        let stamp = |path: &Path| fs::metadata(path).ok().map(|m| (m.len(), m.modified().ok()));
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        stamp(&dir.join("workspace.toml")).hash(&mut hasher);

        let mut entries: Vec<_> = fs::read_dir(dir.join("datasets"))
            .map(|rd| rd.filter_map(Result::ok).filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).collect())
            .unwrap_or_default();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            entry.file_name().hash(&mut hasher);
            stamp(&entry.path().join(DATASET_FILE)).hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Labels of dataset `id`, read in its annotation format.
    pub fn read_labels(&self, id: u32, progress: &dyn Progress) -> Result<(PathBuf, Dataset, LabelSet), crate::Error> {
        let (dir, dataset) = self.find_dataset(id)?;
        let format = LabelFormat::parse(dataset.annotation_format()).ok_or_else(|| {
            format!(
                "'{}' labels aren't supported yet; YOLO, COCO and Pascal VOC are",
                dataset.annotation_format()
            )
        })?;
        let set = labels::read(&dir, format, progress)?;
        Ok((dir, dataset, set))
    }

    /// Explore statistics for dataset `id`. Cached in `.cache/stats/<id>.json`
    /// until its `dataset.toml` changes; `refresh` recomputes anyway.
    pub fn explore_stats(&self, id: u32, refresh: bool, progress: &dyn Progress) -> Result<ExploreStats, crate::Error> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Cached {
            version: u32,
            stamp: String,
            stats: ExploreStats,
        }

        let (dir, _) = self.find_dataset(id)?;
        let meta = fs::metadata(dir.join(DATASET_FILE))?;
        let stamp = format!("{}-{:?}", meta.len(), meta.modified().ok());
        let cache = self.path.join(".cache").join("stats").join(format!("{id}.json"));

        if !refresh {
            let cached = fs::read(&cache).ok().and_then(|bytes| serde_json::from_slice::<Cached>(&bytes).ok());
            if let Some(cached) = cached.filter(|c| c.version == STATS_VERSION && c.stamp == stamp) {
                return Ok(cached.stats);
            }
        }

        let (_, _, set) = self.read_labels(id, progress)?;
        let stats = stats::analyze(&set);
        if let Some(parent) = cache.parent() {
            fs::create_dir_all(parent)?;
        }
        let cached = Cached { version: STATS_VERSION, stamp, stats };
        fs::write(&cache, serde_json::to_vec(&cached)?)?;
        Ok(cached.stats)
    }

    /// Where previews of dataset `id`'s images are cached.
    pub fn thumbs_dir(&self, id: u32) -> PathBuf {
        self.path.join(".cache").join("thumbs").join(id.to_string())
    }

    /// The dataset with this id and its folder.
    pub fn find_dataset(&self, id: u32) -> Result<(PathBuf, Dataset), crate::Error> {
        self.list_datasets()?
            .into_iter()
            .find(|(_, d)| d.id() == id)
            .ok_or_else(|| format!("no dataset with id {id}").into())
    }

    /// A fresh, empty folder in `.cache/tmp` for a tool to write its result
    /// into before `save_output` moves it into place.
    pub fn staging_dir(&self, operation: &str) -> Result<PathBuf, crate::Error> {
        let dir = self.tmp_dir().join(format!("{operation}-{}", timestamp()));
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Scans a tool's result in `staging`, writes its dataset.toml (with
    /// `lineage`) and moves it to `output`. Returns the final folder. On error
    /// the source dataset is left as it was and `staging` stays for inspection.
    pub fn save_output(
        &self,
        staging: &Path,
        output: &Output,
        lineage: Lineage,
        reporter: &dyn Reporter,
    ) -> Result<PathBuf, crate::Error> {
        // Check the target before the (slow) scan.
        let replaced = match output {
            Output::New { name } => {
                validate_dataset_name(name)?;
                if self.datasets_dir().join(name).exists() {
                    return Err(format!("a dataset folder named '{name}' already exists").into());
                }
                None
            }
            Output::Overwrite { id } => Some(self.find_dataset(*id)?),
        };

        let progress = reporter.start("Scanning", Unit::Files);
        let stats = scan_dataset(staging, progress.as_ref());
        progress.finish();
        let stats = stats?;

        let Some((dir, mut dataset)) = replaced else {
            let Output::New { name } = output else { unreachable!() };
            let mut dataset = Dataset::from_stats(self.next_dataset_id()?, name.clone(), stats);
            dataset.set_lineage(Some(lineage));
            dataset.save_to_dir(staging)?;
            let dir = self.datasets_dir().join(name);
            io::move_dir_blocking(staging, &dir, reporter).map_err(|e| e.to_string())?;
            return Ok(dir);
        };

        dataset.refresh(stats);
        dataset.set_lineage(Some(lineage));
        dataset.save_to_dir(staging)?;

        // Swap folders; put the original back if the new one can't move in.
        let backup = self.tmp_dir().join(format!("{}-replaced-{}", dataset.name(), timestamp()));
        io::move_dir_blocking(&dir, &backup, reporter).map_err(|e| e.to_string())?;
        if let Err(e) = io::move_dir_blocking(staging, &dir, reporter) {
            let _ = fs::remove_dir_all(&dir);
            io::move_dir_blocking(&backup, &dir, reporter).map_err(|e| e.to_string())?;
            return Err(e.to_string().into());
        }
        let _ = fs::remove_dir_all(&backup);
        Ok(dir)
    }

    fn tmp_dir(&self) -> PathBuf {
        self.path.join(".cache").join("tmp")
    }

    /// Writes a dataset.toml for every dataset folder that is missing one.
    pub fn scan_datasets(&self, reporter: &dyn Reporter) -> Result<Vec<Dataset>, crate::Error> {
        let mut next_id = self.next_dataset_id()?;
        let mut added = Vec::new();

        for dir in self.dataset_dirs()? {
            if dir.join(DATASET_FILE).exists() {
                continue;
            }
            let name = dir.file_name().ok_or("dataset folder has no name")?.to_string_lossy().into_owned();
            let progress = reporter.start("Scanning", Unit::Files);
            let stats = scan_dataset(&dir, progress.as_ref());
            progress.finish();
            let dataset = Dataset::from_stats(next_id, name, stats?);
            dataset.save_to_dir(&dir)?;
            next_id += 1;
            added.push(dataset);
        }
        Ok(added)
    }

    fn dataset_dirs(&self) -> Result<Vec<PathBuf>, crate::Error> {
        let datasets_dir = self.datasets_dir();
        let mut dirs = Vec::new();
        for entry in fs::read_dir(&datasets_dir)
            .map_err(|e| format!("cannot read {}: {e}", datasets_dir.display()))?
        {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                dirs.push(entry.path());
            }
        }
        dirs.sort();
        Ok(dirs)
    }

    /// Downloads a dataset into `datasets/<name>`. Staging happens in the
    /// workspace's `.cache/tmp`.
    pub async fn download(
        &self,
        link: &str,
        opts: &DownloadOptions,
        reporter: Arc<dyn Reporter>,
    ) -> Result<PathBuf, crate::Error> {
        let downloader = Downloader::new(reporter.clone())?;
        let (source, info) = downloader.inspect(Source::from_link(link)?, opts).await?;
        let name = source.name(opts);

        let final_dir = self.datasets_dir().join(&name);
        if final_dir.exists() {
            return Err(format!("dataset already exists: {}", final_dir.display()).into());
        }

        let tmp_dir = self.tmp_dir();
        tokio::fs::create_dir_all(&tmp_dir).await?;
        let archive = tmp_dir.join(format!("{name}.zip"));
        let extract_dir = tmp_dir.join(&name);
        let _ = tokio::fs::remove_dir_all(&extract_dir).await; // leftovers of an earlier failed run

        let result: Result<(), crate::Error> = async {
            downloader.fetch(&source, opts, &archive, &extract_dir).await?;
            check_cancel(reporter.is_cancelled())?;
            self.describe_download(&extract_dir, &name, link, &source, &info, opts, reporter.as_ref()).await?;
            check_cancel(reporter.is_cancelled())?;
            io::move_dir(&extract_dir, &final_dir, reporter.clone()).await
        }
        .await;

        let _ = tokio::fs::remove_file(&archive).await;
        let _ = tokio::fs::remove_file(archive.with_extension("part")).await;
        if result.is_err() {
            let _ = tokio::fs::remove_dir_all(&extract_dir).await;
        }

        result.map(|_| final_dir)
    }

    /// Imports the dataset folder `src` as `datasets/<name>`, copying it or
    /// moving it. A move is undone if the import fails.
    pub fn import_dir(&self, src: &Path, name: &str, mode: ImportMode, reporter: &dyn Reporter) -> Result<PathBuf, crate::Error> {
        if !src.is_dir() {
            return Err(format!("{} isn't a folder", src.display()).into());
        }
        // Canonical paths only for comparing; the record keeps the readable one.
        let src = std::path::absolute(src)?;
        let canonical = src.canonicalize()?;
        let workspace = self.path.canonicalize()?;
        if workspace.starts_with(&canonical) {
            return Err("that folder contains this workspace; pick the dataset folder itself".into());
        }
        if canonical.starts_with(workspace.join("datasets")) {
            return Err("that folder is already in this workspace's datasets folder".into());
        }
        self.check_new_name(name)?;

        let staging = self.staging_dir("import")?;
        let placed = match mode {
            ImportMode::Copy => {
                let progress = reporter.start("Copying", Unit::Files);
                let result = io::copy_dir_blocking(&src, &staging, progress.as_ref());
                progress.finish();
                result
            }
            ImportMode::Move => {
                fs::remove_dir(&staging)?;
                io::move_dir_blocking(&src, &staging, reporter)
            }
        };
        if let Err(e) = placed {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }

        let lineage = Lineage::new("import", &[])
            .with_param("source", src.display().to_string())
            .with_param("mode", mode.name());
        self.save_output(&staging, &Output::New { name: name.to_string() }, lineage, reporter)
            .inspect_err(|_| match mode {
                ImportMode::Copy => {
                    let _ = fs::remove_dir_all(&staging);
                }
                ImportMode::Move => {
                    let _ = io::move_dir_blocking(&staging, &src, reporter);
                }
            })
    }

    /// Imports a dataset zip as `datasets/<name>`. A single top-level folder
    /// inside the archive is unwrapped.
    pub fn import_zip(&self, zip: &Path, name: &str, reporter: &dyn Reporter) -> Result<PathBuf, crate::Error> {
        if !zip.is_file() {
            return Err(format!("{} isn't a file", zip.display()).into());
        }
        if !io::is_zip(zip)? {
            return Err(format!("{} isn't a zip file", zip.display()).into());
        }
        self.check_new_name(name)?;

        let staging = self.staging_dir("import")?;
        let result = (|| {
            let progress = reporter.start("Extracting", Unit::Files);
            let extracted = io::extract_zip_blocking(zip, &staging, progress.as_ref());
            progress.finish();
            extracted?;
            io::unwrap_single_dir(&staging)?;
            check_cancel(reporter.is_cancelled())?;

            let source = std::path::absolute(zip).unwrap_or_else(|_| zip.to_path_buf());
            let lineage = Lineage::new("import", &[]).with_param("source", source.display().to_string());
            self.save_output(&staging, &Output::New { name: name.to_string() }, lineage, reporter)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    fn check_new_name(&self, name: &str) -> Result<(), crate::Error> {
        validate_dataset_name(name)?;
        if self.datasets_dir().join(name).exists() {
            return Err(format!("a dataset folder named '{name}' already exists").into());
        }
        Ok(())
    }

    // Written before the folder moves into datasets/, so it never appears there without a toml.
    async fn describe_download(
        &self,
        dir: &Path,
        name: &str,
        link: &str,
        source: &Source,
        info: &SourceInfo,
        opts: &DownloadOptions,
        reporter: &dyn Reporter,
    ) -> Result<Dataset, crate::Error> {
        let scan_dir = dir.to_path_buf();
        let progress = reporter.start("Scanning", Unit::Files);
        let stats = tokio::task::spawn_blocking(move || {
            let stats = scan_dataset(&scan_dir, progress.as_ref());
            progress.finish();
            stats
        })
        .await??;

        let mut dataset = Dataset::from_stats(self.next_dataset_id()?, name.to_string(), stats);
        // Roboflow knows the exact export format (e.g. "yolov8"); others keep the scanner's guess.
        if matches!(source, Source::Roboflow { .. }) {
            dataset.set_annotation_format(opts.format.clone());
        }
        if !info.dataset_type.is_empty() {
            dataset.set_dataset_type(info.dataset_type.clone());
        }
        dataset.set_license(info.license.clone());
        dataset.set_dataset_link(link.to_string());
        dataset.save_to_dir(dir)?;
        Ok(dataset)
    }
}

/// Sortable, file-name-safe time stamp for temporary folders.
fn timestamp() -> String {
    Utc::now().format("%Y%m%d-%H%M%S-%3f").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::progress::Silent;

    struct TempWorkspace(Workspace);

    impl TempWorkspace {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("modeloid-ws-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let ws = Workspace::new(name.into(), dir);
            ws.make_to_dir().unwrap();
            TempWorkspace(ws)
        }

        fn staging_with(&self, files: &[(&str, &str)]) -> PathBuf {
            let dir = self.0.staging_dir("test").unwrap();
            for (rel, contents) in files {
                let path = dir.join(rel);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, contents).unwrap();
            }
            dir
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(self.0.path());
        }
    }

    const YOLO: &[(&str, &str)] = &[
        ("data.yaml", "names: ['cat']\n"),
        ("train/images/a.jpg", "x"),
        ("train/labels/a.txt", "0 0.5 0.5 0.1 0.1\n"),
    ];

    fn new_output(name: &str) -> Output {
        Output::New { name: name.into() }
    }

    #[test]
    fn saves_new_output_with_lineage() {
        let tmp = TempWorkspace::new("new");
        let ws = &tmp.0;
        let staging = tmp.staging_with(YOLO);
        let first = ws.save_output(&staging, &new_output("cats"), Lineage::new("import", &[]), &Silent).unwrap();
        assert_eq!(first, ws.datasets_dir().join("cats"));
        assert!(!staging.exists());

        let (_, parent) = ws.find_dataset(1).unwrap();
        let staging = tmp.staging_with(YOLO);
        let lineage = Lineage::new("split", &[&parent]).with_param("ratio", 0.8);
        let dir = ws.save_output(&staging, &new_output("cats-split"), lineage, &Silent).unwrap();

        let saved = Dataset::load_from_dir(&dir).unwrap();
        assert_eq!((saved.id(), saved.name(), saved.image_count()), (2, "cats-split", 1));
        assert_eq!(saved.lineage().unwrap().parents[0].name, "cats");

        let staging = tmp.staging_with(YOLO);
        let taken = ws.save_output(&staging, &new_output("cats"), Lineage::new("x", &[]), &Silent);
        assert!(taken.is_err());
        assert!(staging.exists(), "staging is kept on error");
    }

    #[test]
    fn overwrite_keeps_identity() {
        let tmp = TempWorkspace::new("overwrite");
        let ws = &tmp.0;
        let staging = tmp.staging_with(YOLO);
        let dir = ws.save_output(&staging, &new_output("cats"), Lineage::new("import", &[]), &Silent).unwrap();
        let mut original = Dataset::load_from_dir(&dir).unwrap();
        original.set_description("my cats".into());
        original.save_to_dir(&dir).unwrap();

        let mut files = YOLO.to_vec();
        files.push(("valid/images/b.jpg", "x"));
        let staging = tmp.staging_with(&files);
        let lineage = Lineage::new("transform", &[&original]);
        let out = ws.save_output(&staging, &Output::Overwrite { id: 1 }, lineage, &Silent).unwrap();

        assert_eq!(out, dir);
        let saved = Dataset::load_from_dir(&dir).unwrap();
        assert_eq!((saved.id(), saved.name(), saved.image_count()), (1, "cats", 2));
        assert_eq!(saved.description(), "my cats");
        assert_eq!(saved.lineage().unwrap().operation, "transform");
        assert!(dir.join("valid/images/b.jpg").is_file());
        assert_eq!(fs::read_dir(ws.tmp_dir()).unwrap().count(), 0, "staging and backup are gone");
    }

    fn yolo_folder(dir: &Path) {
        for (rel, contents) in YOLO {
            let path = dir.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }

    #[test]
    fn imports_folder_by_copy_and_move() {
        let tmp = TempWorkspace::new("import-dir");
        let ws = &tmp.0;
        let src = ws.path().join("outside-src");
        yolo_folder(&src);

        let dir = ws.import_dir(&src, "cats", ImportMode::Copy, &Silent).unwrap();
        assert!(src.join("data.yaml").is_file(), "copy keeps the source");
        let saved = Dataset::load_from_dir(&dir).unwrap();
        assert_eq!((saved.name(), saved.image_count()), ("cats", 1));
        let lineage = saved.lineage().unwrap();
        assert_eq!((lineage.operation.as_str(), lineage.params["mode"].as_str()), ("import", Some("copy")));

        assert!(ws.import_dir(&src, "cats", ImportMode::Copy, &Silent).is_err(), "name taken");
        assert!(ws.import_dir(ws.path(), "whole", ImportMode::Copy, &Silent).is_err(), "contains workspace");
        assert!(ws.import_dir(&dir, "again", ImportMode::Copy, &Silent).is_err(), "already inside");

        ws.import_dir(&src, "cats-moved", ImportMode::Move, &Silent).unwrap();
        assert!(!src.exists(), "move takes the source");
        assert!(ws.datasets_dir().join("cats-moved/train/images/a.jpg").is_file());
        assert_eq!(fs::read_dir(ws.tmp_dir()).unwrap().count(), 0);
    }

    #[test]
    fn imports_zip_and_unwraps_single_folder() {
        use std::io::Write;
        let tmp = TempWorkspace::new("import-zip");
        let ws = &tmp.0;
        let zip_path = ws.path().join("cats.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
        for (rel, contents) in YOLO {
            zip.start_file(format!("cats-export/{rel}"), zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(contents.as_bytes()).unwrap();
        }
        zip.finish().unwrap();

        let dir = ws.import_zip(&zip_path, "cats", &Silent).unwrap();
        assert!(dir.join("data.yaml").is_file(), "top folder unwrapped");
        assert_eq!(Dataset::load_from_dir(&dir).unwrap().image_count(), 1);

        let not_zip = ws.path().join("notes.txt");
        fs::write(&not_zip, "hello").unwrap();
        assert!(ws.import_zip(&not_zip, "notes", &Silent).is_err());
        assert_eq!(fs::read_dir(ws.tmp_dir()).unwrap().count(), 0);
    }

    #[test]
    fn cancelled_import_cleans_up() {
        struct Cancelling;
        impl Reporter for Cancelling {
            fn start(&self, _: &'static str, _: Unit) -> Box<dyn crate::Progress> {
                Box::new(CancelledProgress)
            }
            fn message(&self, _: &str) {}
            fn is_cancelled(&self) -> bool {
                true
            }
        }
        struct CancelledProgress;
        impl crate::Progress for CancelledProgress {
            fn set_total(&self, _: u64) {}
            fn add(&self, _: u64) {}
            fn finish(&self) {}
            fn is_cancelled(&self) -> bool {
                true
            }
        }

        let tmp = TempWorkspace::new("import-cancel");
        let ws = &tmp.0;
        let src = ws.path().join("outside-src");
        yolo_folder(&src);
        let err = ws.import_dir(&src, "cats", ImportMode::Copy, &Cancelling).unwrap_err();
        assert!(err.downcast_ref::<crate::Cancelled>().is_some(), "{err}");
        assert!(!ws.datasets_dir().join("cats").exists());
        assert_eq!(fs::read_dir(ws.tmp_dir()).unwrap().count(), 0);
    }

    #[test]
    fn fingerprint_notices_outside_changes() {
        let tmp = TempWorkspace::new("fingerprint");
        let ws = &tmp.0;
        let before = Workspace::fingerprint(ws.path());
        assert_eq!(before, Workspace::fingerprint(ws.path()), "stable without changes");

        let staging = tmp.staging_with(YOLO);
        let dir = ws.save_output(&staging, &new_output("cats"), Lineage::new("import", &[]), &Silent).unwrap();
        let added = Workspace::fingerprint(ws.path());
        assert_ne!(before, added);

        fs::rename(&dir, ws.datasets_dir().join("dogs")).unwrap();
        let renamed = Workspace::fingerprint(ws.path());
        assert_ne!(added, renamed);

        fs::remove_dir_all(ws.datasets_dir().join("dogs")).unwrap();
        assert_eq!(Workspace::fingerprint(ws.path()), before, "back to the empty state");
    }

    #[test]
    fn explore_stats_are_cached_until_dataset_changes() {
        let tmp = TempWorkspace::new("stats");
        let ws = &tmp.0;
        let staging = tmp.staging_with(&[("data.yaml", "names: ['cat']\n"), ("train/labels/a.txt", "0 0.5 0.5 0.2 0.2\n")]);
        fs::create_dir_all(staging.join("train/images")).unwrap();
        image::RgbImage::new(64, 32).save(staging.join("train/images/a.png")).unwrap();
        ws.save_output(&staging, &new_output("cats"), Lineage::new("import", &[]), &Silent).unwrap();

        let stats = ws.explore_stats(1, false, &Silent).unwrap();
        assert_eq!((stats.image_count, stats.label_count), (1, 1));
        assert_eq!(stats.image_sizes[0].width, 64);

        // A doctored cache entry is served while dataset.toml is unchanged...
        let cache = ws.path().join(".cache/stats/1.json");
        let text = fs::read_to_string(&cache).unwrap().replace("\"image_count\":1", "\"image_count\":999");
        fs::write(&cache, text).unwrap();
        assert_eq!(ws.explore_stats(1, false, &Silent).unwrap().image_count, 999);
        // ...and replaced on refresh.
        assert_eq!(ws.explore_stats(1, true, &Silent).unwrap().image_count, 1);
        assert!(ws.explore_stats(7, false, &Silent).is_err(), "unknown id");
    }

    #[test]
    fn download_future_is_send() {
        // Compile-time check: servers can run downloads on their own tasks.
        fn assert_send<T: Send>(_: &T) {}
        let ws = Workspace::new("x".into(), PathBuf::from("."));
        let opts = DownloadOptions { format: "coco".into(), ..Default::default() };
        let future = ws.download("https://example.com", &opts, Arc::new(Silent));
        assert_send(&future);
    }
}
