use dioxus::prelude::*;

use crate::api::activity::{cancel_task, ActivitySnapshot, JobSummary, TaskInfo};
use crate::api::error_message;
use crate::api::jobs::{cancel_job, JobKind, ProgressUnit};
use crate::state::use_activity;
use crate::util::{format_bytes, format_count};

#[derive(Clone, Copy, PartialEq)]
enum Stop {
    Job(JobKind),
    Task(u64),
}

/// One running thing, as the bar shows it.
#[derive(Clone, PartialEq)]
struct Row {
    key: String,
    /// What it is, e.g. "Download · coco8 (1/3)".
    text: String,
    /// How far along, e.g. "Downloading 45%".
    detail: Option<String>,
    stop: Stop,
    stopping: bool,
}

fn amount(n: u64, unit: Option<ProgressUnit>) -> String {
    match unit {
        Some(ProgressUnit::Bytes) => format_bytes(n),
        _ => format_count(n),
    }
}

fn progress(done: u64, total: Option<u64>, unit: Option<ProgressUnit>) -> Option<String> {
    match total {
        Some(total) if total > 0 => Some(format!("{}%", (done * 100 / total).min(100))),
        _ if done > 0 => Some(amount(done, unit)),
        _ => None,
    }
}

fn job_row(job: &JobSummary) -> Row {
    let mut text = job.title.clone();
    if let Some(item) = &job.item {
        text.push_str(&format!(" · {item}"));
    }
    if job.count > 1 && job.index > 0 {
        text.push_str(&format!(" ({}/{})", job.index, job.count));
    }
    let detail = if job.cancel_requested {
        Some("Stopping…".to_string())
    } else {
        let amount = progress(job.done, job.total, job.unit);
        match (&job.stage, amount) {
            (Some(stage), Some(amount)) => Some(format!("{stage} {amount}")),
            (Some(stage), None) => Some(stage.clone()),
            (None, amount) => amount,
        }
    };
    Row { key: "job".into(), text, detail, stop: Stop::Job(job.kind), stopping: job.cancel_requested }
}

fn task_row(task: &TaskInfo) -> Row {
    let detail = if task.cancelled {
        Some("Stopping…".to_string())
    } else {
        match task.total {
            Some(total) => Some(format!("{}/{}", format_count(task.done), format_count(total))),
            None if task.done > 0 => Some(format_count(task.done)),
            None => None,
        }
    };
    Row { key: format!("task-{}", task.id), text: task.label.clone(), detail, stop: Stop::Task(task.id), stopping: task.cancelled }
}

fn rows(snapshot: &ActivitySnapshot) -> Vec<Row> {
    snapshot.job.iter().map(job_row).chain(snapshot.tasks.iter().map(task_row)).collect()
}

/// Thin bar along the bottom of every page: what the server is busy with
/// (the running job, background loads), with a spinner and Stop buttons.
#[component]
pub fn StatusBar() -> Element {
    let activity = use_activity();
    let mut list_open = use_signal(|| false);
    let mut stop_error = use_signal(|| None::<String>);
    let lost = move || activity.error.read().is_some();

    let stop = move |stop: Stop| async move {
        stop_error.set(None);
        let result = match stop {
            Stop::Job(kind) => cancel_job(kind).await,
            Stop::Task(id) => cancel_task(id).await,
        };
        if let Err(e) = result {
            stop_error.set(Some(error_message(&e)));
        }
    };

    let rows = activity.snapshot.read().as_ref().map(rows).unwrap_or_default();
    let busy = !rows.is_empty();
    let show_list = list_open() && rows.len() > 1;
    let announce = match rows.first() {
        Some(row) => row.text.clone(),
        None if lost() => "Lost connection to the server".to_string(),
        None => "No jobs running".to_string(),
    };

    rsx! {
        footer {
            class: "status-bar",
            aria_label: "Running jobs",
            onkeydown: move |e| {
                if e.key() == Key::Escape {
                    list_open.set(false);
                }
            },

            // Announces what starts and stops, not every percent.
            p { class: "visually-hidden", role: "status", "{announce}" }

            if busy {
                span { class: "status-spinner", aria_hidden: "true" }
            } else {
                span { class: if lost() { "status-dot is-lost" } else { "status-dot" }, aria_hidden: "true" }
            }

            match rows.first() {
                None => rsx! {
                    span { class: "status-text is-idle", aria_hidden: "true", "{announce}" }
                },
                Some(row) => rsx! {
                    span { class: "status-text", aria_hidden: "true",
                        "{row.text}"
                        if let Some(detail) = &row.detail {
                            span { class: "status-detail", "{detail}" }
                        }
                    }
                    StopButton { row: row.clone(), on_stop: stop }
                },
            }

            if rows.len() > 1 {
                div { class: "status-more",
                    button {
                        class: "status-button",
                        r#type: "button",
                        aria_expanded: if show_list { "true" } else { "false" },
                        aria_controls: "status-list",
                        onclick: move |_| list_open.toggle(),
                        "+{rows.len() - 1} more"
                    }
                    if show_list {
                        ul { id: "status-list", class: "status-list",
                            for row in rows.iter().cloned() {
                                li { key: "{row.key}", class: "status-list-item",
                                    span { class: "status-text",
                                        "{row.text}"
                                        if let Some(detail) = &row.detail {
                                            span { class: "status-detail", "{detail}" }
                                        }
                                    }
                                    StopButton { row, on_stop: stop }
                                }
                            }
                        }
                    }
                }
            }

            if let Some(message) = stop_error() {
                span { class: "status-error", role: "alert", "{message}" }
            }
        }
    }
}

#[component]
fn StopButton(row: Row, on_stop: EventHandler<Stop>) -> Element {
    let stop = row.stop;
    rsx! {
        button {
            class: "status-button",
            r#type: "button",
            disabled: row.stopping,
            aria_label: "Stop {row.text}",
            onclick: move |_| on_stop.call(stop),
            if row.stopping { "Stopping…" } else { "Stop" }
        }
    }
}
