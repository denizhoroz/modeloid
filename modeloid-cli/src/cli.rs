
use std::env;
use std::error::Error;
use std::path::PathBuf;
use clap::{ Parser, Subcommand };

use modeloid::Workspace;
use modeloid::Dataset;
use modeloid::DownloadOptions;


#[derive(Parser)]
#[command(version, about, long_about = "A CLI interface for Modeloid library.",)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create new workspace
    New {
        /// Workspace name
        name: String,
    },
    
    /// Initialize workspace in current directory
    Init,

    /// Get current workspace information
    Info,

    /// Download a dataset using a link
    Download {
        /// URL
        link: String,

        /// API key for the source (falls back to ROBOFLOW_API_KEY)
        #[arg(long)]
        api_key: Option<String>,

        /// Export format (e.g. coco, yolov8, voc)
        #[arg(long, default_value = "coco")]
        format: String,
    }
}

pub struct Cli {

}
impl Cli {
    pub fn new() -> Cli {
        Cli {}
    }

    pub async fn collect(&self) {
        let args = Args::parse();

        let result = match args.command {
            Commands::New { name } => cmd_new(&name),
            Commands::Init => cmd_init(),
            Commands::Info => cmd_info(),
            Commands::Download { link, api_key, format } => {
                cmd_download(&link, DownloadOptions { api_key, format }).await
            }
        };

        if let Err(e) = result {
            eprintln!("Error: {e}");
        };
    }
}


fn cmd_new(name: &String) -> Result<(), Box<dyn Error>> {
    let ws = Workspace::new(String::from(name), PathBuf::from("."));
    ws.save_to_dir()?;

    Ok(())
}

fn cmd_init() -> Result<(), Box<dyn Error>> {
    let ws = Workspace::new(_get_dir_name()?, PathBuf::from("."));
    ws.make_to_dir()?;

    Ok(())
}

fn cmd_info() -> Result<(), Box<dyn Error>> {
    let ws = _get_workspace()?;
    ws.display_info()?;

    Ok(())
}

async fn cmd_download(link: &str, opts: DownloadOptions) -> Result<(), Box<dyn Error>> {
    let ws = _get_workspace()?;
    let dir = ws.download(link, &opts).await?;
    println!("Dataset saved to {}", dir.display());

    Ok(())
}


fn _get_dir_name() -> Result<(String), Box<dyn Error>> {
    let path = env::current_dir()?;
    
    let name = path
                .file_name()
                .ok_or("Current directory has no name")?
                .to_string_lossy()
                .into_owned();

    Ok(name)
}

fn _get_workspace() -> Result<(Workspace), Box<dyn Error>> {
    Ok(Workspace::load_from_dir(&PathBuf::from("."))?)
}
