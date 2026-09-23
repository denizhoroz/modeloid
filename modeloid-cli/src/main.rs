// use std::env;
// use std::path::PathBuf;

use clap::{ Parser, Subcommand };
use std::path::PathBuf;

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
    
    /// Init workspace in current directory
    Init,
}

fn main() {
    let cli = Cli::parse();
    let local_path = PathBuf::from(".");

    match cli.command {
        Commands::New { name } => cmd_new(&name),
        Commands::Init => cmd_init(),
    }

    // let mut ws = Workspace::new(String::from("New Workspace"), PathBuf::from("."));

    // println!("{new_ws:?}");
    // new_ws.set_description(String::from("A new workspace"));
    // println!("{new_ws:?}");
}   

fn cmd_new(name: &String) {
    let mut ws = Workspace::new(String::from(name), PathBuf::from("."));
    ws.save();
}

fn cmd_init() {

}
