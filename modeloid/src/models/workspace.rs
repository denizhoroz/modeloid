
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{error::Error, fs, path::{Path, PathBuf}, sync::Arc};

use crate::core::{
    download::{DownloadOptions, Downloader, Source, SourceInfo},
    io,
    progress::{Reporter, Unit},
    scan::scan_dataset,
};
use crate::models::dataset::{DATASET_FILE, Dataset};


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

    pub fn save_to_dir(&self) -> Result<(), Box<dyn Error>> {
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

    pub fn make_to_dir(&self) -> Result<(), Box<dyn Error>> {
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

    pub fn load_from_dir(dir: &Path) -> Result<Self, Box<dyn Error>> {
        // Read workspace.toml
        let contents = fs::read_to_string(dir.join("workspace.toml"))
            .map_err(|e| format!("cannot read workspace.toml in {}: {e}", dir.display()))?;

        let file: WorkspaceFile<Workspace> = toml::from_str(&contents)?;
        Ok(file.workspace)
    }

    pub fn display_info(&self) -> Result<(), Box<dyn Error>> {
        let contents = toml::to_string(self)?;
        println!("{contents}");

        Ok(())
    }

    fn _create_workspace_file(&self, root_dir: &PathBuf) -> Result<(), Box<dyn Error>> {
        let file = &WorkspaceFile { workspace: self };
        let contents = toml::to_string(file)?;
        fs::write(root_dir.join("workspace.toml"), contents)?;  

        Ok(())
    }

    pub fn datasets_dir(&self) -> PathBuf {
        self.path.join("datasets")
    }

    /// Datasets that have a dataset.toml, ordered by id.
    pub fn list_datasets(&self) -> Result<Vec<(PathBuf, Dataset)>, Box<dyn Error>> {
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

    pub fn next_dataset_id(&self) -> Result<u32, Box<dyn Error>> {
        let max = self.list_datasets()?.iter().map(|(_, d)| d.id()).max();
        Ok(max.map_or(1, |id| id + 1))
    }

    /// Writes a dataset.toml for every dataset folder that is missing one.
    pub fn scan_datasets(&self, reporter: &dyn Reporter) -> Result<Vec<Dataset>, Box<dyn Error>> {
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

    fn dataset_dirs(&self) -> Result<Vec<PathBuf>, Box<dyn Error>> {
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

    pub async fn download(
        &self,
        link: &str,
        opts: &DownloadOptions,
        reporter: Arc<dyn Reporter>,
    ) -> Result<PathBuf, Box<dyn Error>> {
        let downloader = Downloader::new(reporter.clone())?;
        let (source, info) = downloader.inspect(Source::from_link(link)?, opts).await?;
        let name = source.name(opts);

        let final_dir = self.datasets_dir().join(&name);
        if final_dir.exists() {
            return Err(format!("dataset already exists: {}", final_dir.display()).into());
        }

        let tmp_dir = self.path.join(".cache").join("tmp");
        tokio::fs::create_dir_all(&tmp_dir).await?;
        let archive = tmp_dir.join(format!("{name}.zip"));
        let extract_dir = tmp_dir.join(&name);

        let result: Result<(), Box<dyn Error>> = async {
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
    ) -> Result<(), Box<dyn Error>> {
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
    ) -> Result<Dataset, Box<dyn Error>> {
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