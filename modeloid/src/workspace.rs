
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{error::Error, fs, path::PathBuf};

fn default_version() -> u32 {
    1
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Workspace {
    name: String, 
    path: PathBuf,
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

    pub fn save(&self) -> Result<(), Box<dyn Error>> {
        // Create workspace folders
        let root_dir = self.path.join(&self.name);
        fs::create_dir(&root_dir)?;
        fs::create_dir_all(root_dir.join(".cache").join("tmp"))?;
        fs::create_dir(root_dir.join("datasets"))?;
        fs::create_dir(root_dir.join("models"))?;
        fs::create_dir(root_dir.join("evals"))?;

        // Create workspace.toml
        let contents = toml::to_string(self)?;
        fs::write(root_dir.join("workspace.toml"), contents)?;
        Ok(())
    }
}