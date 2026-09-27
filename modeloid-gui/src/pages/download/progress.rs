use dioxus::prelude::*;

use crate::api::download::{download_status, DownloadItem, DownloadJob, ItemStatus, ProgressUnit, Stage};
use crate::api::error_message;
use crate::util::{format_bytes, plural};

/// Latest download job, kept fresh by long-polling the server.
/// Returns the job and a connection error, if polling stopped.
pub fn use_download_job() -> (Signal<Option<DownloadJob>>, Signal<Option<String>>) {
    let job = use_signal(|| None::<DownloadJob>);
    let mut error = use_signal(|| None::<String>);

    use_future(move || async move {
        let mut since = 0;
        loop {
            match download_status(since).await {
                Ok(Some(latest)) => {
                    since = latest.version;
                    set_if_newer(job, latest);
                }
                Ok(None) => {} // no download yet; the server already waited
                Err(e) => {
                    error.set(Some(error_message(&e)));
                    break;
                }
            }
        }
    });

    (job, error)
}

/// Replaces the job unless `latest` is older (a slow poll can race a fresh start).
pub fn set_if_newer(mut job: Signal<Option<DownloadJob>>, latest: DownloadJob) {
    let newer = job.read().as_ref().is_none_or(|current| latest.version > current.version);
    if newer {
        job.set(Some(latest));
    }
}

#[component]
pub fn ProgressPanel(job: DownloadJob) -> Element {
    let total = job.items.len();
    let done = job.items.iter().filter(|i| i.status == ItemStatus::Done).count();
    let failed = job.items.iter().filter(|i| i.status == ItemStatus::Failed).count();

    let summary = if job.finished {
        match (done, failed) {
            (_, 0) => format!("Finished: {} downloaded.", plural(done, "dataset")),
            (0, _) => format!("Finished: {} failed.", plural(failed, "download")),
            _ => format!("Finished: {done} downloaded, {failed} failed."),
        }
    } else {
        format!("{} of {total} done{}", done + failed, if failed > 0 { format!(", {failed} failed") } else { String::new() })
    };

    rsx! {
        section { class: "section-card", aria_labelledby: "download-progress-title",
            div { class: "section-card-header",
                h2 { id: "download-progress-title", class: "section-card-title", "Downloads" }
                p { class: "section-card-meta", aria_live: "polite", "{summary}" }
            }
            ul { class: "progress-list",
                for (index, item) in job.items.iter().enumerate() {
                    ProgressRow { key: "{index}", item: item.clone() }
                }
            }
        }
    }
}

#[component]
fn ProgressRow(item: DownloadItem) -> Element {
    let (status_class, status_text) = match item.status {
        ItemStatus::Queued => ("is-queued", "Queued".to_string()),
        ItemStatus::Running => ("is-running", item.stage.as_ref().map_or("Starting".into(), |s| s.label.clone())),
        ItemStatus::Done => ("is-done", "Done".to_string()),
        ItemStatus::Failed => ("is-failed", "Failed".to_string()),
    };

    rsx! {
        li { class: "progress-item {status_class}",
            div { class: "progress-item-head",
                span { class: "status-pill", "{status_text}" }
                span { class: "progress-link", title: "{item.link}", "{item.link}" }
            }

            if let Some(stage) = &item.stage {
                StageBar { stage: stage.clone() }
            }
            if item.status == ItemStatus::Running {
                if let Some(message) = &item.message {
                    p { class: "progress-note", "{message}" }
                }
            }
            if let Some(path) = &item.saved_to {
                p { class: "progress-note", "Saved to "
                    code { "{path}" }
                }
            }
            if let Some(error) = &item.error {
                p { class: "field-error", "{error}" }
            }
        }
    }
}

#[component]
fn StageBar(stage: Stage) -> Element {
    let amount = |n: u64| match stage.unit {
        ProgressUnit::Bytes => format_bytes(n),
        ProgressUnit::Files | ProgressUnit::Steps => n.to_string(),
    };
    let suffix = if stage.unit == ProgressUnit::Files { " files" } else { "" };
    let detail = match stage.total {
        Some(total) => format!("{} / {}{suffix}", amount(stage.done), amount(total)),
        None => format!("{}{suffix}", amount(stage.done)),
    };

    rsx! {
        div { class: "stage",
            match stage.total {
                Some(total) => rsx! {
                    progress { class: "progress-bar", max: "{total.max(1)}", value: "{stage.done.min(total)}", aria_label: "{stage.label}" }
                },
                None => rsx! {
                    progress { class: "progress-bar", aria_label: "{stage.label}" }
                },
            }
            span { class: "stage-detail", "{detail}" }
        }
    }
}
