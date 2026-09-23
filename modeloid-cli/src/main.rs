use clap::{ Parser, Subcommand };
use std::error::Error;
use std::path::PathBuf;
use std::env;

use modeloid::Workspace;
use modeloid::Dataset;

#[derive(Parser)]
#[command(version, about, long_about = "A CLI interface for Modeloid library.",)]
struct Cli {
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
}

fn cmd_new(name: &String) -> Result<(), Box<dyn Error>> {
    let ws = Workspace::new(String::from(name), PathBuf::from("."));
    ws.save_to_dir()?;

    Ok(())
}

fn cmd_init() -> Result<(), Box<dyn Error>> {
    let ws = Workspace::new(get_dir_name()?, PathBuf::from("."));
    ws.make_to_dir()?;

    Ok(())
}

fn get_dir_name() -> Result<(String), Box<dyn Error>> {
    let path = env::current_dir()?;
    
    let name = path
                .file_name()
                .ok_or("Current directory has no name")?
                .to_string_lossy()
                .into_owned();

    Ok(name)
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::New { name } => cmd_new(&name),
        Commands::Init => cmd_init(),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}   


