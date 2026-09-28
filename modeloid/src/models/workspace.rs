
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}, sync::Arc};

use crate::core::{
    download::{DownloadOptions, Downloader, Source, SourceInfo},
    io,
    progress::{Reporter, Unit},
    scan::scan_dataset,
};
use crate::models::dataset::{DATASET_FILE, Dataset, Lineage};

/// Where a tool's result goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// A new dataset folder `datasets/<name>`.
    New { name: String },
    /// Replace the dataset with this id, keeping its id, name and description.
    Overwrite { id: u32 },
}

/// Dataset names become folder names, so they follow folder-name rules.
pub fn validate_dataset_name(name: &str) -> Result<(), String> {
    if name.trim() != name {
        return Err("The name can't start or end with spaces.".into());
    }
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

    /// Downloads a dataset into the workspace's `datasets/` folder.
    pub async fn download(
        &self,
        link: &str,
        opts: &DownloadOptions,
        reporter: Arc<dyn Reporter>,
    ) -> Result<PathBuf, crate::Error> {
        self.download_to(link, opts, &self.datasets_dir(), reporter).await
    }

    /// Downloads a dataset into `target/<name>`. Staging still happens in the
    /// workspace's `.cache/tmp`, and the dataset id comes from this workspace.
    pub async fn download_to(
        &self,
        link: &str,
        opts: &DownloadOptions,
        target: &Path,
        reporter: Arc<dyn Reporter>,
    ) -> Result<PathBuf, crate::Error> {
        let downloader = Downloader::new(reporter.clone())?;
        let (source, info) = downloader.inspect(Source::from_link(link)?, opts).await?;
        let name = source.name(opts);

        let final_dir = target.join(&name);
        if final_dir.exists() {
            return Err(format!("dataset already exists: {}", final_dir.display()).into());
        }

        let tmp_dir = self.tmp_dir();
        tokio::fs::create_dir_all(&tmp_dir).await?;
        let archive = tmp_dir.join(format!("{name}.zip"));
        let extract_dir = tmp_dir.join(&name);

        let result: Result<(), crate::Error> = async {
            Self::fetch_into(&downloader, &source, opts, &archive, &extract_dir, reporter.as_ref()).await?;
            self.describe_download(&extract_dir, &name, link, &info, opts, reporter.as_ref()).await?;
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

    async fn fetch_into(
        downloader: &Downloader,
        source: &Source,
        opts: &DownloadOptions,
        archive: &Path,
        extract_dir: &Path,
        reporter: &dyn Reporter,
    ) -> Result<(), crate::Error> {
        let url = downloader.resolve(source, opts).await?;
        downloader.download_file(&url, archive).await?;
        io::extract_zip(archive, extract_dir, reporter).await?;
        Ok(())
    }

    // Written before the folder moves into datasets/, so it never appears there without a toml.
    async fn describe_download(
        &self,
        dir: &Path,
        name: &str,
        link: &str,
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
        dataset.set_annotation_format(opts.format.clone());
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

    #[test]
    fn download_future_is_send() {
        // Compile-time check: servers can run downloads on their own tasks.
        fn assert_send<T: Send>(_: &T) {}
        let ws = Workspace::new("x".into(), PathBuf::from("."));
        let opts = DownloadOptions { api_key: None, format: "coco".into() };
        let future = ws.download_to("https://example.com", &opts, Path::new("."), Arc::new(Silent));
        assert_send(&future);
    }

    #[test]
    fn validates_dataset_names() {
        assert!(validate_dataset_name("cats-v2").is_ok());
        assert!(validate_dataset_name("").is_err());
        assert!(validate_dataset_name(" cats").is_err());
        assert!(validate_dataset_name("a/b").is_err());
        assert!(validate_dataset_name("..").is_err());
    }
}
