//! What the server is busy with, for the status bar: the running job (see
//! `api/jobs.rs`) and background loads (label reading, gallery pages,
//! previews...) started by pages.
//!
//! Background loads run through `track`: a newer load in the same slot (the
//! user clicked another dataset) or a dropped request (the page went away)
//! cancels the older one, so they don't pile up on the server.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::api::jobs::{Job, JobKind, ProgressUnit};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivitySnapshot {
    /// Pass back as `since`.
    pub version: u64,
    pub job: Option<JobSummary>,
    pub tasks: Vec<TaskInfo>,
    /// Latest job of every kind that has run, for the tool pages' progress.
    pub jobs: Vec<Job>,
}

/// The running job, cut down to one line's worth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobSummary {
    pub kind: JobKind,
    /// e.g. "Download".
    pub title: String,
    /// What the current item works on, e.g. a dataset name.
    pub item: Option<String>,
    /// 1-based position of the current item.
    pub index: usize,
    pub count: usize,
    /// e.g. "Downloading".
    pub stage: Option<String>,
    pub unit: Option<ProgressUnit>,
    pub done: u64,
    pub total: Option<u64>,
    pub cancel_requested: bool,
}

/// A background load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: u64,
    /// e.g. "Reading labels · coco8".
    pub label: String,
    pub done: u64,
    pub total: Option<u64>,
    /// Asked to stop; the work ends at its next check.
    pub cancelled: bool,
}

/// Returns once anything changed after `since` (or after ~20s regardless).
#[post("/api/activity/status")]
pub async fn activity_status(since: u64) -> Result<ActivitySnapshot, ServerFnError> {
    Ok(backend::status(since).await)
}

/// Stops background load `id`. Loads that already finished are ignored.
#[post("/api/activity/cancel")]
pub async fn cancel_task(id: u64) -> Result<(), ServerFnError> {
    backend::cancel(id);
    Ok(())
}

#[cfg(feature = "server")]
pub use backend::{changed, track, TaskCtx};

#[cfg(feature = "server")]
mod backend {
    use std::collections::BTreeMap;
    use std::pin::pin;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, LazyLock, Mutex};
    use std::time::Duration;

    use modeloid::Progress;
    use tokio::sync::Notify;
    use tokio::time::{sleep_until, Instant};

    use super::{ActivitySnapshot, JobSummary, TaskInfo};
    use crate::api::jobs::{self, ItemStatus, Job};

    const POLL_TIMEOUT: Duration = Duration::from_secs(20);
    /// Minimum time a status request takes, so progress doesn't flood requests.
    const POLL_MIN: Duration = Duration::from_millis(300);
    /// Wait after a change before answering, so loads that finish quickly
    /// (cached statistics) never flash in the bar.
    const SETTLE: Duration = Duration::from_millis(250);

    /// Starts at 1 so a first request (`since = 0`) answers right away.
    static VERSION: AtomicU64 = AtomicU64::new(1);
    static CHANGED: Notify = Notify::const_new();
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    static TASKS: LazyLock<Mutex<BTreeMap<u64, Task>>> = LazyLock::new(Default::default);

    struct Task {
        slot: &'static str,
        label: String,
        done: u64,
        total: Option<u64>,
        cancelled: Arc<AtomicBool>,
    }

    /// Wakes status requests. Jobs call it on every update.
    pub fn changed() {
        VERSION.fetch_add(1, Ordering::SeqCst);
        CHANGED.notify_waiters();
    }

    fn with_task(id: u64, f: impl FnOnce(&mut Task)) {
        if let Some(task) = TASKS.lock().unwrap().get_mut(&id) {
            f(task);
        }
        changed();
    }

    pub fn cancel(id: u64) {
        with_task(id, |task| task.cancelled.store(true, Ordering::Relaxed));
    }

    /// Handed to a tracked load: its progress and cancel flag.
    pub struct TaskCtx {
        id: u64,
        cancelled: Arc<AtomicBool>,
    }

    impl TaskCtx {
        /// Replaces the label, e.g. once the dataset's name is known.
        pub fn set_label(&self, label: impl Into<String>) {
            let label = label.into();
            with_task(self.id, |task| task.label = label);
        }

        /// Progress that counts into this task and reports its cancel flag.
        pub fn progress(&self) -> TaskProgress {
            TaskProgress { id: self.id, cancelled: self.cancelled.clone() }
        }
    }

    pub struct TaskProgress {
        id: u64,
        cancelled: Arc<AtomicBool>,
    }

    impl Progress for TaskProgress {
        fn set_total(&self, total: u64) {
            with_task(self.id, |task| task.total = Some(total));
        }

        fn add(&self, n: u64) {
            with_task(self.id, |task| task.done += n);
        }

        fn finish(&self) {}

        fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::Relaxed)
        }
    }

    /// Cancels the load if the request is dropped before it finished.
    struct CancelOnDrop(Arc<AtomicBool>);

    impl Drop for CancelOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    /// Runs `work` on a blocking thread, listed in the status bar as `label`
    /// until it returns. Cancels earlier loads in the same `slot`. A cancelled
    /// load gives `Err("Cancelled")`.
    pub async fn track<T: Send + 'static>(
        slot: &'static str,
        label: impl Into<String>,
        work: impl FnOnce(&TaskCtx) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let mut tasks = TASKS.lock().unwrap();
            for task in tasks.values().filter(|t| t.slot == slot) {
                task.cancelled.store(true, Ordering::Relaxed);
            }
            let task = Task { slot, label: label.into(), done: 0, total: None, cancelled: cancelled.clone() };
            tasks.insert(id, task);
        }
        changed();

        let _guard = CancelOnDrop(cancelled.clone());
        let ctx = TaskCtx { id, cancelled: cancelled.clone() };
        // Removed when the work really ends, not when the request goes away,
        // so the bar shows loads that are still stopping.
        let result = tokio::task::spawn_blocking(move || {
            let result = work(&ctx);
            TASKS.lock().unwrap().remove(&id);
            changed();
            result
        })
        .await;

        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) if cancelled.load(Ordering::Relaxed) => Err("Cancelled".into()),
            Ok(Err(e)) => Err(e),
            Err(e) => {
                TASKS.lock().unwrap().remove(&id);
                changed();
                Err(e.to_string())
            }
        }
    }

    fn summarize(job: Job) -> JobSummary {
        let count = job.items.len();
        let position = job.items.iter().position(|i| i.status == ItemStatus::Running);
        let current = position.map(|i| &job.items[i]);
        let stage = current.and_then(|i| i.stage.as_ref());
        JobSummary {
            kind: job.kind,
            title: job.kind.label().to_string(),
            item: current.map(|i| i.label.clone()),
            index: position.map_or(0, |i| i + 1),
            count,
            stage: stage.map(|s| s.label.clone()),
            unit: stage.map(|s| s.unit),
            done: stage.map_or(0, |s| s.done),
            total: stage.and_then(|s| s.total),
            cancel_requested: job.cancel_requested,
        }
    }

    pub fn snapshot() -> ActivitySnapshot {
        let version = VERSION.load(Ordering::SeqCst);
        let tasks = TASKS
            .lock()
            .unwrap()
            .iter()
            .map(|(&id, t)| TaskInfo {
                id,
                label: t.label.clone(),
                done: t.done,
                total: t.total,
                cancelled: t.cancelled.load(Ordering::Relaxed),
            })
            .collect();
        ActivitySnapshot { version, job: jobs::running().map(summarize), tasks, jobs: jobs::latest_all() }
    }

    pub async fn status(since: u64) -> ActivitySnapshot {
        let started = Instant::now();
        let deadline = started + POLL_TIMEOUT;
        loop {
            let mut notified = pin!(CHANGED.notified());
            notified.as_mut().enable();
            if VERSION.load(Ordering::SeqCst) > since {
                sleep_until((started + POLL_MIN).max(Instant::now() + SETTLE)).await;
                return snapshot();
            }
            tokio::select! {
                _ = notified => {}
                _ = sleep_until(deadline) => return snapshot(),
            }
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use std::time::Duration;

    use modeloid::Progress;

    use super::backend::{snapshot, track};

    /// Waits until `work` sees its cancel flag, then fails.
    fn until_cancelled(ctx: &super::TaskCtx) -> Result<(), String> {
        let progress = ctx.progress();
        while !progress.is_cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
        Err("stopped".into())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn newer_load_in_slot_cancels_older() {
        let first = tokio::spawn(track("test-slot", "first", until_cancelled));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(snapshot().tasks.iter().any(|t| t.label == "first"));

        let second = track("test-slot", "second", |ctx| {
            ctx.progress().add(1);
            Ok(7)
        })
        .await;
        assert_eq!(second, Ok(7));
        assert_eq!(first.await.unwrap(), Err("Cancelled".to_string()));
        assert!(!snapshot().tasks.iter().any(|t| t.label == "first" || t.label == "second"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dropped_request_cancels_load() {
        let load = tokio::spawn(track("test-drop", "dropped", until_cancelled));
        tokio::time::sleep(Duration::from_millis(50)).await;
        load.abort();
        for _ in 0..100 {
            if !snapshot().tasks.iter().any(|t| t.label == "dropped") {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the load kept running after its request was dropped");
    }
}
