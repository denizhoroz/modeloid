
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use clap::{ Parser, Subcommand, ValueEnum };

use modeloid::Workspace;
use modeloid::Dataset;
use modeloid::DownloadOptions;
use modeloid::{Error, ImportMode, Reporter, Service, Settings, Unit};

use crate::progress::TermReporter;


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

    /// Download a dataset from Roboflow, Kaggle, Hugging Face or a direct zip link
    Download {
        /// URL
        link: String,

        /// Roboflow API key (falls back to the saved key, then ROBOFLOW_API_KEY)
        #[arg(long)]
        api_key: Option<String>,

        /// Roboflow export format (e.g. coco, yolov8, voc)
        #[arg(long, default_value = "coco")]
        format: String,
    },

    /// Import a local dataset folder or zip file into the workspace
    Import {
        /// Folder or .zip file
        path: PathBuf,

        /// Dataset name (defaults to the folder or zip name)
        #[arg(long)]
        name: Option<String>,

        /// Move the folder instead of copying it
        #[arg(long = "move")]
        move_folder: bool,
    },

    /// Manage datasets in the workspace
    Dataset {
        #[command(subcommand)]
        command: DatasetCommands,
    },

    /// User settings, shared with the GUI
    Settings {
        #[command(subcommand)]
        command: SettingsCommands,
    },
}

#[derive(Subcommand)]
enum SettingsCommands {
    /// Show where settings are stored and what is set
    Show,
    /// Save credentials for a service (Kaggle: kaggle.json contents or username:key)
    Set {
        service: ServiceArg,
        value: String,
    },
    /// Remove saved credentials for a service
    Remove {
        service: ServiceArg,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ServiceArg {
    Roboflow,
    Kaggle,
    Huggingface,
}

impl From<ServiceArg> for Service {
    fn from(arg: ServiceArg) -> Self {
        match arg {
            ServiceArg::Roboflow => Service::Roboflow,
            ServiceArg::Kaggle => Service::Kaggle,
            ServiceArg::Huggingface => Service::HuggingFace,
        }
    }
}

#[derive(Subcommand)]
enum DatasetCommands {
    /// Create dataset.toml for dataset folders that don't have one
    Scan,
    /// Show label statistics of a dataset
    Stats {
        /// Dataset name or id
        dataset: String,
        /// Recompute instead of using cached statistics
        #[arg(long)]
        refresh: bool,
    },
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
                let mut opts = Settings::load_default().download_options(format);
                if api_key.is_some() {
                    opts.api_key = api_key;
                }
                cmd_download(&link, opts).await
            }
            Commands::Import { path, name, move_folder } => cmd_import(&path, name, move_folder),
            Commands::Dataset { command: DatasetCommands::Scan } => cmd_dataset_scan(),
            Commands::Dataset { command: DatasetCommands::Stats { dataset, refresh } } => cmd_dataset_stats(&dataset, refresh),
            Commands::Settings { command } => cmd_settings(command),
        };

        if let Err(e) = result {
            eprintln!("Error: {e}");
        };
    }
}


fn cmd_new(name: &String) -> Result<(), Error> {
    let ws = Workspace::new(String::from(name), PathBuf::from("."));
    ws.save_to_dir()?;

    Ok(())
}

fn cmd_init() -> Result<(), Error> {
    let ws = Workspace::new(_get_dir_name()?, PathBuf::from("."));
    ws.make_to_dir()?;

    Ok(())
}

fn cmd_info() -> Result<(), Error> {
    let ws = _get_workspace()?;
    ws.display_info()?;

    Ok(())
}

async fn cmd_download(link: &str, opts: DownloadOptions) -> Result<(), Error> {
    let ws = _get_workspace()?;
    let dir = ws.download(link, &opts, Arc::new(TermReporter::new())).await?;
    println!("Dataset saved to {}", dir.display());

    Ok(())
}

fn cmd_dataset_stats(dataset: &str, refresh: bool) -> Result<(), Error> {
    let ws = _get_workspace()?;
    let found = ws
        .list_datasets()?
        .into_iter()
        .find(|(_, d)| d.name() == dataset || d.id().to_string() == dataset)
        .ok_or_else(|| format!("no dataset named or numbered '{dataset}'"))?;
    let (_, d) = found;

    let reporter = TermReporter::new();
    let progress = reporter.start("Reading labels", Unit::Files);
    let stats = ws.explore_stats(d.id(), refresh, progress.as_ref());
    progress.finish();
    let stats = stats?;

    let splits = ["train", "val", "test", "no split"];
    println!("#{} {} ({})", d.id(), d.name(), d.annotation_format());
    println!("{} images, {} labels, {} without labels", stats.image_count, stats.label_count, stats.empty_images.iter().sum::<u32>());
    for (i, name) in splits.iter().enumerate().filter(|(i, _)| stats.images[*i] > 0) {
        println!("  {name:<9} {:>7} images  {:>6} empty", stats.images[i], stats.empty_images[i]);
    }

    println!("
Classes (labels / images):");
    let mut classes: Vec<_> = stats.classes.iter().filter(|c| c.total_labels() > 0).collect();
    classes.sort_by_key(|c| std::cmp::Reverse(c.total_labels()));
    for class in &classes {
        println!("  {:<24} {:>7} / {:>6}", class.name, class.total_labels(), class.images);
    }
    let unused = stats.classes.len() - classes.len();
    if unused > 0 {
        println!("  ({unused} classes without labels)");
    }

    let [small, medium, large] = stats.size_classes;
    println!("
Object sizes: {small} small, {medium} medium, {large} large (COCO: <32², <96² px)");
    if stats.polygon_count > 0 {
        println!("{} objects have segmentation outlines", stats.polygon_count);
    }
    let sizes: Vec<String> = stats.image_sizes.iter().take(5).map(|s| format!("{}x{} ({})", s.width, s.height, s.count)).collect();
    println!("Image sizes: {}{}", sizes.join(", "), if stats.image_sizes.len() > 5 || stats.other_sizes > 0 { ", ..." } else { "" });
    if stats.warning_count > 0 {
        println!("
{} warnings while reading labels, first ones:", stats.warning_count);
        for warning in stats.warnings.iter().take(5) {
            println!("  {warning}");
        }
    }
    Ok(())
}

fn cmd_dataset_scan() -> Result<(), Error> {
    let ws = _get_workspace()?;
    let added: Vec<Dataset> = ws.scan_datasets(&TermReporter::new())?;

    if added.is_empty() {
        println!("All datasets already have dataset.toml");
    }
    for d in &added {
        println!("Added #{} {} ({} images, {} labels)", d.id(), d.name(), d.image_count(), d.label_count());
    }

    Ok(())
}


fn cmd_import(path: &PathBuf, name: Option<String>, move_folder: bool) -> Result<(), Error> {
    let ws = _get_workspace()?;
    let name = match name {
        Some(name) => name,
        None => path
            .file_stem()
            .ok_or("can't take a name from that path, pass --name")?
            .to_string_lossy()
            .into_owned(),
    };
    let reporter = TermReporter::new();
    let dir = if path.is_dir() {
        let mode = if move_folder { ImportMode::Move } else { ImportMode::Copy };
        ws.import_dir(path, &name, mode, &reporter)?
    } else {
        ws.import_zip(path, &name, &reporter)?
    };
    println!("Dataset imported to {}", dir.display());

    Ok(())
}

fn cmd_settings(command: SettingsCommands) -> Result<(), Error> {
    let path = Settings::default_path().ok_or("no config folder on this computer")?;
    let mut settings = Settings::load(&path)?;

    match command {
        SettingsCommands::Show => {
            println!("Settings file: {}", path.display());
            for service in Service::ALL {
                let state = match (settings.credential_hint(service), service.fallback()) {
                    (Some(hint), _) if service == Service::Kaggle => format!("saved for {hint}"),
                    (Some(hint), _) => format!("saved, ends in {hint}"),
                    (None, Some(fallback)) => format!("from {fallback}"),
                    (None, None) => "not set".to_string(),
                };
                println!("{}: {state}", service.label());
            }
        }
        SettingsCommands::Set { service, value } => {
            let service = Service::from(service);
            settings.set_credential(service, Some(&value))?;
            settings.save(&path)?;
            println!("{} credentials saved to {}", service.label(), path.display());
        }
        SettingsCommands::Remove { service } => {
            let service = Service::from(service);
            settings.set_credential(service, None)?;
            settings.save(&path)?;
            println!("{} credentials removed", service.label());
        }
    }
    Ok(())
}


fn _get_dir_name() -> Result<String, Error> {
    let path = env::current_dir()?;
    
    let name = path
                .file_name()
                .ok_or("Current directory has no name")?
                .to_string_lossy()
                .into_owned();

    Ok(name)
}

fn _get_workspace() -> Result<Workspace, Error> {
    Ok(Workspace::load_from_dir(&PathBuf::from("."))?)
}
