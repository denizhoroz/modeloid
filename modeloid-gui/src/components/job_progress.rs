use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::jobs::{job_status, ItemStatus, Job, JobItem, JobKind, ProgressUnit, Stage};
use crate::state::use_datasets_version;
use crate::util::{format_bytes, plural};

/// Latest `kind` job, kept fresh by long-polling the server.
/// Returns the job and a connection error, if polling stopped.
/// Every finished item refreshes the datasets panel, since tools add or change datasets.
pub fn use_job(kind: JobKind) -> (Signal<Option<Job>>, Signal<Option<String>>) {
    let job = use_signal(|| None::<Job>);
    let mut error = use_signal(|| None::<String>);
    let mut datasets_version = use_datasets_version();

    use_future(move || async move {
        let mut since = 0;
        let mut done_seen = 0;
        loop {
            match job_status(kind, since).await {
                Ok(Some(latest)) => {
                    since = latest.version;
                    let done = latest.items.iter().filter(|i| i.status == ItemStatus::Done).count();
                    if done != done_seen {
                        done_seen = done;
                        *datasets_version.write() += 1;
                    }
                    set_if_newer(job, latest);
                }
                Ok(None) => {} // no job yet; the server already waited
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
pub fn set_if_newer(mut job: Signal<Option<Job>>, latest: Job) {
    let newer = job.read().as_ref().is_none_or(|current| latest.version > current.version);
    if newer {
        job.set(Some(latest));
    }
}

/// Progress of a job: one row per item with its stage bar, notes, output and error.
#[component]
pub fn JobProgress(
    job: Job,
    /// Section title, e.g. "Downloads".
    #[props(into)]
    title: String,
    /// What an item makes, for the summary, e.g. "dataset".
    #[props(into, default = "item".to_string())]
    noun: String,
    /// Past tense for a done item, e.g. "downloaded".
    #[props(into, default = "done".to_string())]
    verb: String,
    /// Shown before an item's output, e.g. "Saved to".
    #[props(into, default = "Saved to".to_string())]
    output_label: String,
) -> Element {
    let total = job.items.len();
    let done = job.items.iter().filter(|i| i.status == ItemStatus::Done).count();
    let failed = job.items.iter().filter(|i| i.status == ItemStatus::Failed).count();

    let summary = if job.finished {
        match (done, failed) {
            (_, 0) => format!("Finished: {} {verb}.", plural(done, &noun)),
            (0, _) => format!("Finished: {} failed.", plural(failed, &noun)),
            _ => format!("Finished: {done} {verb}, {failed} failed."),
        }
    } else {
        format!("{} of {total} done{}", done + failed, if failed > 0 { format!(", {failed} failed") } else { String::new() })
    };

    rsx! {
        section { class: "section-card", aria_labelledby: "job-progress-title",
            div { class: "section-card-header",
                h2 { id: "job-progress-title", class: "section-card-title", "{title}" }
                p { class: "section-card-meta", aria_live: "polite", "{summary}" }
            }
            ul { class: "progress-list",
                for (index, item) in job.items.iter().enumerate() {
                    ProgressRow { key: "{index}", item: item.clone(), output_label: output_label.clone() }
                }
            }
        }
    }
}

#[component]
fn ProgressRow(item: JobItem, output_label: String) -> Element {
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
                span { class: "progress-link", title: "{item.label}", "{item.label}" }
            }

            if let Some(stage) = &item.stage {
                StageBar { stage: stage.clone() }
            }
            if item.status == ItemStatus::Running {
                if let Some(message) = &item.message {
                    p { class: "progress-note", "{message}" }
                }
            }
            if let Some(output) = &item.output {
                p { class: "progress-note", "{output_label} "
                    code { "{output}" }
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
