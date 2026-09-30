//! Long-running jobs (downloads, merges, conversions...). A job is a list of
//! items run in order on a blocking thread of the server's runtime. Only one job runs at a time,
//! across all kinds, so tools don't fight over the same files.
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
    HealthCheck,
    HealthFix,
    Rescan,
    ClassesMerge,
    Split,
    Balance,
    Convert,
}

impl JobKind {
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    pub fn label(self) -> &'static str {
        match self {
            JobKind::Download => "Download",
            JobKind::Import => "Import",
            JobKind::HealthCheck => "Health Check",
            JobKind::HealthFix => "Health Check fix",
            JobKind::Rescan => "Rescan",
            JobKind::ClassesMerge => "Classes & Merge",
            JobKind::Split => "Split",
            JobKind::Balance => "Balance",
            JobKind::Convert => "Convert",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ItemStatus {
    Queued,
    Running,
    Done,
    Failed,
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
pub use backend::{latest_all, running, start};

#[cfg(feature = "server")]
mod backend {
    use std::collections::HashMap;
    use std::future::Future;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, LazyLock, Mutex};

    use modeloid::{Progress, Reporter, Unit};
    use tokio::runtime::Handle;

    use super::{ItemStatus, Job, JobItem, JobKind, ProgressUnit, Stage};

    static VERSION: AtomicU64 = AtomicU64::new(0);
    /// Latest job of each kind.
    static JOBS: LazyLock<Mutex<HashMap<JobKind, Arc<Shared>>>> = LazyLock::new(Default::default);

    fn next_version() -> u64 {
        VERSION.fetch_add(1, Ordering::SeqCst) + 1
    }

    struct Shared {
        job: Mutex<Job>,
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

    /// The job that hasn't finished yet, if any (only one runs at a time).
    pub fn running() -> Option<Job> {
        JOBS.lock().unwrap().values().find(|s| !s.finished()).map(|s| s.snapshot())
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
    }

    impl ItemCtx<'_> {
        pub fn index(&self) -> usize {
            self.index
        }

        pub fn reporter(&self) -> Arc<dyn Reporter> {
            Arc::new(JobReporter { job: self.job.clone(), index: self.index })
        }

        /// Runs a future to completion on the server's runtime, blocking this
        /// job's thread (never an async worker) until it's done.
        pub fn block_on<F: Future>(&self, future: F) -> F::Output {
            self.runtime.block_on(future)
        }
    }

    /// Starts a `kind` job with one item per label and returns its first state.
    /// `work` runs once per item, in order, on a blocking thread (so it can do
    /// slow file work directly); it returns the item's output (shown when done)
    /// or an error message. Call from inside the server's runtime.
    pub fn start<F>(kind: JobKind, labels: Vec<String>, mut work: F) -> Result<Job, String>
    where
        F: FnMut(&ItemCtx) -> Result<Option<String>, String> + Send + 'static,
    {
        let mut jobs = JOBS.lock().unwrap();
        if let Some(running) = jobs.values().find(|s| !s.finished()) {
            let label = running.snapshot().kind.label();
            return Err(format!("A {label} job is already running. Wait for it to finish."));
        }
        let shared = Arc::new(Shared {
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

    fn run(job: &Arc<Shared>, runtime: &Handle, work: &mut dyn FnMut(&ItemCtx) -> Result<Option<String>, String>) {
        let count = job.snapshot().items.len();
        for index in 0..count {
            if job.is_cancelled() {
                job.item(index, |item| item.status = ItemStatus::Cancelled);
                continue;
            }
            job.item(index, |item| item.status = ItemStatus::Running);
            let result = work(&ItemCtx { job, index, runtime });
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
                        item.status = ItemStatus::Failed;
                        item.error = Some(e);
                    }
                }
            });
        }
        job.update(|j| j.finished = true);
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
        let labels = vec!["ok".to_string(), "fails".to_string()];
        let started = start(JobKind::Download, labels, |ctx| {
            let reporter = ctx.reporter();
            let n = ctx.block_on(async {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                ctx.index()
            });
            reporter.message(&format!("item {n}"));
            if n == 0 { Ok(Some("out".into())) } else { Err("boom".into()) }
        })
        .unwrap();
        assert!(!started.finished);
        assert_eq!(running().map(|j| j.kind), Some(JobKind::Download), "the status bar sees it");

        // A second job can't start while this one runs.
        assert!(start(JobKind::Download, vec!["x".into()], |_| Ok(None)).is_err());

        let job = finished(JobKind::Download).await;
        assert_eq!(job.items[0].status, ItemStatus::Done);
        assert_eq!(job.items[0].output.as_deref(), Some("out"));
        assert_eq!(job.items[0].message.as_deref(), Some("item 0"));
        assert_eq!(job.items[1].status, ItemStatus::Failed);
        assert_eq!(job.items[1].error.as_deref(), Some("boom"));

        // Cancelling stops the running item (via the reporter) and skips the rest.
        let labels = vec!["slow".to_string(), "skipped".to_string()];
        start(JobKind::Import, labels, |ctx| {
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
    }
}
