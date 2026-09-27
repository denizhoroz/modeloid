//! Dataset downloads. One job (one or more links, run in order) at a time.
//! The page starts a job, then long-polls `download_status` for progress.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

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

/// The step a link is on right now (e.g. "Downloading", "Extracting").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    pub label: String,
    pub unit: ProgressUnit,
    pub done: u64,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadItem {
    pub link: String,
    pub status: ItemStatus,
    pub stage: Option<Stage>,
    /// Latest note from the downloader, e.g. "Using latest version: 3".
    pub message: Option<String>,
    /// Folder the dataset was saved to.
    pub saved_to: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadJob {
    /// Increases on every change, across jobs. Pass it back as `since`.
    pub version: u64,
    pub items: Vec<DownloadItem>,
    pub finished: bool,
}

/// Link lines that don't look like URLs, as (line number, text). Empty lines are ignored.
pub fn invalid_links(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .map(|(i, line)| (i + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty())
        .filter(|(_, line)| !(line.starts_with("https://") || line.starts_with("http://")))
        .map(|(n, line)| (n, line.to_string()))
        .collect()
}

/// Non-empty, trimmed link lines.
pub fn parse_links(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

/// Starts downloading `links` in order. `target` is the parent folder for the
/// datasets; `None` means the workspace's `datasets/` folder.
#[post("/api/download/start")]
pub async fn start_download(
    links: Vec<String>,
    format: String,
    target: Option<String>,
) -> Result<DownloadJob, ServerFnError> {
    backend::start(links, format, target).map_err(ServerFnError::new)
}

/// Returns the current job once its `version` is newer than `since`, waiting
/// up to ~20s for a change. `None` if no download has run yet.
#[post("/api/download/status")]
pub async fn download_status(since: u64) -> Result<Option<DownloadJob>, ServerFnError> {
    Ok(backend::status(since).await)
}

#[cfg(feature = "server")]
mod backend {
    use std::path::PathBuf;
    use std::pin::pin;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use modeloid::{DownloadOptions, Progress, Reporter, Unit, Workspace};
    use tokio::sync::Notify;
    use tokio::time::{sleep_until, timeout_at, Instant};

    use super::{DownloadItem, DownloadJob, ItemStatus, ProgressUnit, Stage};
    use crate::api::{settings, workspace};

    const MAX_LINKS: usize = 50;
    const POLL_TIMEOUT: Duration = Duration::from_secs(20);
    /// Minimum time a status request takes, so progress updates don't flood requests.
    const POLL_MIN: Duration = Duration::from_millis(300);

    static VERSION: AtomicU64 = AtomicU64::new(0);
    static CURRENT: Mutex<Option<Arc<Shared>>> = Mutex::new(None);
    static JOB_STARTED: Notify = Notify::const_new();

    struct Shared {
        job: Mutex<DownloadJob>,
        changed: Notify,
    }

    impl Shared {
        fn update(&self, f: impl FnOnce(&mut DownloadJob)) {
            {
                let mut job = self.job.lock().unwrap();
                f(&mut job);
                job.version = VERSION.fetch_add(1, Ordering::SeqCst) + 1;
            }
            self.changed.notify_waiters();
        }

        fn item(&self, index: usize, f: impl FnOnce(&mut DownloadItem)) {
            self.update(|job| f(&mut job.items[index]));
        }

        fn snapshot(&self) -> DownloadJob {
            self.job.lock().unwrap().clone()
        }
    }

    fn current() -> Option<Arc<Shared>> {
        CURRENT.lock().unwrap().clone()
    }

    fn validate_format(format: &str) -> Result<(), String> {
        let ok = !format.is_empty()
            && format.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if ok {
            Ok(())
        } else {
            Err(format!("'{format}' isn't a valid format name. Use lowercase letters, digits and dashes."))
        }
    }

    pub fn start(links: Vec<String>, format: String, target: Option<String>) -> Result<DownloadJob, String> {
        let links: Vec<String> = links.iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        if links.is_empty() {
            return Err("Enter at least one link.".into());
        }
        if links.len() > MAX_LINKS {
            return Err(format!("Up to {MAX_LINKS} links at a time."));
        }
        let format = format.trim().to_lowercase();
        validate_format(&format)?;

        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        let ws = Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))?;
        let target = match target {
            Some(t) => {
                let t = PathBuf::from(t);
                if !t.is_dir() {
                    return Err(format!("{} isn't a folder. Choose another target.", t.display()));
                }
                t
            }
            None => ws.datasets_dir(),
        };

        let mut slot = CURRENT.lock().unwrap();
        if slot.as_ref().is_some_and(|s| !s.snapshot().finished) {
            return Err("A download is already running. Wait for it to finish.".into());
        }
        let shared = Arc::new(Shared {
            job: Mutex::new(DownloadJob {
                version: VERSION.fetch_add(1, Ordering::SeqCst) + 1,
                items: links
                    .iter()
                    .map(|link| DownloadItem {
                        link: link.clone(),
                        status: ItemStatus::Queued,
                        stage: None,
                        message: None,
                        saved_to: None,
                        error: None,
                    })
                    .collect(),
                finished: false,
            }),
            changed: Notify::new(),
        });
        *slot = Some(shared.clone());
        drop(slot);
        JOB_STARTED.notify_waiters();

        let opts = DownloadOptions { api_key: settings::roboflow_api_key(), format };
        let job = shared.clone();
        std::thread::Builder::new()
            .name("modeloid-download".into())
            .spawn(move || run(job, ws, links, opts, target))
            .map_err(|e| format!("Couldn't start the download: {e}"))?;

        Ok(shared.snapshot())
    }

    /// Runs on its own thread: the core's download futures aren't `Send`.
    fn run(job: Arc<Shared>, ws: Workspace, links: Vec<String>, opts: DownloadOptions, target: PathBuf) {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
        for (index, link) in links.iter().enumerate() {
            job.item(index, |item| item.status = ItemStatus::Running);
            let result = match &runtime {
                Ok(rt) => {
                    let reporter = Arc::new(JobReporter { job: job.clone(), index });
                    rt.block_on(ws.download_to(link, &opts, &target, reporter)).map_err(|e| e.to_string())
                }
                Err(e) => Err(format!("Couldn't start the download runtime: {e}")),
            };
            job.item(index, |item| {
                item.stage = None;
                match result {
                    Ok(dir) => {
                        item.status = ItemStatus::Done;
                        item.saved_to = Some(dir.display().to_string());
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

    pub async fn status(since: u64) -> Option<DownloadJob> {
        let started = Instant::now();
        let deadline = started + POLL_TIMEOUT;

        let shared = loop {
            let started_signal = pin!(JOB_STARTED.notified());
            if let Some(shared) = current() {
                break shared;
            }
            if timeout_at(deadline, started_signal).await.is_err() {
                return None;
            }
        };

        loop {
            let mut changed = pin!(shared.changed.notified());
            changed.as_mut().enable();
            let job = shared.snapshot();
            if job.version > since {
                sleep_until(started + POLL_MIN).await;
                return Some(shared.snapshot());
            }
            if timeout_at(deadline, changed).await.is_err() {
                return Some(job);
            }
        }
    }

    /// Feeds core progress into the job for one link.
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

    #[cfg(test)]
    mod tests {
        use super::validate_format;

        #[test]
        fn validates_formats() {
            assert!(validate_format("coco").is_ok());
            assert!(validate_format("yolov8-obb").is_ok());
            assert!(validate_format("").is_err());
            assert!(validate_format("COCO").is_err());
            assert!(validate_format("a/b").is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{invalid_links, parse_links};

    #[test]
    fn parses_link_lines() {
        let text = "  https://a.com/x \n\nhttp://b.com/y\nnot a link\n";
        assert_eq!(parse_links(text), vec!["https://a.com/x", "http://b.com/y", "not a link"]);
        assert_eq!(invalid_links(text), vec![(4, "not a link".to_string())]);
    }
}
