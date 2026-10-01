
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}, sync::Arc};

use crate::core::{
    balance::{self, BalancePlan, BalancePreview, FilterMode, Resplit},
    classes::{self, ClassList, MergePlan, MergePreview, MergeSummary, SplitMode},
    convert::{self, ConvertPlan, ConvertPreview, CropSummary, TargetTask},
    download::{AlreadyExists, DownloadOptions, Downloader, Inspected, Source, SourceInfo},
    health::{self, FixSummary, HEALTH_VERSION, HealthFixes, HealthReport},
    io,
    labels::{self, LabelFormat, LabelSet},
    progress::{Progress, Reporter, Unit, check_cancel},
    scan::{DatasetStats, scan_dataset},
    stats::{self, ExploreStats, STATS_VERSION},
};
use crate::models::dataset::{DATASET_FILE, Dataset, Lineage, ParentRef};
use crate::models::parameters::ParameterSet;

/// Workspace folder for training: `training/parameters`, `training/trains`.
pub const TRAINING_DIR: &str = "training";
pub const PARAMETERS_DIR: &str = "parameters";
pub const TRAINS_DIR: &str = "trains";

fn create_training_dirs(root: &Path) -> std::io::Result<()> {
    fs::create_dir_all(root.join(TRAINING_DIR).join(PARAMETERS_DIR))?;
    fs::create_dir_all(root.join(TRAINING_DIR).join(TRAINS_DIR))
}

/// Brings an older workspace to the current layout: `models/` (which only
/// held `parameters/`) becomes `training/`, and the training folders exist.
/// Best effort: a read-only workspace still loads.
fn upgrade_layout(root: &Path) {
    let (old, new) = (root.join("models"), root.join(TRAINING_DIR));
    if old.is_dir() && !new.exists() {
        let _ = crate::core::io::rename(&old, &new);
    }
    if !new.join(TRAINS_DIR).is_dir() || !new.join(PARAMETERS_DIR).is_dir() {
        let _ = create_training_dirs(root);
    }
}

/// How `Workspace::import_dir` brings a folder in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    Copy,
    /// Moves the folder; fast on the same drive, and the original is gone.
    Move,
}

impl ImportMode {
    pub fn name(self) -> &'static str {
        match self {
            ImportMode::Copy => "copy",
            ImportMode::Move => "move",
        }
    }
}

/// What `Workspace::apply_convert` wrote.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConvertSummary {
    /// Images written (for classification: crops).
    pub images: u32,
    /// Outlines that became plain boxes.
    pub outlines_lost: u32,
    /// Classification only.
    pub crops: CropSummary,
}

/// Where a tool's result goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// A new dataset folder `datasets/<name>`.
    New { name: String },
    /// Replace the dataset with this id, keeping its id, name and description.
    Overwrite { id: u32 },
}

pub use modeloid_types::validate_dataset_name;


#[derive(Serialize, Deserialize)]
struct WorkspaceFile<T> {
    workspace: T,
}


fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    name: String, 
    path: PathBuf,
    #[serde(default = "default_version")]
    format_version: u32,
    // created_at: DateTime<Utc>,
    seed: u64,
    description: String,
    /// Label format tools start on ("yolo", "coco", "voc"); empty = follow the dataset.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    default_format: String,
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
            default_format: String::new(),
        }
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name; 
    }

    pub fn set_description(&mut self, description: String) {
        self.description = description;
    }

    pub fn set_seed(&mut self, seed: u64) {
        self.seed = seed;
    }

    /// The label format tools start on, if the workspace sets one.
    pub fn default_format(&self) -> Option<LabelFormat> {
        LabelFormat::parse(&self.default_format)
    }

    /// Sets the label format tools start on (any name `LabelFormat::parse`
    /// knows), or clears it with `None` so tools follow the dataset.
    pub fn set_default_format(&mut self, format: Option<&str>) -> Result<(), crate::Error> {
        self.default_format = match format.map(str::trim).filter(|f| !f.is_empty()) {
            Some(name) => LabelFormat::parse(name).ok_or_else(|| format!("unknown label format '{name}'; use yolo, coco or voc"))?.name().to_string(),
            None => String::new(),
        };
        Ok(())
    }

    /// Writes `workspace.toml` of a loaded workspace again, after a change.
    pub fn save(&self) -> Result<(), crate::Error> {
        // The stored path isn't read back (loading uses the folder itself), so keep it portable.
        let mut stored = self.clone();
        stored.path = PathBuf::from(".");
        stored._create_workspace_file(&self.path)
    }

    pub fn save_to_dir(&self) -> Result<(), crate::Error> {
        // Create workspace folders
        let root_dir = &self.path.join(&self.name);
        fs::create_dir(&root_dir)?;
        fs::create_dir_all(root_dir.join(".cache").join("tmp"))?;
        fs::create_dir(root_dir.join("datasets"))?;
        create_training_dirs(root_dir)?;
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
        create_training_dirs(root_dir)?;
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
        upgrade_layout(dir);
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

    /// Seed for random choices (splits, subsets), so results can be repeated.
    pub fn seed(&self) -> u64 {
        self.seed
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

    /// Cheap summary of the workspace at `dir` on disk: changes when
    /// `workspace.toml` or a dataset folder / `dataset.toml` is added, removed,
    /// renamed or edited. Used to notice changes made outside Modeloid.
    pub fn fingerprint(dir: &Path) -> u64 {
        use std::hash::{Hash, Hasher};

        let stamp = |path: &Path| fs::metadata(path).ok().map(|m| (m.len(), m.modified().ok()));
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        stamp(&dir.join("workspace.toml")).hash(&mut hasher);

        let mut entries: Vec<_> = fs::read_dir(dir.join("datasets"))
            .map(|rd| rd.filter_map(Result::ok).filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).collect())
            .unwrap_or_default();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            entry.file_name().hash(&mut hasher);
            stamp(&entry.path().join(DATASET_FILE)).hash(&mut hasher);
        }

        // Saved parameter sets, so hand edits show in the table too.
        let mut params: Vec<_> = fs::read_dir(dir.join(TRAINING_DIR).join(PARAMETERS_DIR))
            .map(|rd| rd.filter_map(Result::ok).collect())
            .unwrap_or_default();
        params.sort_by_key(|e| e.file_name());
        for entry in params {
            entry.file_name().hash(&mut hasher);
            stamp(&entry.path()).hash(&mut hasher);
        }

        // Training runs' records (written when a run starts and ends).
        let mut runs: Vec<_> = fs::read_dir(dir.join(TRAINING_DIR).join(TRAINS_DIR))
            .map(|rd| rd.filter_map(Result::ok).collect())
            .unwrap_or_default();
        runs.sort_by_key(|e| e.file_name());
        for entry in runs {
            entry.file_name().hash(&mut hasher);
            stamp(&entry.path().join("train.toml")).hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Labels of dataset `id`, read in its annotation format.
    pub fn read_labels(&self, id: u32, progress: &dyn Progress) -> Result<(PathBuf, Dataset, LabelSet), crate::Error> {
        let (dir, dataset) = self.find_dataset(id)?;
        let format = LabelFormat::parse(dataset.annotation_format()).ok_or_else(|| {
            format!(
                "'{}' labels aren't supported yet; YOLO, COCO and Pascal VOC are",
                dataset.annotation_format()
            )
        })?;
        let set = labels::read(&dir, format, progress)?;
        Ok((dir, dataset, set))
    }

    /// Explore statistics for dataset `id`. Cached in `.cache/stats/<id>.json`
    /// until its `dataset.toml` changes; `refresh` recomputes anyway.
    pub fn explore_stats(&self, id: u32, refresh: bool, progress: &dyn Progress) -> Result<ExploreStats, crate::Error> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Cached {
            version: u32,
            stamp: String,
            stats: ExploreStats,
        }

        let (dir, _) = self.find_dataset(id)?;
        let stamp = dataset_stamp(&dir)?;
        let cache = self.path.join(".cache").join("stats").join(format!("{id}.json"));

        if !refresh {
            let cached = fs::read(&cache).ok().and_then(|bytes| serde_json::from_slice::<Cached>(&bytes).ok());
            if let Some(cached) = cached.filter(|c| c.version == STATS_VERSION && c.stamp == stamp) {
                return Ok(cached.stats);
            }
        }

        let (_, _, set) = self.read_labels(id, progress)?;
        let stats = stats::analyze(&set);
        if let Some(parent) = cache.parent() {
            fs::create_dir_all(parent)?;
        }
        let cached = Cached { version: STATS_VERSION, stamp, stats };
        fs::write(&cache, serde_json::to_vec(&cached)?)?;
        Ok(cached.stats)
    }

    fn health_cache(&self, id: u32) -> PathBuf {
        self.path.join(".cache").join("health").join(format!("{id}.json"))
    }

    /// Where Health Check remembers which images it has decoded, for every dataset of the workspace.
    fn image_cache(&self) -> PathBuf {
        self.path.join(".cache").join("images").join("checked.json")
    }

    /// Runs Health Check on dataset `id` and caches the report in
    /// `.cache/health/<id>.json`. Every image it hasn't seen before is decoded.
    pub fn health_check(&self, id: u32, reporter: &dyn Reporter) -> Result<HealthReport, crate::Error> {
        let (dir, _) = self.find_dataset(id)?;
        let stamp = dataset_stamp(&dir)?;
        let reading = reporter.start("Reading labels", Unit::Files);
        let read = self.read_labels(id, reading.as_ref());
        reading.finish();
        let (_, dataset, set) = read?;
        let format = LabelFormat::parse(dataset.annotation_format()).ok_or("unsupported label format")?;
        let orphans = labels::orphan_labels(&dir, format)?;

        let images = health::ImageCache::load(&self.image_cache());
        let checking = reporter.start("Checking images", Unit::Files);
        let report = health::check(&dir, &set, orphans, stamp, &images, checking.as_ref());
        checking.finish();
        let report = report?;
        // Losing the cache only costs time on the next check.
        if let Err(e) = images.save(&self.image_cache()) {
            reporter.message(&format!("Couldn't save the image cache: {e}"));
        }

        let cache = self.health_cache(id);
        fs::create_dir_all(cache.parent().unwrap_or(&self.path))?;
        fs::write(&cache, serde_json::to_vec(&report)?)?;
        Ok(report)
    }

    /// The last Health Check report of dataset `id`, and whether it still
    /// matches the dataset (its `dataset.toml` unchanged since).
    pub fn health_report(&self, id: u32) -> Result<Option<(HealthReport, bool)>, crate::Error> {
        let (dir, _) = self.find_dataset(id)?;
        let Ok(bytes) = fs::read(self.health_cache(id)) else { return Ok(None) };
        let Ok(report) = serde_json::from_slice::<HealthReport>(&bytes) else { return Ok(None) };
        if report.version != HEALTH_VERSION {
            return Ok(None);
        }
        let fresh = report.stamp == dataset_stamp(&dir)?;
        Ok(Some((report, fresh)))
    }

    /// Applies Health Check fixes to dataset `id`: its labels are fixed in
    /// memory, written in the same format to a staging folder, and saved as
    /// `output`. Needs an up-to-date report. Only images and labels are kept.
    pub fn apply_health_fixes(
        &self,
        id: u32,
        fixes: &HealthFixes,
        output: &Output,
        reporter: &dyn Reporter,
    ) -> Result<(PathBuf, FixSummary), crate::Error> {
        let (report, fresh) = self.health_report(id)?.ok_or("run Health Check first")?;
        if !fresh {
            return Err("the dataset changed since it was checked; run Health Check again".into());
        }
        let reading = reporter.start("Reading labels", Unit::Files);
        let read = self.read_labels(id, reading.as_ref());
        reading.finish();
        let (dir, dataset, mut set) = read?;
        let format = LabelFormat::parse(dataset.annotation_format()).ok_or("unsupported label format")?;

        let summary = health::apply_fixes(&dir, &mut set, &report, fixes);
        let staging = self.staging_dir("health-fix")?;
        let writing = reporter.start("Writing", Unit::Files);
        let written = labels::write(&set, &staging, format, writing.as_ref());
        writing.finish();
        let described = match written {
            Ok(written) => labels::describe(&set, format, written),
            Err(e) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(e.into());
            }
        };

        let lineage = Lineage::new("health-fix", &[&dataset])
            .with_param("removed_images", summary.removed_images as i64)
            .with_param("dropped_objects", summary.dropped_objects as i64)
            .with_param("clipped_objects", summary.clipped_objects as i64)
            .with_param("fixes", fix_names(fixes));
        match self.save_described(&staging, output, lineage, described, reporter) {
            Ok(dir) => Ok((dir, summary)),
            Err(e) => {
                let _ = fs::remove_dir_all(&staging);
                Err(e)
            }
        }
    }

    /// Rebuilds dataset `id`'s `dataset.toml` from its files (after manual
    /// edits), keeping its id, name, description and history.
    pub fn rescan_dataset(&self, id: u32, progress: &dyn Progress) -> Result<Dataset, crate::Error> {
        let (dir, mut dataset) = self.find_dataset(id)?;
        let stats = scan_dataset(&dir, progress)?;
        dataset.refresh(stats);
        dataset.save_to_dir(&dir)?;
        Ok(dataset)
    }

    fn class_lists_dir(&self) -> PathBuf {
        self.path.join("classes")
    }

    /// Saved target class lists (presets), by name.
    pub fn class_lists(&self) -> Result<Vec<ClassList>, crate::Error> {
        let dir = self.class_lists_dir();
        let Ok(entries) = fs::read_dir(&dir) else { return Ok(Vec::new()) };
        let mut lists = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "toml") {
                let text = fs::read_to_string(&path)?;
                lists.push(toml::from_str::<ClassList>(&text).map_err(|e| format!("invalid {}: {e}", path.display()))?);
            }
        }
        lists.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(lists)
    }

    /// Saves (or replaces) a class list as `classes/<name>.toml`.
    pub fn save_class_list(&self, list: &ClassList) -> Result<(), crate::Error> {
        validate_dataset_name(&list.name)?;
        if list.classes.is_empty() {
            return Err("a class list needs at least one class".into());
        }
        fs::create_dir_all(self.class_lists_dir())?;
        fs::write(self.class_lists_dir().join(format!("{}.toml", list.name)), toml::to_string(list)?)?;
        Ok(())
    }

    pub fn delete_class_list(&self, name: &str) -> Result<(), crate::Error> {
        validate_dataset_name(name)?;
        fs::remove_file(self.class_lists_dir().join(format!("{name}.toml"))).map_err(|e| format!("cannot delete class list '{name}': {e}"))?;
        Ok(())
    }

    /// `training/`: everything about training models.
    pub fn training_dir(&self) -> PathBuf {
        self.path.join(TRAINING_DIR)
    }

    /// `training/parameters`: saved training parameter sets, one TOML file each.
    pub fn parameters_dir(&self) -> PathBuf {
        self.training_dir().join(PARAMETERS_DIR)
    }

    /// `training/trains`: training runs.
    pub fn trains_dir(&self) -> PathBuf {
        self.training_dir().join(TRAINS_DIR)
    }

    fn parameter_file(&self, name: &str) -> PathBuf {
        self.parameters_dir().join(format!("{name}.toml"))
    }

    /// Saved parameter sets by name, and a warning for each file that
    /// couldn't be read (those are left out).
    pub fn parameter_sets(&self) -> Result<(Vec<ParameterSet>, Vec<String>), crate::Error> {
        let Ok(entries) = fs::read_dir(self.parameters_dir()) else { return Ok((Vec::new(), Vec::new())) };
        let (mut sets, mut warnings) = (Vec::new(), Vec::new());
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "toml") {
                continue;
            }
            let read = fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|text| toml::from_str::<ParameterSet>(&text).map_err(|e| e.to_string()));
            match read {
                // The file name is the name; a hand-renamed file keeps working.
                Ok(mut set) => {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        set.name = stem.to_string();
                    }
                    sets.push(set);
                }
                Err(e) => warnings.push(format!("{} couldn't be read: {e}", path.display())),
            }
        }
        sets.sort_by_key(|s| s.name.to_lowercase());
        Ok((sets, warnings))
    }

    /// Saves `set` as `models/parameters/<name>.toml` and returns it as saved.
    /// `replacing` is the name of the set being edited (`None` adds a new
    /// one); a different name renames it. Never overwrites another set.
    pub fn save_parameter_set(&self, set: &ParameterSet, replacing: Option<&str>) -> Result<ParameterSet, crate::Error> {
        if let Some(problem) = set.problems().into_iter().next() {
            return Err(problem.into());
        }
        let file = self.parameter_file(&set.name);
        let renamed = replacing.is_some_and(|old| old != set.name);
        if (replacing.is_none() || renamed) && file.exists() {
            return Err(format!("a parameter set named '{}' already exists", set.name).into());
        }

        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let created = match replacing {
            Some(old) => {
                let path = self.parameter_file(old);
                let text = fs::read_to_string(&path).map_err(|e| format!("cannot read parameter set '{old}': {e}"))?;
                toml::from_str::<ParameterSet>(&text).map(|s| s.created).unwrap_or_default()
            }
            None => String::new(),
        };
        let saved = ParameterSet { created: if created.is_empty() { now.clone() } else { created }, updated: now, ..set.clone() };

        fs::create_dir_all(self.parameters_dir())?;
        fs::write(&file, toml::to_string(&saved)?)?;
        if let Some(old) = replacing.filter(|_| renamed) {
            fs::remove_file(self.parameter_file(old)).map_err(|e| format!("saved '{}' but couldn't remove '{old}': {e}", set.name))?;
        }
        Ok(saved)
    }

    pub fn delete_parameter_set(&self, name: &str) -> Result<(), crate::Error> {
        validate_dataset_name(name)?;
        fs::remove_file(self.parameter_file(name)).map_err(|e| format!("cannot delete parameter set '{name}': {e}"))?;
        Ok(())
    }

    /// Name and statistics of each input, for suggestions and previews.
    /// Statistics of each input; reads labels (counted on `progress`) for
    /// inputs without cached statistics.
    fn merge_inputs(&self, ids: &[u32], progress: &dyn Progress) -> Result<Vec<(u32, String, ExploreStats)>, crate::Error> {
        ids.iter()
            .map(|&id| {
                let (_, dataset) = self.find_dataset(id)?;
                Ok((id, dataset.name().to_string(), self.explore_stats(id, false, progress)?))
            })
            .collect()
    }

    /// A first class mapping for `ids` (same-looking names joined), writing
    /// in the workspace's default format, else the first input's format family.
    pub fn class_merge_suggestion(&self, ids: &[u32], progress: &dyn Progress) -> Result<MergePlan, crate::Error> {
        let inputs = self.merge_inputs(ids, progress)?;
        let format = match ids.first() {
            _ if self.default_format().is_some() => self.default_format().map_or("", LabelFormat::name),
            Some(&id) => {
                let (_, dataset) = self.find_dataset(id)?;
                LabelFormat::parse(dataset.annotation_format()).unwrap_or(LabelFormat::Yolo).name()
            }
            None => LabelFormat::Yolo.name(),
        };
        let stats: Vec<classes::InputStats> =
            inputs.iter().map(|(id, name, stats)| classes::InputStats { id: *id, name, stats }).collect();
        Ok(classes::suggest(&stats, format))
    }

    /// What `plan` would produce, from cached statistics.
    pub fn class_merge_preview(&self, plan: &MergePlan, progress: &dyn Progress) -> Result<MergePreview, crate::Error> {
        let inputs = self.merge_inputs(&plan.inputs, progress)?;
        let stats: Vec<classes::InputStats> =
            inputs.iter().map(|(id, name, stats)| classes::InputStats { id: *id, name, stats }).collect();
        Ok(classes::preview(plan, &stats))
    }

    /// Applies a class mapping to one or more datasets and saves the result
    /// as `output` (overwrite only with a single input).
    pub fn apply_class_merge(
        &self,
        plan: &MergePlan,
        output: &Output,
        reporter: &dyn Reporter,
    ) -> Result<(PathBuf, MergeSummary), crate::Error> {
        if matches!(output, Output::Overwrite { .. }) && plan.inputs.len() != 1 {
            return Err("overwriting works with exactly one input dataset".into());
        }
        let format = LabelFormat::parse(&plan.format).ok_or_else(|| format!("unknown output format '{}'", plan.format))?;

        let mut sets = Vec::new();
        let mut parents = Vec::new();
        for &id in &plan.inputs {
            let reading = reporter.start("Reading labels", Unit::Files);
            let read = self.read_labels(id, reading.as_ref());
            reading.finish();
            let (_, dataset, set) = read?;
            sets.push((id, set));
            parents.push(dataset);
        }
        let (set, summary) = classes::apply(plan, sets)?;

        let staging = self.staging_dir("classes-merge")?;
        let writing = reporter.start("Writing", Unit::Files);
        let written = labels::write(&set, &staging, format, writing.as_ref());
        writing.finish();
        let described = match written {
            Ok(written) => labels::describe(&set, format, written),
            Err(e) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(e.into());
            }
        };

        let parent_refs: Vec<&Dataset> = parents.iter().collect();
        let lineage = Lineage::new(if plan.inputs.len() > 1 { "merge" } else { "classes" }, &parent_refs)
            .with_param("targets", plan.targets.clone())
            .with_param("format", format.name())
            .with_param("split", if plan.split == SplitMode::AllTrain { "all-train" } else { "keep" })
            .with_param("removed_images", summary.removed_images as i64)
            .with_param("dropped_objects", summary.dropped_objects as i64);
        match self.save_described(&staging, output, lineage, described, reporter) {
            Ok(dir) => Ok((dir, summary)),
            Err(e) => {
                let _ = fs::remove_dir_all(&staging);
                Err(e)
            }
        }
    }

    /// What a Split & Balance `plan` does to `set` (its input's labels),
    /// with the workspace's seed unless the plan has its own.
    pub fn balance_preview_of(&self, plan: &BalancePlan, set: &LabelSet) -> BalancePreview {
        balance::preview(&self.seeded(plan), set)
    }

    /// What a Split & Balance `plan` does to its input; reads its labels.
    pub fn balance_preview(&self, plan: &BalancePlan, progress: &dyn Progress) -> Result<BalancePreview, crate::Error> {
        let (_, _, set) = self.read_labels(plan.input, progress)?;
        Ok(self.balance_preview_of(plan, &set))
    }

    fn seeded(&self, plan: &BalancePlan) -> BalancePlan {
        BalancePlan { seed: Some(plan.seed.unwrap_or(self.seed)), ..plan.clone() }
    }

    /// Applies a Split & Balance plan to its input and saves the result as
    /// `output`, in the input's label format.
    pub fn apply_balance(
        &self,
        plan: &BalancePlan,
        output: &Output,
        reporter: &dyn Reporter,
    ) -> Result<(PathBuf, BalancePreview), crate::Error> {
        let plan = self.seeded(plan);
        let reading = reporter.start("Reading labels", Unit::Files);
        let read = self.read_labels(plan.input, reading.as_ref());
        reading.finish();
        let (_, dataset, set) = read?;
        let format = LabelFormat::parse(dataset.annotation_format()).ok_or("unsupported label format")?;
        let (set, summary) = balance::apply(&plan, set)?;

        let staging = self.staging_dir("split-balance")?;
        let writing = reporter.start("Writing", Unit::Files);
        let written = labels::write(&set, &staging, format, writing.as_ref());
        writing.finish();
        let described = match written {
            Ok(written) => labels::describe(&set, format, written),
            Err(e) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(e.into());
            }
        };

        let seed: toml::Value = i64::try_from(summary.seed).map_or_else(|_| summary.seed.to_string().into(), toml::Value::from);
        let mut lineage = Lineage::new("split-balance", &[&dataset])
            .with_param("seed", seed)
            .with_param("images", i64::from(summary.total_after()));
        match plan.split {
            Resplit::Keep => lineage = lineage.with_param("split", "keep"),
            Resplit::Ratios { train, val, test } => {
                lineage = lineage.with_param("split", format!("{train}/{val}/{test}")).with_param("stratify", plan.stratify);
            }
        }
        if !plan.drop_classes.is_empty() {
            lineage = lineage.with_param("drop_classes", plan.drop_classes.clone());
        }
        if !plan.filter.classes.is_empty() {
            let key =if plan.filter.mode == FilterMode::Keep { "keep_classes" } else { "remove_classes" };
            lineage = lineage.with_param(key, plan.filter.classes.clone());
        }
        if let Some(percent) = plan.subset_percent {
            lineage = lineage.with_param("subset_percent", i64::from(percent));
        }
        if let Some(cap) = plan.class_cap {
            lineage = lineage.with_param("class_cap", i64::from(cap));
        }
        match self.save_described(&staging, output, lineage, described, reporter) {
            Ok(dir) => Ok((dir, summary)),
            Err(e) => {
                let _ = fs::remove_dir_all(&staging);
                Err(e)
            }
        }
    }

    /// What a Convert `plan` would write, from the input's statistics.
    pub fn convert_preview(&self, plan: &ConvertPlan, progress: &dyn Progress) -> Result<ConvertPreview, crate::Error> {
        let (_, dataset) = self.find_dataset(plan.input)?;
        let Some(source) = LabelFormat::parse(dataset.annotation_format()) else {
            let problem = format!("'{}' labels can't be converted; YOLO, COCO and Pascal VOC can.", dataset.annotation_format());
            return Ok(ConvertPreview { problems: vec![problem], ..Default::default() });
        };
        let stats = self.explore_stats(plan.input, false, progress)?;
        Ok(convert::preview(plan, source, &stats))
    }

    /// Converts a dataset to another label format and / or task and saves the
    /// result as `output`.
    pub fn apply_convert(
        &self,
        plan: &ConvertPlan,
        output: &Output,
        reporter: &dyn Reporter,
    ) -> Result<(PathBuf, ConvertSummary), crate::Error> {
        let reading = reporter.start("Reading labels", Unit::Files);
        let read = self.read_labels(plan.input, reading.as_ref());
        reading.finish();
        let (_, dataset, mut set) = read?;
        let source = LabelFormat::parse(dataset.annotation_format()).ok_or("unsupported label format")?;
        let preview = convert::preview(plan, source, &stats::analyze(&set));
        if let Some(problem) = preview.problems.first().or(preview.unchanged.as_ref()) {
            return Err(problem.clone().into());
        }

        let staging = self.staging_dir("convert")?;
        let mut summary = ConvertSummary::default();
        // Known for a label format written here; class folders are scanned instead.
        let mut described = None;
        let mut lineage = Lineage::new("convert", &[&dataset]).with_param("task", plan.task.name());
        let written: Result<(), crate::Error> = if plan.task == TargetTask::Classification {
            let cropping = reporter.start("Cutting out objects", Unit::Files);
            let crops = convert::write_crops(&set, &staging, plan.padding_percent, plan.min_size, cropping.as_ref());
            cropping.finish();
            crops.and_then(|crops| {
                summary.crops = crops;
                summary.images = crops.crops;
                if crops.crops == 0 { Err("no object was large enough to cut out".into()) } else { Ok(()) }
            })
        } else {
            let format = LabelFormat::parse(&plan.format).ok_or("unknown label format")?;
            if plan.task == TargetTask::Detection || format == LabelFormat::Voc {
                summary.outlines_lost = convert::strip_outlines(&mut set);
            }
            summary.images = set.images.len() as u32;
            lineage = lineage.with_param("format", format.name());
            let writing = reporter.start("Writing", Unit::Files);
            let written = labels::write(&set, &staging, format, writing.as_ref());
            writing.finish();
            written.map(|written| described = labels::describe(&set, format, written)).map_err(Into::into)
        };
        if let Err(e) = written {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }

        if plan.task == TargetTask::Classification {
            lineage = lineage
                .with_param("format", convert::FOLDERS_FORMAT)
                .with_param("padding_percent", i64::from(plan.padding_percent))
                .with_param("min_size", i64::from(plan.min_size))
                .with_param("crops", i64::from(summary.crops.crops))
                .with_param("skipped_small", i64::from(summary.crops.skipped_small));
        } else {
            lineage = lineage.with_param("source_format", source.name()).with_param("outlines_to_boxes", i64::from(summary.outlines_lost));
        }
        match self.save_described(&staging, output, lineage, described, reporter) {
            Ok(dir) => Ok((dir, summary)),
            Err(e) => {
                let _ = fs::remove_dir_all(&staging);
                Err(e)
            }
        }
    }

    /// Deletes dataset `id`: its folder with all images and labels, and what
    /// was cached about it. Datasets made from it stay (their history still
    /// names it). Returns its name.
    pub fn delete_dataset(&self, id: u32) -> Result<String, crate::Error> {
        let (dir, dataset) = self.find_dataset(id)?;
        // Only ever a folder directly inside datasets/.
        if dir.parent() != Some(self.datasets_dir().as_path()) {
            return Err(format!("{} isn't in this workspace's datasets folder", dir.display()).into());
        }
        io::retry_busy(&dir, || fs::remove_dir_all(&dir))?;

        let cache = self.path.join(".cache");
        let _ = fs::remove_file(cache.join("stats").join(format!("{id}.json")));
        let _ = fs::remove_file(self.health_cache(id));
        let _ = fs::remove_dir_all(self.thumbs_dir(id));
        Ok(dataset.name().to_string())
    }

    /// Gives the datasets the ids 1, 2, 3… in their current order, so a
    /// deleted one leaves no gap. Cached statistics, Health Check reports and
    /// previews move with their dataset and stay current, and lineage records
    /// follow renumbered parents. Returns the `(old, new)` ids that changed.
    /// Call only while no tool works on the datasets.
    pub fn renumber_datasets(&self) -> Result<Vec<(u32, u32)>, crate::Error> {
        let datasets = self.list_datasets()?;
        let changes: Vec<(u32, u32, String)> = datasets
            .iter()
            .zip(1..)
            .filter(|((_, d), new)| d.id() != *new)
            .map(|((_, d), new)| (d.id(), new, d.name().to_string()))
            .collect();
        if changes.is_empty() {
            return Ok(Vec::new());
        }
        let renumbered = |parent: &ParentRef| changes.iter().find(|(old, _, name)| *old == parent.id && *name == parent.name).map(|c| c.1);

        let cache = self.path.join(".cache");
        // In id order a dataset only moves to an id at or below its own, which
        // the dataset that had it has already left.
        for ((dir, mut dataset), new) in datasets.into_iter().zip(1..) {
            let old = dataset.id();
            let mut lineage = dataset.lineage().cloned();
            let mut parents_changed = false;
            for parent in lineage.iter_mut().flat_map(|l| l.parents.iter_mut()) {
                if let Some(id) = renumbered(parent) {
                    parent.id = id;
                    parents_changed = true;
                }
            }
            if old == new && !parents_changed {
                continue;
            }

            let old_stamp = dataset_stamp(&dir)?;
            dataset.set_id(new);
            dataset.set_lineage(lineage);
            dataset.save_to_dir(&dir)?;
            let new_stamp = dataset_stamp(&dir)?;

            for kind in ["stats", "health"] {
                let from = cache.join(kind).join(format!("{old}.json"));
                let to = cache.join(kind).join(format!("{new}.json"));
                if old != new {
                    let _ = fs::remove_file(&to);
                    if io::rename(&from, &to).is_err() {
                        continue;
                    }
                }
                restamp(&to, &old_stamp, &new_stamp);
            }
            if old != new {
                let (from, to) = (self.thumbs_dir(old), self.thumbs_dir(new));
                if to.exists() {
                    let _ = fs::remove_dir_all(&to);
                }
                if from.exists() {
                    let _ = io::rename(&from, &to);
                }
            }
        }
        Ok(changes.into_iter().map(|(old, new, _)| (old, new)).collect())
    }

    /// The folders of derived data that can be rebuilt at any time: image
    /// previews, Explore statistics and Health Check's memory of decoded
    /// images. Health Check reports and temporary job folders aren't among them.
    fn rebuildable_caches(&self) -> [PathBuf; 3] {
        let cache = self.path.join(".cache");
        [cache.join("thumbs"), cache.join("stats"), cache.join("images")]
    }

    /// Bytes of cached previews and statistics.
    pub fn cache_size(&self) -> u64 {
        fn size(dir: &Path) -> u64 {
            let Ok(entries) = fs::read_dir(dir) else { return 0 };
            entries
                .filter_map(Result::ok)
                .map(|entry| match entry.metadata() {
                    Ok(meta) if meta.is_dir() => size(&entry.path()),
                    Ok(meta) => meta.len(),
                    Err(_) => 0,
                })
                .sum()
        }
        self.rebuildable_caches().iter().map(|dir| size(dir)).sum()
    }

    /// Deletes cached previews and statistics; they are made again when
    /// needed. Returns the bytes freed.
    pub fn clear_cache(&self) -> Result<u64, crate::Error> {
        let freed = self.cache_size();
        for dir in self.rebuildable_caches() {
            if dir.exists() {
                io::retry_busy(&dir, || fs::remove_dir_all(&dir))?;
            }
        }
        Ok(freed)
    }

    /// Where previews of dataset `id`'s images are cached.
    pub fn thumbs_dir(&self, id: u32) -> PathBuf {
        self.path.join(".cache").join("thumbs").join(id.to_string())
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
        self.save_described(staging, output, lineage, None, reporter)
    }

    /// `save_output` for a result whose statistics are known already
    /// (`labels::describe`): the folder isn't read again to count them.
    /// With `None` it is scanned.
    pub fn save_described(
        &self,
        staging: &Path,
        output: &Output,
        lineage: Lineage,
        described: Option<DatasetStats>,
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

        let stats = match described {
            Some(stats) => stats,
            None => {
                let progress = reporter.start("Scanning", Unit::Files);
                let stats = scan_dataset(staging, progress.as_ref());
                progress.finish();
                stats?
            }
        };

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

    /// Downloads a dataset into `datasets/<name>`. Staging happens in the
    /// workspace's `.cache/tmp`.
    pub async fn download(
        &self,
        link: &str,
        opts: &DownloadOptions,
        reporter: Arc<dyn Reporter>,
    ) -> Result<PathBuf, crate::Error> {
        let downloader = Downloader::new(reporter.clone())?;
        let inspected = downloader.inspect(Source::from_link(link)?, opts).await?;
        let result = self.download_inspected(&downloader, &inspected, link, opts, reporter).await;
        // A copy made in the user's Roboflow workspace goes again, however the download ended.
        downloader.cleanup(&inspected, opts).await;
        result
    }

    async fn download_inspected(
        &self,
        downloader: &Downloader,
        inspected: &Inspected,
        link: &str,
        opts: &DownloadOptions,
        reporter: Arc<dyn Reporter>,
    ) -> Result<PathBuf, crate::Error> {
        let Inspected { source, info, name, .. } = inspected;
        // The format actually exported (the project's type may not offer the requested one).
        let opts = &DownloadOptions { format: inspected.format.clone(), ..opts.clone() };

        let final_dir = self.datasets_dir().join(name);
        if final_dir.exists() {
            return Err(AlreadyExists { path: final_dir }.into());
        }

        let tmp_dir = self.tmp_dir();
        tokio::fs::create_dir_all(&tmp_dir).await?;
        let archive = tmp_dir.join(format!("{name}.zip"));
        let extract_dir = tmp_dir.join(name);
        let _ = tokio::fs::remove_dir_all(&extract_dir).await; // leftovers of an earlier failed run

        let result: Result<(), crate::Error> = async {
            downloader.fetch(source, opts, &archive, &extract_dir).await?;
            check_cancel(reporter.is_cancelled())?;
            self.describe_download(&extract_dir, name, link, source, info, opts, reporter.as_ref()).await?;
            check_cancel(reporter.is_cancelled())?;
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

    /// Imports the dataset folder `src` as `datasets/<name>`, copying it or
    /// moving it. A move is undone if the import fails.
    pub fn import_dir(&self, src: &Path, name: &str, mode: ImportMode, reporter: &dyn Reporter) -> Result<PathBuf, crate::Error> {
        if !src.is_dir() {
            return Err(format!("{} isn't a folder", src.display()).into());
        }
        // Canonical paths only for comparing; the record keeps the readable one.
        let src = std::path::absolute(src)?;
        let canonical = src.canonicalize()?;
        let workspace = self.path.canonicalize()?;
        if workspace.starts_with(&canonical) {
            return Err("that folder contains this workspace; pick the dataset folder itself".into());
        }
        if canonical.starts_with(workspace.join("datasets")) {
            return Err("that folder is already in this workspace's datasets folder".into());
        }
        self.check_new_name(name)?;

        let staging = self.staging_dir("import")?;
        let placed = match mode {
            ImportMode::Copy => {
                let progress = reporter.start("Copying", Unit::Files);
                let result = io::copy_dir_blocking(&src, &staging, progress.as_ref());
                progress.finish();
                result
            }
            ImportMode::Move => {
                fs::remove_dir(&staging)?;
                io::move_dir_blocking(&src, &staging, reporter)
            }
        };
        if let Err(e) = placed {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }

        let lineage = Lineage::new("import", &[])
            .with_param("source", src.display().to_string())
            .with_param("mode", mode.name());
        self.save_output(&staging, &Output::New { name: name.to_string() }, lineage, reporter)
            .inspect_err(|_| match mode {
                ImportMode::Copy => {
                    let _ = fs::remove_dir_all(&staging);
                }
                ImportMode::Move => {
                    let _ = io::move_dir_blocking(&staging, &src, reporter);
                }
            })
    }

    /// Imports a dataset zip as `datasets/<name>`. A single top-level folder
    /// inside the archive is unwrapped.
    pub fn import_zip(&self, zip: &Path, name: &str, reporter: &dyn Reporter) -> Result<PathBuf, crate::Error> {
        if !zip.is_file() {
            return Err(format!("{} isn't a file", zip.display()).into());
        }
        if !io::is_zip(zip)? {
            return Err(format!("{} isn't a zip file", zip.display()).into());
        }
        self.check_new_name(name)?;

        let staging = self.staging_dir("import")?;
        let result = (|| {
            let progress = reporter.start("Extracting", Unit::Files);
            let extracted = io::extract_zip_blocking(zip, &staging, progress.as_ref());
            progress.finish();
            extracted?;
            io::unwrap_single_dir(&staging)?;
            check_cancel(reporter.is_cancelled())?;

            let source = std::path::absolute(zip).unwrap_or_else(|_| zip.to_path_buf());
            let lineage = Lineage::new("import", &[]).with_param("source", source.display().to_string());
            self.save_output(&staging, &Output::New { name: name.to_string() }, lineage, reporter)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    fn check_new_name(&self, name: &str) -> Result<(), crate::Error> {
        validate_dataset_name(name)?;
        if self.datasets_dir().join(name).exists() {
            return Err(format!("a dataset folder named '{name}' already exists").into());
        }
        Ok(())
    }

    // Written before the folder moves into datasets/, so it never appears there without a toml.
    async fn describe_download(
        &self,
        dir: &Path,
        name: &str,
        link: &str,
        source: &Source,
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
        // Roboflow knows the exact export format (e.g. "yolov8"); others keep the scanner's guess.
        if matches!(source, Source::Roboflow { .. }) {
            dataset.set_annotation_format(opts.format.clone());
        }
        if !info.dataset_type.is_empty() {
            dataset.set_dataset_type(info.dataset_type.clone());
        }
        dataset.set_license(info.license.clone());
        dataset.set_dataset_link(link.to_string());
        dataset.save_to_dir(dir)?;
        Ok(dataset)
    }
}

/// Changes whenever the dataset's `dataset.toml` does; cache key for derived data.
fn dataset_stamp(dir: &Path) -> Result<String, crate::Error> {
    let meta = fs::metadata(dir.join(DATASET_FILE))?;
    Ok(format!("{}-{:?}", meta.len(), meta.modified().ok()))
}

/// A cache file at `path` made for the `dataset.toml` stamped `old` belongs to
/// it again after `dataset.toml` was rewritten without real changes (stamp `new`).
fn restamp(path: &Path, old: &str, new: &str) {
    let Ok(bytes) = fs::read(path) else { return };
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return };
    if value.get("stamp").and_then(|s| s.as_str()) != Some(old) {
        return;
    }
    value["stamp"] = new.into();
    if let Ok(bytes) = serde_json::to_vec(&value) {
        let _ = fs::write(path, bytes);
    }
}

/// Names of the chosen fixes, for the lineage record.
fn fix_names(fixes: &HealthFixes) -> Vec<String> {
    [
        (fixes.remove_corrupt, "remove_corrupt"),
        (fixes.drop_degenerate, "drop_degenerate"),
        (fixes.clip_out_of_bounds, "clip_out_of_bounds"),
        (fixes.drop_unknown_classes, "drop_unknown_classes"),
        (fixes.remove_leaks, "remove_leaks"),
        (fixes.remove_exact_duplicates, "remove_exact_duplicates"),
        (fixes.remove_near_duplicates, "remove_near_duplicates"),
        (fixes.remove_unlabeled, "remove_unlabeled"),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, name)| name.to_string())
    .collect()
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

    fn yolo_folder(dir: &Path) {
        for (rel, contents) in YOLO {
            let path = dir.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }

    #[test]
    fn imports_folder_by_copy_and_move() {
        let tmp = TempWorkspace::new("import-dir");
        let ws = &tmp.0;
        let src = ws.path().join("outside-src");
        yolo_folder(&src);

        let dir = ws.import_dir(&src, "cats", ImportMode::Copy, &Silent).unwrap();
        assert!(src.join("data.yaml").is_file(), "copy keeps the source");
        let saved = Dataset::load_from_dir(&dir).unwrap();
        assert_eq!((saved.name(), saved.image_count()), ("cats", 1));
        let lineage = saved.lineage().unwrap();
        assert_eq!((lineage.operation.as_str(), lineage.params["mode"].as_str()), ("import", Some("copy")));

        assert!(ws.import_dir(&src, "cats", ImportMode::Copy, &Silent).is_err(), "name taken");
        assert!(ws.import_dir(ws.path(), "whole", ImportMode::Copy, &Silent).is_err(), "contains workspace");
        assert!(ws.import_dir(&dir, "again", ImportMode::Copy, &Silent).is_err(), "already inside");

        ws.import_dir(&src, "cats-moved", ImportMode::Move, &Silent).unwrap();
        assert!(!src.exists(), "move takes the source");
        assert!(ws.datasets_dir().join("cats-moved/train/images/a.jpg").is_file());
        assert_eq!(fs::read_dir(ws.tmp_dir()).unwrap().count(), 0);
    }

    #[test]
    fn imports_zip_and_unwraps_single_folder() {
        use std::io::Write;
        let tmp = TempWorkspace::new("import-zip");
        let ws = &tmp.0;
        let zip_path = ws.path().join("cats.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
        for (rel, contents) in YOLO {
            zip.start_file(format!("cats-export/{rel}"), zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(contents.as_bytes()).unwrap();
        }
        zip.finish().unwrap();

        let dir = ws.import_zip(&zip_path, "cats", &Silent).unwrap();
        assert!(dir.join("data.yaml").is_file(), "top folder unwrapped");
        assert_eq!(Dataset::load_from_dir(&dir).unwrap().image_count(), 1);

        let not_zip = ws.path().join("notes.txt");
        fs::write(&not_zip, "hello").unwrap();
        assert!(ws.import_zip(&not_zip, "notes", &Silent).is_err());
        assert_eq!(fs::read_dir(ws.tmp_dir()).unwrap().count(), 0);
    }

    #[test]
    fn cancelled_import_cleans_up() {
        struct Cancelling;
        impl Reporter for Cancelling {
            fn start(&self, _: &'static str, _: Unit) -> Box<dyn crate::Progress> {
                Box::new(CancelledProgress)
            }
            fn message(&self, _: &str) {}
            fn is_cancelled(&self) -> bool {
                true
            }
        }
        struct CancelledProgress;
        impl crate::Progress for CancelledProgress {
            fn set_total(&self, _: u64) {}
            fn add(&self, _: u64) {}
            fn finish(&self) {}
            fn is_cancelled(&self) -> bool {
                true
            }
        }

        let tmp = TempWorkspace::new("import-cancel");
        let ws = &tmp.0;
        let src = ws.path().join("outside-src");
        yolo_folder(&src);
        let err = ws.import_dir(&src, "cats", ImportMode::Copy, &Cancelling).unwrap_err();
        assert!(err.downcast_ref::<crate::Cancelled>().is_some(), "{err}");
        assert!(!ws.datasets_dir().join("cats").exists());
        assert_eq!(fs::read_dir(ws.tmp_dir()).unwrap().count(), 0);
    }

    #[test]
    fn fingerprint_notices_outside_changes() {
        let tmp = TempWorkspace::new("fingerprint");
        let ws = &tmp.0;
        let before = Workspace::fingerprint(ws.path());
        assert_eq!(before, Workspace::fingerprint(ws.path()), "stable without changes");

        let staging = tmp.staging_with(YOLO);
        let dir = ws.save_output(&staging, &new_output("cats"), Lineage::new("import", &[]), &Silent).unwrap();
        let added = Workspace::fingerprint(ws.path());
        assert_ne!(before, added);

        fs::rename(&dir, ws.datasets_dir().join("dogs")).unwrap();
        let renamed = Workspace::fingerprint(ws.path());
        assert_ne!(added, renamed);

        fs::remove_dir_all(ws.datasets_dir().join("dogs")).unwrap();
        assert_eq!(Workspace::fingerprint(ws.path()), before, "back to the empty state");
    }

    #[test]
    fn explore_stats_are_cached_until_dataset_changes() {
        let tmp = TempWorkspace::new("stats");
        let ws = &tmp.0;
        let staging = tmp.staging_with(&[("data.yaml", "names: ['cat']\n"), ("train/labels/a.txt", "0 0.5 0.5 0.2 0.2\n")]);
        fs::create_dir_all(staging.join("train/images")).unwrap();
        image::RgbImage::new(64, 32).save(staging.join("train/images/a.png")).unwrap();
        ws.save_output(&staging, &new_output("cats"), Lineage::new("import", &[]), &Silent).unwrap();

        let stats = ws.explore_stats(1, false, &Silent).unwrap();
        assert_eq!((stats.image_count, stats.label_count), (1, 1));
        assert_eq!(stats.image_sizes[0].width, 64);

        // A doctored cache entry is served while dataset.toml is unchanged...
        let cache = ws.path().join(".cache/stats/1.json");
        let text = fs::read_to_string(&cache).unwrap().replace("\"image_count\":1", "\"image_count\":999");
        fs::write(&cache, text).unwrap();
        assert_eq!(ws.explore_stats(1, false, &Silent).unwrap().image_count, 999);
        // ...and replaced on refresh.
        assert_eq!(ws.explore_stats(1, true, &Silent).unwrap().image_count, 1);
        assert!(ws.explore_stats(7, false, &Silent).is_err(), "unknown id");
    }

    #[test]
    fn upgrades_models_folder_to_training() {
        let tmp = TempWorkspace::new("layout");
        let ws = &tmp.0;
        ws.save_parameter_set(&ParameterSet::new("kept"), None).unwrap();
        // An older workspace: models/parameters, no training/
        fs::rename(ws.training_dir(), ws.path().join("models")).unwrap();

        let loaded = Workspace::load_from_dir(ws.path()).unwrap();
        assert!(!loaded.path().join("models").exists());
        assert!(loaded.trains_dir().is_dir());
        assert_eq!(loaded.parameter_sets().unwrap().0[0].name, "kept");
    }

    #[test]
    fn saves_edits_renames_and_deletes_parameter_sets() {
        let tmp = TempWorkspace::new("parameters");
        let ws = &tmp.0;
        assert!(ws.parameters_dir().is_dir() && ws.trains_dir().is_dir(), "new workspaces get training/parameters and training/trains");
        assert!(ws.parameter_sets().unwrap().0.is_empty());

        // Add
        let saved = ws.save_parameter_set(&ParameterSet::new("baseline"), None).unwrap();
        assert!(!saved.created.is_empty() && saved.created == saved.updated);
        assert!(ws.parameters_dir().join("baseline.toml").is_file());
        assert!(ws.save_parameter_set(&ParameterSet::new("baseline"), None).is_err(), "names are unique");
        assert!(ws.save_parameter_set(&ParameterSet::new("bad/name"), None).is_err());

        // Edit with rename keeps `created`, removes the old file
        let mut edited = saved.clone();
        edited.name = "longer".into();
        edited.description = Some("more epochs".into());
        let renamed = ws.save_parameter_set(&edited, Some("baseline")).unwrap();
        assert_eq!(renamed.created, saved.created);
        assert!(!ws.parameters_dir().join("baseline.toml").exists());

        // Renaming onto another set is refused
        ws.save_parameter_set(&ParameterSet::new("other"), None).unwrap();
        let mut clash = renamed.clone();
        clash.name = "other".into();
        assert!(ws.save_parameter_set(&clash, Some("longer")).is_err());

        // A broken file is reported, not fatal
        fs::write(ws.parameters_dir().join("broken.toml"), "not = [toml").unwrap();
        let (sets, warnings) = ws.parameter_sets().unwrap();
        assert_eq!(sets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["longer", "other"]);
        assert_eq!(sets[0].description.as_deref(), Some("more epochs"));
        assert_eq!(warnings.len(), 1);

        ws.delete_parameter_set("other").unwrap();
        assert!(ws.delete_parameter_set("other").is_err());
        assert_eq!(ws.parameter_sets().unwrap().0.len(), 1);
    }

    #[test]
    fn saves_and_deletes_class_lists() {
        let tmp = TempWorkspace::new("class-lists");
        let ws = &tmp.0;
        assert!(ws.class_lists().unwrap().is_empty());
        let list = ClassList { name: "vehicles".into(), classes: vec!["car".into(), "truck".into()] };
        ws.save_class_list(&list).unwrap();
        ws.save_class_list(&ClassList { name: "Animals".into(), classes: vec!["cat".into()] }).unwrap();
        let lists = ws.class_lists().unwrap();
        assert_eq!(lists.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Animals", "vehicles"]);
        assert_eq!(lists[1], list);
        assert!(ws.save_class_list(&ClassList { name: "a/b".into(), classes: vec!["x".into()] }).is_err());
        assert!(ws.save_class_list(&ClassList { name: "empty".into(), classes: vec![] }).is_err());
        ws.delete_class_list("vehicles").unwrap();
        assert_eq!(ws.class_lists().unwrap().len(), 1);
    }

    #[test]
    fn merges_yolo_and_coco_into_yolo() {
        use crate::core::classes::{ClassAction, DropMode};

        let tmp = TempWorkspace::new("merge");
        let ws = &tmp.0;
        // YOLO: car, person.
        let yolo = tmp.staging_with(&[
            ("data.yaml", "names: ['car', 'person']\n"),
            ("train/labels/a.txt", "0 0.5 0.5 0.2 0.2\n1 0.2 0.2 0.1 0.1\n"),
        ]);
        fs::create_dir_all(yolo.join("train/images")).unwrap();
        image::RgbImage::new(64, 64).save(yolo.join("train/images/a.png")).unwrap();
        ws.save_output(&yolo, &new_output("street"), Lineage::new("import", &[]), &Silent).unwrap();
        // COCO: Cars, dog (same file name as the YOLO image).
        let coco = tmp.staging_with(&[(
            "valid/_annotations.coco.json",
            r#"{"images": [{"id": 1, "file_name": "a.png", "width": 64, "height": 64}],
                "categories": [{"id": 1, "name": "Cars"}, {"id": 2, "name": "dog"}],
                "annotations": [{"id": 1, "image_id": 1, "category_id": 1, "bbox": [1, 2, 10, 10]},
                                {"id": 2, "image_id": 1, "category_id": 2, "bbox": [20, 20, 10, 10]}]}"#,
        )]);
        image::RgbImage::new(64, 64).save(coco.join("valid/a.png")).unwrap();
        ws.save_output(&coco, &new_output("pets"), Lineage::new("import", &[]), &Silent).unwrap();

        let mut plan = ws.class_merge_suggestion(&[1, 2], &Silent).unwrap();
        assert_eq!(plan.format, "yolo");
        assert_eq!(plan.targets, ["car", "person", "dog"]);
        plan.remove_target("dog");
        plan.set_rule("dog", ClassAction::Drop(DropMode::Boxes));
        let preview = ws.class_merge_preview(&plan, &Silent).unwrap();
        assert!(preview.can_run(), "{preview:?}");
        assert_eq!(preview.missing_labels.iter().map(|m| m.target.as_str()).collect::<Vec<_>>(), ["person"]);

        let (dir, summary) = ws.apply_class_merge(&plan, &new_output("street-pets"), &Silent).unwrap();
        assert_eq!((summary.images, summary.dropped_objects), (2, 1));
        let merged = Dataset::load_from_dir(&dir).unwrap();
        assert_eq!((merged.image_count(), merged.label_count()), (2, 3));
        assert_eq!(merged.class_names(), ["car", "person"]);
        assert_eq!(merged.lineage().unwrap().parents.len(), 2);
        // Both images were a.png: the second got a new name in train/.
        assert!(dir.join("train/images/a.png").is_file() && dir.join("train/images/a_1.png").is_file());
        let label = fs::read_to_string(dir.join("train/labels/a_1.txt")).unwrap();
        assert!(label.starts_with("0 "), "Cars mapped to car = 0: {label}");

        assert!(ws.apply_class_merge(&plan, &Output::Overwrite { id: 1 }, &Silent).is_err(), "no overwrite with 2 inputs");
    }

    #[test]
    fn splits_a_dataset_with_the_workspace_seed() {
        let tmp = TempWorkspace::new("balance");
        let ws = &tmp.0;
        let staging = tmp.staging_with(&[("data.yaml", "names: ['cat']\n")]);
        fs::create_dir_all(staging.join("train/images")).unwrap();
        fs::create_dir_all(staging.join("train/labels")).unwrap();
        for i in 0..10 {
            image::RgbImage::new(32, 32).save(staging.join(format!("train/images/{i}.png"))).unwrap();
            fs::write(staging.join(format!("train/labels/{i}.txt")), "0 0.5 0.5 0.2 0.2\n").unwrap();
        }
        ws.save_output(&staging, &new_output("cats"), Lineage::new("import", &[]), &Silent).unwrap();

        let plan = BalancePlan::new(1);
        let preview = ws.balance_preview(&plan, &Silent).unwrap();
        assert_eq!((preview.seed, preview.images_after), (ws.seed(), [7, 2, 1, 0]));

        let (dir, summary) = ws.apply_balance(&plan, &new_output("cats-split"), &Silent).unwrap();
        assert_eq!(summary, preview);
        let count = |split: &str| fs::read_dir(dir.join(split).join("images")).unwrap().count();
        assert_eq!((count("train"), count("valid"), count("test")), (7, 2, 1));
        let saved = Dataset::load_from_dir(&dir).unwrap();
        let lineage = saved.lineage().unwrap();
        assert_eq!((lineage.operation.as_str(), lineage.params["split"].as_str()), ("split-balance", Some("70/20/10")));
        assert_eq!(lineage.params["seed"].as_integer(), Some(42));

        // Overwriting keeps the dataset's identity.
        let out = ws.apply_balance(&plan, &Output::Overwrite { id: 1 }, &Silent).unwrap().0;
        assert_eq!(out, ws.datasets_dir().join("cats"));
        assert!(out.join("test/images").is_dir());
        assert_eq!(fs::read_dir(ws.tmp_dir()).unwrap().count(), 0);

        let bad = BalancePlan { split: Resplit::Ratios { train: 50, val: 20, test: 10 }, ..plan };
        assert!(ws.apply_balance(&bad, &new_output("bad"), &Silent).is_err());
    }

    #[test]
    fn converts_format_and_task() {
        let tmp = TempWorkspace::new("convert");
        let ws = &tmp.0;
        // One image, a box and an outlined object.
        let staging = tmp.staging_with(&[
            ("data.yaml", "names: ['cat', 'dog']\n"),
            ("train/labels/a.txt", "0 0.5 0.5 0.5 0.5\n1 0.1 0.1 0.4 0.1 0.4 0.4\n"),
        ]);
        fs::create_dir_all(staging.join("train/images")).unwrap();
        image::RgbImage::new(64, 64).save(staging.join("train/images/a.png")).unwrap();
        ws.save_output(&staging, &new_output("pets"), Lineage::new("import", &[]), &Silent).unwrap();
        assert_eq!(Dataset::load_from_dir(&ws.datasets_dir().join("pets")).unwrap().dataset_type(), "instance-segmentation");

        let plan = |task, format: &str| ConvertPlan { input: 1, task, format: format.into(), padding_percent: 0, min_size: 4 };
        assert!(!ws.convert_preview(&plan(TargetTask::Same, "yolo"), &Silent).unwrap().can_run());
        assert!(ws.apply_convert(&plan(TargetTask::Same, "yolo"), &new_output("same"), &Silent).is_err());

        // YOLO to COCO keeps the outline.
        let (dir, summary) = ws.apply_convert(&plan(TargetTask::Same, "coco"), &new_output("pets-coco"), &Silent).unwrap();
        assert_eq!((summary.images, summary.outlines_lost), (1, 0));
        let coco = Dataset::load_from_dir(&dir).unwrap();
        assert_eq!((coco.annotation_format(), coco.dataset_type(), coco.label_count()), ("coco", "instance-segmentation", 2));
        assert_eq!(coco.class_names(), ["cat", "dog"]);
        assert_eq!(coco.lineage().unwrap().params["source_format"].as_str(), Some("yolo"));

        // Outlines to boxes, same format.
        let (dir, summary) = ws.apply_convert(&plan(TargetTask::Detection, "yolo"), &new_output("pets-det"), &Silent).unwrap();
        assert_eq!(summary.outlines_lost, 1);
        assert_eq!(Dataset::load_from_dir(&dir).unwrap().dataset_type(), "object-detection");
        assert!(dir.join("data.yaml").is_file());

        // COCO to VOC, read back from the converted copy.
        let (dir, summary) = ws.apply_convert(&ConvertPlan { input: 2, ..plan(TargetTask::Same, "voc") }, &new_output("pets-voc"), &Silent).unwrap();
        assert_eq!(summary.outlines_lost, 1);
        assert!(dir.join("train/a.xml").is_file());

        // Objects into class folders.
        let (dir, summary) = ws.apply_convert(&plan(TargetTask::Classification, ""), &new_output("pets-cls"), &Silent).unwrap();
        assert_eq!(summary.crops.crops, 2);
        assert!(dir.join("train/cat/a_0.png").is_file() && dir.join("train/dog/a_1.png").is_file());
        let cls = Dataset::load_from_dir(&dir).unwrap();
        assert_eq!((cls.dataset_type(), cls.annotation_format(), cls.image_count()), ("classification", "folders", 2));
        assert_eq!(cls.class_names(), ["cat", "dog"]);
        let again = ws.convert_preview(&ConvertPlan { input: cls.id(), ..plan(TargetTask::Same, "yolo") }, &Silent).unwrap();
        assert!(again.problems[0].contains("can't be converted"));
        assert_eq!(fs::read_dir(ws.tmp_dir()).unwrap().count(), 0);
    }

    #[test]
    fn keeps_seed_and_default_format() {
        let tmp = TempWorkspace::new("options");
        let mut ws = Workspace::load_from_dir(tmp.0.path()).unwrap();
        assert_eq!((ws.seed(), ws.default_format()), (42, None));
        assert!(!fs::read_to_string(ws.path().join("workspace.toml")).unwrap().contains("default_format"));

        ws.set_seed(7);
        ws.set_default_format(Some("COCO")).unwrap();
        ws.set_description("street scenes".into());
        assert!(ws.set_default_format(Some("parquet")).is_err());
        ws.save().unwrap();

        let mut loaded = Workspace::load_from_dir(tmp.0.path()).unwrap();
        assert_eq!((loaded.seed(), loaded.default_format(), loaded.description()), (7, Some(LabelFormat::Coco), "street scenes"));
        assert_eq!(loaded.path(), tmp.0.path());

        // A plan without its own seed, and a class mapping without inputs' say, follow the workspace.
        let staging = tmp.staging_with(YOLO);
        fs::remove_file(staging.join("train/images/a.jpg")).unwrap();
        image::RgbImage::new(16, 16).save(staging.join("train/images/a.png")).unwrap();
        loaded.save_output(&staging, &new_output("cats"), Lineage::new("import", &[]), &Silent).unwrap();
        assert_eq!(loaded.class_merge_suggestion(&[1], &Silent).unwrap().format, "coco");
        assert_eq!(loaded.balance_preview(&BalancePlan::new(1), &Silent).unwrap().seed, 7);

        loaded.set_default_format(None).unwrap();
        assert_eq!(loaded.class_merge_suggestion(&[1], &Silent).unwrap().format, "yolo");
    }

    #[test]
    fn deletes_a_dataset_and_its_caches() {
        let tmp = TempWorkspace::new("delete");
        let ws = &tmp.0;
        for name in ["cats", "dogs"] {
            let staging = tmp.staging_with(YOLO);
            ws.save_output(&staging, &new_output(name), Lineage::new("import", &[]), &Silent).unwrap();
        }
        let cache = ws.path().join(".cache");
        for rel in ["stats/1.json", "health/1.json", "thumbs/1/a.jpg", "stats/2.json"] {
            fs::create_dir_all(cache.join(rel).parent().unwrap()).unwrap();
            fs::write(cache.join(rel), "x").unwrap();
        }

        assert_eq!(ws.delete_dataset(1).unwrap(), "cats");
        assert!(!ws.datasets_dir().join("cats").exists());
        assert!(!cache.join("stats/1.json").exists() && !cache.join("health/1.json").exists() && !cache.join("thumbs/1").exists());
        assert!(ws.datasets_dir().join("dogs").is_dir() && cache.join("stats/2.json").is_file(), "others stay");
        assert_eq!(ws.list_datasets().unwrap().len(), 1);
        assert!(ws.delete_dataset(1).is_err(), "already gone");
    }

    #[test]
    fn renumbers_datasets_without_gaps() {
        let tmp = TempWorkspace::new("renumber");
        let ws = &tmp.0;
        for name in ["a", "b", "c"] {
            let staging = tmp.staging_with(YOLO);
            ws.save_output(&staging, &new_output(name), Lineage::new("import", &[]), &Silent).unwrap();
        }
        let (_, b) = ws.find_dataset(2).unwrap();
        let staging = tmp.staging_with(YOLO);
        ws.save_output(&staging, &new_output("d"), Lineage::new("split", &[&b]), &Silent).unwrap();

        // Caches of "c" (id 3), one of them current.
        let (c_dir, _) = ws.find_dataset(3).unwrap();
        let stamp = dataset_stamp(&c_dir).unwrap();
        let cache = ws.path().join(".cache");
        fs::create_dir_all(cache.join("health")).unwrap();
        fs::write(cache.join("health/3.json"), serde_json::json!({ "stamp": stamp, "x": 1 }).to_string()).unwrap();
        fs::create_dir_all(cache.join("thumbs/3")).unwrap();
        fs::write(cache.join("thumbs/3/a.jpg"), "x").unwrap();

        ws.delete_dataset(1).unwrap();
        assert_eq!(ws.renumber_datasets().unwrap(), [(2, 1), (3, 2), (4, 3)]);
        let names: Vec<(u32, String)> = ws.list_datasets().unwrap().iter().map(|(_, d)| (d.id(), d.name().to_string())).collect();
        assert_eq!(names, [(1, "b".into()), (2, "c".into()), (3, "d".into())]);

        let (c_dir, _) = ws.find_dataset(2).unwrap();
        let health: serde_json::Value = serde_json::from_slice(&fs::read(cache.join("health/2.json")).unwrap()).unwrap();
        assert_eq!(health["stamp"], dataset_stamp(&c_dir).unwrap(), "report stays current");
        assert!(!cache.join("health/3.json").exists() && cache.join("thumbs/2/a.jpg").is_file());

        let (_, d) = ws.find_dataset(3).unwrap();
        assert_eq!(d.lineage().unwrap().parents, [ParentRef { id: 1, name: "b".into() }], "parent follows");
        assert!(ws.renumber_datasets().unwrap().is_empty(), "nothing left to do");
    }

    #[test]
    fn clears_previews_and_statistics_only() {
        let tmp = TempWorkspace::new("cache");
        let ws = &tmp.0;
        let cache = ws.path().join(".cache");
        for (rel, bytes) in [("thumbs/1/a.jpg", 100), ("stats/1.json", 20), ("health/1.json", 7)] {
            fs::create_dir_all(cache.join(rel).parent().unwrap()).unwrap();
            fs::write(cache.join(rel), vec![0u8; bytes]).unwrap();
        }
        assert_eq!(ws.cache_size(), 120);
        assert_eq!(ws.clear_cache().unwrap(), 120);
        assert_eq!(ws.cache_size(), 0);
        assert!(cache.join("health/1.json").is_file() && cache.join("tmp").is_dir(), "reports and job folders stay");
        assert_eq!(ws.clear_cache().unwrap(), 0, "nothing to clear is fine");
    }

    #[test]
    fn download_future_is_send() {
        // Compile-time check: servers can run downloads on their own tasks.
        fn assert_send<T: Send>(_: &T) {}
        let ws = Workspace::new("x".into(), PathBuf::from("."));
        let opts = DownloadOptions { format: "coco".into(), ..Default::default() };
        let future = ws.download("https://example.com", &opts, Arc::new(Silent));
        assert_send(&future);
    }
}
