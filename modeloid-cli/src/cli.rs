
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

    /// Saved target class lists of the workspace
    ClassLists {
        #[command(subcommand)]
        command: ClassListCommands,
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
    /// Rebuild a dataset's dataset.toml from its files
    Rescan {
        /// Dataset name or id
        dataset: String,
    },
    /// Check a dataset for broken images, bad boxes and duplicates
    Check {
        /// Dataset name or id
        dataset: String,
        /// Apply the recommended fixes afterwards
        #[arg(long)]
        fix: bool,
        /// With --fix: name of the fixed copy (default: <dataset>-fixed)
        #[arg(long)]
        output: Option<String>,
        /// With --fix: replace the dataset instead of making a copy
        #[arg(long, conflicts_with = "output")]
        overwrite: bool,
    },
    /// Rename, merge or drop classes of one dataset
    Classes {
        /// Dataset name or id
        dataset: String,
        /// Rename or merge: OLD=NEW (NEW may be an existing class). Repeatable
        #[arg(long, value_name = "OLD=NEW")]
        rename: Vec<String>,
        /// Drop a class: NAME drops its boxes, NAME:images drops images containing it. Repeatable
        #[arg(long, value_name = "NAME[:images]")]
        drop: Vec<String>,
        /// Only show the result, don't write anything
        #[arg(long)]
        dry_run: bool,
        /// Name of the new dataset (default: <dataset>-classes)
        #[arg(long)]
        output: Option<String>,
        /// Replace the dataset instead of making a copy
        #[arg(long, conflicts_with = "output")]
        overwrite: bool,
    },
    /// Merge datasets, matching classes by name (or as a plan file says)
    Merge {
        /// Dataset names or ids, in order
        #[arg(required = true, num_args = 2..)]
        datasets: Vec<String>,
        /// Class mapping to use (TOML, as written by --write-plan)
        #[arg(long)]
        plan: Option<PathBuf>,
        /// Write the suggested plan to this file and stop, for editing
        #[arg(long)]
        write_plan: Option<PathBuf>,
        /// Output label format: yolo, coco or voc (default: the first dataset's)
        #[arg(long)]
        format: Option<String>,
        /// Keep each dataset's train/val/test instead of putting everything into train
        #[arg(long)]
        keep_splits: bool,
        /// Only show the result, don't write anything
        #[arg(long)]
        dry_run: bool,
        /// Name of the merged dataset (default: first-second-...)
        #[arg(long)]
        output: Option<String>,
    },
}

#[derive(Subcommand)]
enum ClassListCommands {
    /// List saved class lists
    List,
    /// Show one class list
    Show { name: String },
    /// Save a class list: classes in id order, comma-separated
    Save { name: String, classes: String },
    /// Delete a class list
    Delete { name: String },
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
            Commands::Dataset { command: DatasetCommands::Rescan { dataset } } => cmd_dataset_rescan(&dataset),
            Commands::Dataset { command: DatasetCommands::Check { dataset, fix, output, overwrite } } => {
                cmd_dataset_check(&dataset, fix, output, overwrite)
            }
            Commands::Dataset { command: DatasetCommands::Classes { dataset, rename, drop, dry_run, output, overwrite } } => {
                cmd_dataset_classes(&dataset, &rename, &drop, dry_run, output, overwrite)
            }
            Commands::Dataset {
                command: DatasetCommands::Merge { datasets, plan, write_plan, format, keep_splits, dry_run, output },
            } => cmd_dataset_merge(&datasets, plan, write_plan, format, keep_splits, dry_run, output),
            Commands::ClassLists { command } => cmd_class_lists(command),
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

/// The dataset with this name or id.
fn find_dataset(ws: &Workspace, dataset: &str) -> Result<Dataset, Error> {
    ws.list_datasets()?
        .into_iter()
        .map(|(_, d)| d)
        .find(|d| d.name() == dataset || d.id().to_string() == dataset)
        .ok_or_else(|| format!("no dataset named or numbered '{dataset}'").into())
}

fn cmd_dataset_rescan(dataset: &str) -> Result<(), Error> {
    let ws = _get_workspace()?;
    let d = find_dataset(&ws, dataset)?;
    let reporter = TermReporter::new();
    let progress = reporter.start("Scanning", Unit::Files);
    let result = ws.rescan_dataset(d.id(), progress.as_ref());
    progress.finish();
    let d = result?;
    println!("#{} {}: {} images, {} labels, {} classes", d.id(), d.name(), d.image_count(), d.label_count(), d.class_names().len());
    Ok(())
}

fn cmd_dataset_check(dataset: &str, fix: bool, output: Option<String>, overwrite: bool) -> Result<(), Error> {
    let ws = _get_workspace()?;
    let d = find_dataset(&ws, dataset)?;
    let reporter = TermReporter::new();
    let report = ws.health_check(d.id(), &reporter)?;
    let counts = report.counts();

    println!("#{} {}: {} images, {} labels", d.id(), d.name(), report.image_count, report.label_count);
    let lines = [
        (counts.corrupt_images, "corrupt or unreadable images"),
        (counts.size_mismatches, "images whose size differs from their labels"),
        (counts.orphan_labels, "label files without an image"),
        (counts.label_warnings, "unreadable label lines"),
        (counts.degenerate_boxes, "empty or zero-area boxes"),
        (counts.out_of_bounds, "boxes reaching outside the image"),
        (counts.unknown_classes, "objects with unknown class ids"),
        (counts.exact_groups, "groups of identical images"),
        (counts.near_groups, "groups of look-alike images"),
        (counts.leak_groups, "duplicate groups spanning splits (leaks)"),
        (counts.unlabeled_images, "images without labels (can be intentional)"),
    ];
    for (count, what) in lines.iter().filter(|(c, _)| *c > 0) {
        println!("  {count:>6}  {what}");
    }
    if counts.problems() == 0 {
        println!("No problems found.");
        return Ok(());
    }
    if !fix {
        println!("\nRun again with --fix to apply the recommended fixes.");
        return Ok(());
    }

    let fixes = modeloid::core::health::HealthFixes::recommended(&counts);
    if !fixes.any() {
        println!("Nothing the recommended fixes can repair automatically.");
        return Ok(());
    }
    let output = if overwrite {
        modeloid::Output::Overwrite { id: d.id() }
    } else {
        modeloid::Output::New { name: output.unwrap_or_else(|| format!("{}-fixed", d.name())) }
    };
    let (dir, summary) = ws.apply_health_fixes(d.id(), &fixes, &output, &reporter)?;
    println!(
        "Fixed: {} images removed, {} objects dropped, {} boxes clipped. Saved to {}",
        summary.removed_images,
        summary.dropped_objects,
        summary.clipped_objects,
        dir.display()
    );
    Ok(())
}

fn cmd_dataset_classes(
    dataset: &str,
    renames: &[String],
    drops: &[String],
    dry_run: bool,
    output: Option<String>,
    overwrite: bool,
) -> Result<(), Error> {
    use modeloid::core::classes::{ClassAction, DropMode};

    let ws = _get_workspace()?;
    let d = find_dataset(&ws, dataset)?;
    let mut plan = ws.class_merge_suggestion(&[d.id()])?;

    for rename in renames {
        let (old, new) = rename.split_once('=').ok_or_else(|| format!("--rename takes OLD=NEW, got '{rename}'"))?;
        let (old, new) = (old.trim(), new.trim());
        if !plan.targets.iter().any(|t| t == old) {
            return Err(format!("'{old}' isn't a class of {}", d.name()).into());
        }
        // Renaming onto an existing class merges the two.
        plan.rename_target(old, new);
        let mut seen = std::collections::HashSet::new();
        plan.targets.retain(|t| seen.insert(t.clone()));
    }
    for drop in drops {
        let (name, mode) = match drop.split_once(':') {
            Some((name, "images")) => (name.trim(), DropMode::Images),
            Some((_, other)) => return Err(format!("--drop takes NAME or NAME:images, got ':{other}'").into()),
            None => (drop.trim(), DropMode::Boxes),
        };
        let sources: Vec<String> = plan
            .rules
            .iter()
            .filter(|r| matches!(&r.action, ClassAction::To(t) if t == name))
            .map(|r| r.source.clone())
            .collect();
        if sources.is_empty() {
            return Err(format!("'{name}' isn't a class of {}", d.name()).into());
        }
        plan.remove_target(name);
        for source in sources {
            plan.set_rule(&source, ClassAction::Drop(mode));
        }
    }

    let preview = ws.class_merge_preview(&plan)?;
    print_preview(&plan, &preview);
    if dry_run || !preview.can_run() {
        return Ok(());
    }
    let output = if overwrite {
        modeloid::Output::Overwrite { id: d.id() }
    } else {
        modeloid::Output::New { name: output.unwrap_or_else(|| format!("{}-classes", d.name())) }
    };
    let (dir, summary) = ws.apply_class_merge(&plan, &output, &TermReporter::new())?;
    println!(
        "Done: {} images ({} removed), {} objects dropped. Saved to {}",
        summary.images,
        summary.removed_images,
        summary.dropped_objects,
        dir.display()
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_dataset_merge(
    datasets: &[String],
    plan_file: Option<PathBuf>,
    write_plan: Option<PathBuf>,
    format: Option<String>,
    keep_splits: bool,
    dry_run: bool,
    output: Option<String>,
) -> Result<(), Error> {
    use modeloid::core::classes::{MergePlan, SplitMode};

    let ws = _get_workspace()?;
    let inputs: Vec<Dataset> = datasets.iter().map(|d| find_dataset(&ws, d)).collect::<Result<_, _>>()?;
    let ids: Vec<u32> = inputs.iter().map(|d| d.id()).collect();

    let mut plan = match plan_file {
        Some(path) => {
            let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let mut plan: MergePlan = toml::from_str(&text).map_err(|e| format!("invalid plan {}: {e}", path.display()))?;
            plan.inputs = ids.clone();
            plan
        }
        None => ws.class_merge_suggestion(&ids)?,
    };
    if let Some(format) = format {
        plan.format = format;
    }
    if keep_splits {
        plan.split = SplitMode::Keep;
    }

    if let Some(path) = write_plan {
        std::fs::write(&path, toml::to_string(&plan)?)?;
        println!("Plan written to {}. Edit it, then run with --plan {}", path.display(), path.display());
        return Ok(());
    }

    let preview = ws.class_merge_preview(&plan)?;
    print_preview(&plan, &preview);
    if dry_run || !preview.can_run() {
        return Ok(());
    }
    let name = output.unwrap_or_else(|| inputs.iter().map(|d| d.name()).collect::<Vec<_>>().join("-"));
    let (dir, summary) = ws.apply_class_merge(&plan, &modeloid::Output::New { name }, &TermReporter::new())?;
    println!(
        "Merged: {} images ({} removed), {} objects dropped. Saved to {}",
        summary.images,
        summary.removed_images,
        summary.dropped_objects,
        dir.display()
    );
    Ok(())
}

fn print_preview(plan: &modeloid::core::classes::MergePlan, preview: &modeloid::core::classes::MergePreview) {
    println!("Inputs: {}", preview.input_names.join(", "));
    println!("Output: {} classes, {} labels, splits {:?}", plan.targets.len(), plan.format, plan.split);
    for (i, target) in preview.targets.iter().enumerate() {
        let counts: Vec<String> = target.labels.iter().map(u32::to_string).collect();
        println!("  {i:>3}  {:<24} {}", target.target, counts.join(" + "));
    }
    if preview.dropped_labels > 0 {
        println!("{} labels dropped", preview.dropped_labels);
    }
    if preview.dropped_images > 0 {
        println!("up to {} images dropped", preview.dropped_images);
    }
    let name = |id: u32| {
        plan.inputs.iter().position(|&i| i == id).and_then(|p| preview.input_names.get(p)).cloned().unwrap_or_else(|| format!("#{id}"))
    };
    for missing in &preview.missing_labels {
        let missing_in: Vec<String> = missing.missing_in.iter().map(|&id| name(id)).collect();
        println!(
            "Warning: '{}' isn't labelled in {}; its {} images may contain unlabeled {}",
            missing.target,
            missing_in.join(", "),
            missing.images_at_risk,
            missing.target
        );
    }
    for target in &preview.empty_targets {
        println!("Warning: target '{target}' gets no labels");
    }
    for (id, class) in &preview.unmapped {
        println!("Not mapped: '{class}' in {}", name(*id));
    }
    for problem in &preview.problems {
        println!("Problem: {problem}");
    }
    if !preview.can_run() {
        println!("Fix the problems above first (use --write-plan and --plan to edit the mapping).");
    }
}

fn cmd_class_lists(command: ClassListCommands) -> Result<(), Error> {
    use modeloid::core::classes::ClassList;

    let ws = _get_workspace()?;
    match command {
        ClassListCommands::List => {
            for list in ws.class_lists()? {
                println!("{:<24} {} classes", list.name, list.classes.len());
            }
        }
        ClassListCommands::Show { name } => {
            let list = ws.class_lists()?.into_iter().find(|l| l.name == name).ok_or_else(|| format!("no class list '{name}'"))?;
            for (i, class) in list.classes.iter().enumerate() {
                println!("{i:>3}  {class}");
            }
        }
        ClassListCommands::Save { name, classes } => {
            let classes: Vec<String> = classes.split(',').map(str::trim).filter(|c| !c.is_empty()).map(String::from).collect();
            ws.save_class_list(&ClassList { name: name.clone(), classes })?;
            println!("Saved class list '{name}'");
        }
        ClassListCommands::Delete { name } => {
            ws.delete_class_list(&name)?;
            println!("Deleted class list '{name}'");
        }
    }
    Ok(())
}

fn cmd_dataset_stats(dataset: &str, refresh: bool) -> Result<(), Error> {
    let ws = _get_workspace()?;
    let d = find_dataset(&ws, dataset)?;

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
