//! Long-running jobs (downloads, merges, conversions...). A job is a list of
//! items run in order on a blocking thread of the server's runtime. Only one job runs at a time,
//! across all kinds, so tools don't fight over the same files.
//!
//! Each kind keeps its latest job, so every tool page shows its own. Pages
//! long-poll `job_status(kind, since)` for progress.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// Which tool a job belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JobKind {
    Download,
}

impl JobKind {
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    pub fn label(self) -> &'static str {
        match self {
            JobKind::Download => "Download",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ItemStatus {
    Queued,
    Running,
    Done,
    Failed,
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
}

/// Returns the latest `kind` job once its `version` is newer than `since`,
/// waiting up to ~20s for a change. `None` if no such job has run yet.
#[post("/api/jobs/status")]
pub async fn job_status(kind: JobKind, since: u64) -> Result<Option<Job>, ServerFnError> {
    Ok(backend::status(kind, since).await)
}

#[cfg(feature = "server")]
pub use backend::start;

#[cfg(feature = "server")]
mod backend {
    use std::collections::HashMap;
    use std::future::Future;
    use std::pin::pin;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, LazyLock, Mutex};
    use std::time::Duration;

    use modeloid::{Progress, Reporter, Unit};
    use tokio::runtime::Handle;
    use tokio::sync::Notify;
    use tokio::time::{sleep_until, timeout_at, Instant};

    use super::{ItemStatus, Job, JobItem, JobKind, ProgressUnit, Stage};

    const POLL_TIMEOUT: Duration = Duration::from_secs(20);
    /// Minimum time a status request takes, so progress updates don't flood requests.
    const POLL_MIN: Duration = Duration::from_millis(300);

    static VERSION: AtomicU64 = AtomicU64::new(0);
    /// Latest job of each kind.
    static JOBS: LazyLock<Mutex<HashMap<JobKind, Arc<Shared>>>> = LazyLock::new(Default::default);
    static JOB_STARTED: Notify = Notify::const_new();

    fn next_version() -> u64 {
        VERSION.fetch_add(1, Ordering::SeqCst) + 1
    }

    struct Shared {
        job: Mutex<Job>,
        changed: Notify,
    }

    impl Shared {
        fn update(&self, f: impl FnOnce(&mut Job)) {
            {
                let mut job = self.job.lock().unwrap();
                f(&mut job);
                job.version = next_version();
            }
            self.changed.notify_waiters();
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
            }),
            changed: Notify::new(),
        });

        let runtime = Handle::try_current().map_err(|e| format!("Couldn't start the job: {e}"))?;
        let job = shared.clone();
        runtime.clone().spawn_blocking(move || run(&job, &runtime, &mut work));

        jobs.insert(kind, shared.clone());
        drop(jobs);
        JOB_STARTED.notify_waiters();
        Ok(shared.snapshot())
    }

    fn run(job: &Arc<Shared>, runtime: &Handle, work: &mut dyn FnMut(&ItemCtx) -> Result<Option<String>, String>) {
        let count = job.snapshot().items.len();
        for index in 0..count {
            job.item(index, |item| item.status = ItemStatus::Running);
            let result = work(&ItemCtx { job, index, runtime });
            job.item(index, |item| {
                item.stage = None;
                match result {
                    Ok(output) => {
                        item.status = ItemStatus::Done;
                        item.output = output;
                    }
                    Err(e) => {
                        item.status = ItemStatus::Failed;
                        item.error = Some(e);
                    }
                }
            });
        }
        job.update(|j| j.finished = true);
    }

    pub async fn status(kind: JobKind, since: u64) -> Option<Job> {
        let started = Instant::now();
        let deadline = started + POLL_TIMEOUT;

        let shared = loop {
            let started_signal = pin!(JOB_STARTED.notified());
            if let Some(shared) = latest(kind) {
                break shared;
            }
            if timeout_at(deadline, started_signal).await.is_err() {
                return None;
            }
        };

        loop {
            let mut changed = pin!(shared.changed.notified());
            changed.as_mut().enable();
            let started_signal = pin!(JOB_STARTED.notified());
            // A newer job of this kind replaced the one we're watching.
            let shared = match latest(kind) {
                Some(newer) if !Arc::ptr_eq(&newer, &shared) => newer,
                _ => shared.clone(),
            };
            let job = shared.snapshot();
            if job.version > since {
                sleep_until(started + POLL_MIN).await;
                return Some(shared.snapshot());
            }
            tokio::select! {
                _ = changed => {}
                _ = started_signal => {}
                _ = sleep_until(deadline) => return Some(job),
            }
        }
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
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;

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

        // A second job can't start while this one runs.
        assert!(start(JobKind::Download, vec!["x".into()], |_| Ok(None)).is_err());

        let mut since = 0;
        let job = loop {
            let job = backend::status(JobKind::Download, since).await.unwrap();
            if job.finished {
                break job;
            }
            since = job.version;
        };
        assert_eq!(job.items[0].status, ItemStatus::Done);
        assert_eq!(job.items[0].output.as_deref(), Some("out"));
        assert_eq!(job.items[0].message.as_deref(), Some("item 0"));
        assert_eq!(job.items[1].status, ItemStatus::Failed);
        assert_eq!(job.items[1].error.as_deref(), Some("boom"));
    }
}
