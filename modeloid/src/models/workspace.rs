
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{error::Error, fs, path::{Path, PathBuf}};


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

    pub fn download(&self, link: &String) -> Result<(), Box<dyn Error>> {
        let datasets_dir = self.path.join("datasets");

        Ok(())
    }

}