
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{error::Error, fs, path::{Path, PathBuf}};

use crate::core::{
    download::{DownloadOptions, Downloader, Source},
    io,
};


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

    pub async fn download(&self, link: &str, opts: &DownloadOptions) -> Result<PathBuf, Box<dyn Error>> {
        let downloader = Downloader::new()?;
        let source = downloader.pin_version(Source::from_link(link)?, opts).await?;
        let name = source.name(opts);

        let final_dir = self.path.join("datasets").join(&name);
        if final_dir.exists() {
            return Err(format!("dataset already exists: {}", final_dir.display()).into());
        }

        let tmp_dir = self.path.join(".cache").join("tmp");
        tokio::fs::create_dir_all(&tmp_dir).await?;
        let archive = tmp_dir.join(format!("{name}.zip"));
        let extract_dir = tmp_dir.join(&name);

        let result = Self::fetch_into(&downloader, &source, opts, &archive, &extract_dir, &final_dir).await;

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
        final_dir: &Path,
    ) -> Result<(), Box<dyn Error>> {
        let url = downloader.resolve(source, opts).await?;
        downloader.download_file(&url, archive).await?;
        io::extract_zip(archive, extract_dir).await?;
        io::move_dir(extract_dir, final_dir).await?;
        Ok(())
    }

}