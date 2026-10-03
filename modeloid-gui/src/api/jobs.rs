//! Long-running jobs (downloads, merges, conversions...). A job is a list of
//! items run in order on a blocking thread of the server's runtime. One job
//! runs at a time unless the system settings allow more ("Jobs at once");
//! even then a tool runs one job at a time, and two jobs never work on the
//! same dataset, so tools don't fight over the same files.
//!
//! Each kind keeps its latest job, so every tool page shows its own. Progress
//! reaches the browser through the one activity long-poll (`api/activity.rs`),
//! not a request per job: browsers allow only 6 open requests per server, and
//! a poll per job starved page loads.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// Which tool a job belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JobKind {
    Download,
    Import,
    AddImages,
    HealthCheck,
    HealthFix,
    Rescan,
    ClassesMerge,
    Split,
    Balance,
    Convert,
    Delete,
    PythonSetup,
    Train,
    Tune,
}

impl JobKind {
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    pub fn label(self) -> &'static str {
        match self {
            JobKind::Download => "Download",
            JobKind::Import => "Import",
            JobKind::AddImages => "Add images",
            JobKind::HealthCheck => "Health Check",
            JobKind::HealthFix => "Health Check fix",
            JobKind::Rescan => "Rescan",
            JobKind::ClassesMerge => "Classes & Merge",
            JobKind::Split => "Split",
            JobKind::Balance => "Balance",
            JobKind::Convert => "Convert",
            JobKind::Delete => "Delete",
            JobKind::PythonSetup => "Python setup",
            JobKind::Train => "Training",
            JobKind::Tune => "Tuning",
        }
    }

    /// Where the job is started and what it does, for the history:
    /// (sidebar group, page, process), e.g. ("Dataset Manager", "Datasets", "Download").
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    pub fn history_path(self) -> (&'static str, &'static str, &'static str) {
        const DATASETS: (&str, &str) = ("Dataset Manager", "Datasets");
        let ((category, section), process) = match self {
            JobKind::Download => (DATASETS, "Download"),
            JobKind::Import => (DATASETS, "Import"),
            JobKind::AddImages => (DATASETS, "Add images"),
            JobKind::Rescan => (DATASETS, "Rescan"),
            JobKind::Delete => (DATASETS, "Remove datasets"),
            JobKind::HealthCheck => (("Dataset Manager", "Health Check"), "Check"),
            JobKind::HealthFix => (("Dataset Manager", "Health Check"), "Fix problems"),
            JobKind::ClassesMerge => (("Dataset Manager", "Classes & Merge"), "Merge"),
            JobKind::Split => (("Dataset Manager", "Split & Balance"), "Split"),
            JobKind::Balance => (("Dataset Manager", "Split & Balance"), "Balance"),
            JobKind::Convert => (("Dataset Manager", "Convert"), "Convert"),
            JobKind::PythonSetup => (("Settings", "Python environment"), "Install libraries"),
            JobKind::Train => (("Training", "Train Model"), "Train"),
            JobKind::Tune => (("Training", "Tune Hyperparameters"), "Tune"),
        };
        (category, section, process)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ItemStatus {
    Queued,
    Running,
    Done,
    Failed,
    /// Didn't run to the end, but not because something broke: the user has
    /// a choice to make (see `JobItem::action`).
    Warning,
    /// Nothing to do, e.g. the dataset is already there.
    Skipped,
    /// Stopped by the user, or skipped because the job was cancelled.
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ProgressUnit {
    Bytes,
    Files,
    Steps,
}

/// The step an item is on right now (e.g. "Downloading", "Extracting").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    pub label: String,
    pub unit: ProgressUnit,
    pub done: u64,
    pub total: Option<u64>,
}

/// Something the user can do about a failed item; the tool's page offers it as a button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemAction {
    /// A Roboflow project without a dataset version: copy it into the user's
    /// Roboflow workspace, generate a version, download it, delete the copy.
    RoboflowFork,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobItem {
    /// What this item works on, e.g. a link or a dataset name.
    pub label: String,
    pub status: ItemStatus,
    pub stage: Option<Stage>,
    /// Latest note from the core, e.g. "Using latest version: 3".
    pub message: Option<String>,
    /// Result to show when done, e.g. the folder a dataset was saved to.
    pub output: Option<String>,
    pub error: Option<String>,
    /// Offered with the error, when there is a way on from it.
    pub action: Option<ItemAction>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub kind: JobKind,
    /// Increases on every change, across jobs. Pass it back as `since`.
    pub version: u64,
    pub items: Vec<JobItem>,
    pub finished: bool,
    /// The user asked to stop; the running item stops at its next check.
    pub cancel_requested: bool,
}

/// Asks the running `kind` job to stop. The current item stops at its next
/// check (and cleans up), queued items are skipped.
#[post("/api/jobs/cancel")]
pub async fn cancel_job(kind: JobKind) -> Result<(), ServerFnError> {
    backend::cancel(kind).map_err(ServerFnError::new)
}

#[cfg(feature = "server")]
pub use backend::{busy, latest_all, renumber_when_idle, rerun_item, running, start, ItemCtx, Touches};

#[cfg(feature = "server")]
mod backend {
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::future::Future;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
    use std::sync::{Arc, LazyLock, Mutex};

    use modeloid::{Progress, Reporter, Unit};
    use tokio::runtime::Handle;

    use super::{ItemAction, ItemStatus, Job, JobItem, JobKind, ProgressUnit, Stage};
    use crate::api::history::{HistoryEntry, HistoryItem, Outcome};

    static VERSION: AtomicU64 = AtomicU64::new(0);
    /// Latest job of each kind.
    static JOBS: LazyLock<Mutex<HashMap<JobKind, Arc<Shared>>>> = LazyLock::new(Default::default);

    fn next_version() -> u64 {
        VERSION.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The datasets a job reads or replaces. Two running jobs never touch the same one.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Touches {
        /// Only adds datasets (downloads, imports).
        Nothing,
        /// These dataset ids.
        Datasets(Vec<u32>),
        /// Every dataset (rescan).
        All,
    }

    impl Touches {
        fn overlaps(&self, other: &Touches) -> bool {
            match (self, other) {
                (Touches::Nothing, _) | (_, Touches::Nothing) => false,
                (Touches::All, _) | (_, Touches::All) => true,
                (Touches::Datasets(a), Touches::Datasets(b)) => a.iter().any(|id| b.contains(id)),
            }
        }
    }

    struct Shared {
        job: Mutex<Job>,
        touches: Touches,
        /// The workspace open when it started: its history gets the job.
        workspace: Option<PathBuf>,
        /// When the job (or its re-run item) started, Unix seconds.
        started: AtomicI64,
        /// Checked by the core through `Reporter`/`Progress::is_cancelled`.
        cancelled: AtomicBool,
    }

    impl Shared {
        fn update(&self, f: impl FnOnce(&mut Job)) {
            {
                let mut job = self.job.lock().unwrap();
                f(&mut job);
                job.version = next_version();
            }
            crate::api::activity::changed();
        }

        fn item(&self, index: usize, f: impl FnOnce(&mut JobItem)) {
            self.update(|job| f(&mut job.items[index]));
        }

        fn snapshot(&self) -> Job {
            self.job.lock().unwrap().clone()
        }

        fn finished(&self) -> bool {
            self.job.lock().unwrap().finished
        }

        fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::Relaxed)
        }
    }

    pub fn cancel(kind: JobKind) -> Result<(), String> {
        let shared = latest(kind).filter(|s| !s.finished()).ok_or("Nothing is running.")?;
        shared.cancelled.store(true, Ordering::Relaxed);
        shared.update(|job| job.cancel_requested = true);
        Ok(())
    }

    /// The latest job of every kind that has run.
    pub fn latest_all() -> Vec<Job> {
        JOBS.lock().unwrap().values().map(|s| s.snapshot()).collect()
    }

    /// The jobs that haven't finished yet.
    pub fn running() -> Vec<Job> {
        JOBS.lock().unwrap().values().filter(|s| !s.finished()).map(|s| s.snapshot()).collect()
    }

    /// Why dataset `id` can't be changed right now: the running job working on it.
    pub fn busy(id: u32) -> Result<(), String> {
        let jobs = JOBS.lock().unwrap();
        let touches = Touches::Datasets(vec![id]);
        match jobs.values().find(|s| !s.finished() && s.touches.overlaps(&touches)) {
            Some(other) => Err(format!("The {} job is working on this dataset. Wait for it to finish.", other.snapshot().kind.label())),
            None => Ok(()),
        }
    }

    /// Whether a `kind` job touching `touches` may start now, given the
    /// latest jobs `jobs`; the reason when it may not.
    fn admit(jobs: &HashMap<JobKind, Arc<Shared>>, kind: JobKind, touches: &Touches) -> Result<(), String> {
        let running: Vec<&Arc<Shared>> = jobs.values().filter(|s| !s.finished()).collect();
        let label = |shared: &Shared| shared.snapshot().kind.label();
        if let Some(same) = running.iter().find(|s| s.snapshot().kind == kind) {
            return Err(format!("A {} job is already running. Wait for it to finish.", label(same)));
        }
        if let Some(other) = running.iter().find(|s| s.touches.overlaps(touches)) {
            return Err(format!("The {} job is working on the same dataset. Wait for it to finish.", label(other)));
        }
        let limit = modeloid::core::limits::max_jobs();
        match running.as_slice() {
            [] => Ok(()),
            [only] if limit == 1 => Err(format!("A {} job is already running. Wait for it to finish.", label(only))),
            running if running.len() >= limit => {
                Err(format!("{limit} jobs are already running, the most the system settings allow. Wait for one to finish."))
            }
            _ => Ok(()),
        }
    }

    fn latest(kind: JobKind) -> Option<Arc<Shared>> {
        JOBS.lock().unwrap().get(&kind).cloned()
    }

    /// What an item's work function gets: the server's runtime for the core's
    /// async functions and a reporter that feeds progress into this item.
    pub struct ItemCtx<'a> {
        job: &'a Arc<Shared>,
        index: usize,
        runtime: &'a Handle,
        /// How an item that returns an error ends, when not as `Failed`.
        ends_as: Cell<Option<ItemStatus>>,
    }

    impl ItemCtx<'_> {
        pub fn index(&self) -> usize {
            self.index
        }

        pub fn reporter(&self) -> Arc<dyn Reporter> {
            Arc::new(JobReporter { job: self.job.clone(), index: self.index })
        }

        /// Offers `action` with this item's error (call before returning it).
        /// The item then ends as a warning, not as failed.
        pub fn offer(&self, action: ItemAction) {
            self.job.item(self.index, |item| item.action = Some(action));
            self.ends_as.set(Some(ItemStatus::Warning));
        }

        /// This item's error means there was nothing to do (call before
        /// returning it): it ends as skipped, not as failed.
        pub fn skip(&self) {
            self.ends_as.set(Some(ItemStatus::Skipped));
        }

        /// Runs a future to completion on the server's runtime, blocking this
        /// job's thread (never an async worker) until it's done.
        pub fn block_on<F: Future>(&self, future: F) -> F::Output {
            self.runtime.block_on(future)
        }
    }

    /// Starts a `kind` job with one item per label and returns its first state.
    /// `touches` names the datasets it reads or replaces. `work` runs once
    /// per item, in order, on a blocking thread (so it can do slow file work
    /// directly); it returns the item's output (shown when done) or an error
    /// message. Call from inside the server's runtime.
    pub fn start<F>(kind: JobKind, labels: Vec<String>, touches: Touches, mut work: F) -> Result<Job, String>
    where
        F: FnMut(&ItemCtx) -> Result<Option<String>, String> + Send + 'static,
    {
        // Held while checking and adding, so two jobs can't slip in at once.
        let mut jobs = JOBS.lock().unwrap();
        admit(&jobs, kind, &touches)?;
        let shared = Arc::new(Shared {
            touches,
            workspace: crate::api::workspace::current_dir(),
            started: AtomicI64::new(modeloid::core::history::now()),
            job: Mutex::new(Job {
                kind,
                version: next_version(),
                items: labels
                    .into_iter()
                    .map(|label| JobItem {
                        label,
                        status: ItemStatus::Queued,
                        stage: None,
                        message: None,
                        output: None,
                        error: None,
                        action: None,
                    })
                    .collect(),
                finished: false,
                cancel_requested: false,
            }),
            cancelled: AtomicBool::new(false),
        });

        let runtime = Handle::try_current().map_err(|e| format!("Couldn't start the job: {e}"))?;
        let job = shared.clone();
        runtime.clone().spawn_blocking(move || run(&job, &runtime, &mut work));

        jobs.insert(kind, shared.clone());
        drop(jobs);
        Ok(shared.snapshot())
    }

    /// Runs item `index` of the finished latest `kind` job again, with
    /// `work`, inside that job: the other items stay as they are, so the
    /// page keeps its whole list. `label` must be the item's label (the page
    /// may be looking at an older job). Returns the job's new state.
    pub fn rerun_item<F>(kind: JobKind, index: usize, label: &str, work: F) -> Result<Job, String>
    where
        F: FnOnce(&ItemCtx) -> Result<Option<String>, String> + Send + 'static,
    {
        // Held while checking and marking, so two jobs can't start at once.
        let jobs = JOBS.lock().unwrap();
        let shared = jobs.get(&kind).cloned().ok_or("That list is gone. Start again.")?;
        admit(&jobs, kind, &shared.touches)?;
        if shared.snapshot().items.get(index).is_none_or(|item| item.label != label) {
            return Err("That list has changed. Start again.".into());
        }
        let runtime = Handle::try_current().map_err(|e| format!("Couldn't start the job: {e}"))?;

        shared.cancelled.store(false, Ordering::Relaxed);
        shared.started.store(modeloid::core::history::now(), Ordering::Relaxed);
        shared.update(|job| {
            job.finished = false;
            job.cancel_requested = false;
            let item = &mut job.items[index];
            *item = JobItem { label: std::mem::take(&mut item.label), status: ItemStatus::Queued, stage: None, message: None, output: None, error: None, action: None };
        });
        drop(jobs);

        let job = shared.clone();
        runtime.clone().spawn_blocking(move || {
            let mut work = Some(work);
            run_item(&job, &runtime, index, &mut |ctx| work.take().map_or(Err("already ran".into()), |work| work(ctx)));
            finish(&job, Some(index));
        });
        Ok(shared.snapshot())
    }

    fn run(job: &Arc<Shared>, runtime: &Handle, work: &mut dyn FnMut(&ItemCtx) -> Result<Option<String>, String>) {
        let count = job.snapshot().items.len();
        for index in 0..count {
            run_item(job, runtime, index, work);
        }
        finish(job, None);
    }

    /// Marks `job` finished and adds it to its workspace's history (only item
    /// `only` when that one was run again). When it was the last one running,
    /// the datasets are renumbered first (a delete or a merge may have left a
    /// gap), while the job list is held so nothing starts on the old ids meanwhile.
    fn finish(job: &Arc<Shared>, only: Option<usize>) {
        let jobs = JOBS.lock().unwrap();
        if jobs.values().all(|s| Arc::ptr_eq(s, job) || s.finished()) {
            crate::api::datasets::renumber();
        }
        if let Some(dir) = &job.workspace {
            crate::api::history::record_in(dir, &history_entry(&job.snapshot(), job.started.load(Ordering::Relaxed), only));
        }
        job.update(|j| j.finished = true);
    }

    /// What the history keeps of a job that just ended.
    fn history_entry(job: &Job, started: i64, only: Option<usize>) -> HistoryEntry {
        let (category, section, process) = job.kind.history_path();
        let items = job
            .items
            .iter()
            .enumerate()
            .filter(|(index, _)| only.is_none_or(|only| only == *index))
            .map(|(_, item)| HistoryItem {
                label: item.label.clone(),
                outcome: match item.status {
                    ItemStatus::Done => Outcome::Done,
                    ItemStatus::Failed => Outcome::Failed,
                    ItemStatus::Warning => Outcome::Warning,
                    ItemStatus::Skipped => Outcome::Skipped,
                    ItemStatus::Cancelled | ItemStatus::Queued | ItemStatus::Running => Outcome::Cancelled,
                },
                detail: item.error.clone().or_else(|| item.output.clone()),
            })
            .collect();
        HistoryEntry {
            started,
            finished: modeloid::core::history::now(),
            category: category.into(),
            section: section.into(),
            process: process.into(),
            items,
        }
    }

    /// Renumbers the datasets (`Workspace::renumber_datasets`) unless a job is
    /// running; the next job to finish does it then.
    pub fn renumber_when_idle() {
        let jobs = JOBS.lock().unwrap();
        if jobs.values().all(|s| s.finished()) {
            crate::api::datasets::renumber();
        }
    }

    fn run_item(job: &Arc<Shared>, runtime: &Handle, index: usize, work: &mut dyn FnMut(&ItemCtx) -> Result<Option<String>, String>) {
        if job.is_cancelled() {
            job.item(index, |item| item.status = ItemStatus::Cancelled);
            return;
        }
        job.item(index, |item| item.status = ItemStatus::Running);
        let ctx = ItemCtx { job, index, runtime, ends_as: Cell::new(None) };
        let result = work(&ctx);
        let ends_as = ctx.ends_as.get();
        let cancelled = job.is_cancelled();
        job.item(index, |item| {
            item.stage = None;
            match result {
                Ok(output) => {
                    item.status = ItemStatus::Done;
                    item.output = output;
                }
                // Whatever error cancelling caused, it's a cancel to the user.
                Err(_) if cancelled => item.status = ItemStatus::Cancelled,
                Err(e) => {
                    item.status = ends_as.unwrap_or(ItemStatus::Failed);
                    item.error = Some(e);
                }
            }
        });
    }

    /// Feeds core progress into one job item.
    struct JobReporter {
        job: Arc<Shared>,
        index: usize,
    }

    impl Reporter for JobReporter {
        fn start(&self, label: &'static str, unit: Unit) -> Box<dyn Progress> {
            let unit = match unit {
                Unit::Bytes => ProgressUnit::Bytes,
                Unit::Files => ProgressUnit::Files,
                Unit::Steps => ProgressUnit::Steps,
            };
            self.job.item(self.index, |item| {
                item.stage = Some(Stage { label: label.to_string(), unit, done: 0, total: None });
            });
            Box::new(JobProgress { job: self.job.clone(), index: self.index })
        }

        fn message(&self, msg: &str) {
            self.job.item(self.index, |item| item.message = Some(msg.to_string()));
        }

        fn is_cancelled(&self) -> bool {
            self.job.is_cancelled()
        }
    }

    struct JobProgress {
        job: Arc<Shared>,
        index: usize,
    }

    impl JobProgress {
        fn stage(&self, f: impl FnOnce(&mut Stage)) {
            self.job.item(self.index, |item| {
                if let Some(stage) = item.stage.as_mut() {
                    f(stage);
                }
            });
        }
    }

    impl Progress for JobProgress {
        fn set_total(&self, total: u64) {
            self.stage(|s| s.total = Some(total));
        }

        fn add(&self, n: u64) {
            self.stage(|s| s.done += n);
        }

        fn finish(&self) {
            self.stage(|s| {
                if let Some(total) = s.total {
                    s.done = total;
                }
            });
        }

        fn is_cancelled(&self) -> bool {
            self.job.is_cancelled()
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;

    use super::backend::cancel;

    /// Waits for the latest `kind` job to finish.
    async fn finished(kind: JobKind) -> Job {
        loop {
            if let Some(job) = latest_all().into_iter().find(|j| j.kind == kind && j.finished) {
                return job;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    // One test: jobs are global and only one runs at a time.
    #[tokio::test(flavor = "multi_thread")]
    async fn runs_items_in_order_and_reports() {
        let labels = vec!["ok".to_string(), "fails".to_string(), "warns".to_string(), "skips".to_string()];
        let started = start(JobKind::Download, labels, Touches::Nothing, |ctx| {
            let reporter = ctx.reporter();
            let n = ctx.block_on(async {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                ctx.index()
            });
            reporter.message(&format!("item {n}"));
            match n {
                0 => return Ok(Some("out".into())),
                2 => ctx.offer(ItemAction::RoboflowFork),
                3 => ctx.skip(),
                _ => {}
            }
            Err("boom".into())
        })
        .unwrap();
        assert!(!started.finished);
        assert_eq!(running().iter().map(|j| j.kind).collect::<Vec<_>>(), [JobKind::Download], "the status bar sees it");

        // A second job can't start while this one runs: not of the same tool, nor of another.
        assert!(start(JobKind::Download, vec!["x".into()], Touches::Nothing, |_| Ok(None)).is_err());
        assert!(start(JobKind::Convert, vec!["x".into()], Touches::Nothing, |_| Ok(None)).is_err());

        let job = finished(JobKind::Download).await;
        assert_eq!(job.items[0].status, ItemStatus::Done);
        assert_eq!(job.items[0].output.as_deref(), Some("out"));
        assert_eq!(job.items[0].message.as_deref(), Some("item 0"));
        assert_eq!(job.items[1].status, ItemStatus::Failed);
        assert_eq!(job.items[1].error.as_deref(), Some("boom"));
        assert_eq!((job.items[2].status, job.items[2].action), (ItemStatus::Warning, Some(ItemAction::RoboflowFork)));
        assert_eq!((job.items[3].status, job.items[3].error.as_deref()), (ItemStatus::Skipped, Some("boom")));
        assert_eq!((job.items[0].action, job.items[1].action), (None, None));

        // One item again, inside the same job: the others keep their state.
        assert!(rerun_item(JobKind::Download, 2, "not its label", |_| Ok(None)).is_err());
        let again = rerun_item(JobKind::Download, 2, "warns", |ctx| Ok(Some(format!("fixed {}", ctx.index())))).unwrap();
        assert!(!again.finished);
        assert_eq!((again.items[2].status, again.items[2].action, again.items[2].error.as_deref()), (ItemStatus::Queued, None, None));
        let job = finished(JobKind::Download).await;
        assert_eq!((job.items[2].status, job.items[2].output.as_deref()), (ItemStatus::Done, Some("fixed 2")));
        assert_eq!((job.items[0].status, job.items[1].status, job.items[3].status), (ItemStatus::Done, ItemStatus::Failed, ItemStatus::Skipped));
        assert_eq!(job.items[0].output.as_deref(), Some("out"));

        // Cancelling stops the running item (via the reporter) and skips the rest.
        let labels = vec!["slow".to_string(), "skipped".to_string()];
        start(JobKind::Import, labels, Touches::Nothing, |ctx| {
            let reporter = ctx.reporter();
            while !reporter.is_cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err("stopped".into())
        })
        .unwrap();
        cancel(JobKind::Import).unwrap();
        let job = finished(JobKind::Import).await;
        assert!(job.cancel_requested);
        assert_eq!(job.items[0].status, ItemStatus::Cancelled);
        assert_eq!(job.items[0].error, None);
        assert_eq!(job.items[1].status, ItemStatus::Cancelled);
        assert!(cancel(JobKind::Import).is_err(), "nothing left to cancel");

        // "Jobs at once" above 1: other tools may start, unless they share a dataset or the limit is reached.
        let waits = |ctx: &ItemCtx| {
            let reporter = ctx.reporter();
            while !reporter.is_cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err("stopped".to_string())
        };
        modeloid::core::limits::apply(modeloid::SystemPrefs { max_jobs: 2, ..Default::default() });
        start(JobKind::HealthCheck, vec!["a".into()], Touches::Datasets(vec![1, 2]), waits).unwrap();
        let same_dataset = start(JobKind::Convert, vec!["b".into()], Touches::Datasets(vec![2]), waits).unwrap_err();
        assert!(same_dataset.contains("same dataset"), "{same_dataset}");
        assert!(start(JobKind::Rescan, vec!["all".into()], Touches::All, waits).unwrap_err().contains("same dataset"));
        assert!(start(JobKind::HealthCheck, vec!["c".into()], Touches::Datasets(vec![9]), waits).unwrap_err().contains("already running"));
        start(JobKind::Convert, vec!["d".into()], Touches::Datasets(vec![3]), waits).unwrap();
        assert_eq!(running().len(), 2);
        let full = start(JobKind::Download, vec!["e".into()], Touches::Nothing, waits).unwrap_err();
        assert!(full.contains("2 jobs are already running"), "{full}");
        cancel(JobKind::HealthCheck).unwrap();
        cancel(JobKind::Convert).unwrap();
        finished(JobKind::HealthCheck).await;
        finished(JobKind::Convert).await;
        modeloid::core::limits::apply(modeloid::SystemPrefs::default());
    }
}
