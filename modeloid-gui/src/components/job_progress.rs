use dioxus::prelude::*;

use crate::api::error_message;
use crate::components::{IconName, TitleIcon};
use crate::api::jobs::{cancel_job, ItemAction, ItemStatus, Job, JobItem, JobKind, ProgressUnit, Stage};
use crate::state::use_activity;
use crate::util::{format_bytes, plural};

/// Latest `kind` job, kept fresh from the app-wide activity poll
/// (`state::provide_activity_state`). Pages may also set it right after
/// starting a job (`set_if_newer`). Returns the job and a connection error.
///
/// A job that had already finished when the page opened is left out: after
/// switching sections and back the page starts clean, and the sidebar's
/// History has it. A running job, or one that changes later, shows.
pub fn use_job(kind: JobKind) -> (Signal<Option<Job>>, Memo<Option<String>>) {
    let job = use_signal(|| None::<Job>);
    let activity = use_activity();
    // The version of the job that was already finished on opening (`Some(None)`: none was).
    let mut finished_before = use_signal(|| None::<Option<u64>>);

    use_effect(move || {
        let snapshot = activity.snapshot.read();
        let Some(snapshot) = snapshot.as_ref() else { return };
        let latest = snapshot.jobs.iter().find(|j| j.kind == kind).cloned();
        let before = *finished_before.peek();
        let before = before.unwrap_or_else(|| {
            let seen = latest.as_ref().filter(|j| j.finished).map(|j| j.version);
            finished_before.set(Some(seen));
            seen
        });
        if let Some(latest) = latest {
            if !before.is_some_and(|version| latest.finished && latest.version <= version) {
                set_if_newer(job, latest);
            }
        }
    });
    let error = use_memo(move || activity.error.read().clone());

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
    /// Show a Cancel button while the job runs.
    #[props(default = true)]
    cancellable: bool,
    /// Called with an item's position and the action its error offers, when
    /// the user takes it. Without a handler, offered actions aren't shown.
    #[props(default)]
    on_action: Option<EventHandler<(usize, ItemAction)>>,
) -> Element {
    let mut cancel_error = use_signal(|| None::<String>);
    let total = job.items.len();
    let count = |status| job.items.iter().filter(|i| i.status == status).count();
    let (done, failed, cancelled) = (count(ItemStatus::Done), count(ItemStatus::Failed), count(ItemStatus::Cancelled));
    let (warnings, skipped) = (count(ItemStatus::Warning), count(ItemStatus::Skipped));

    let summary = if job.finished {
        let mut parts = Vec::new();
        if done > 0 || (failed == 0 && cancelled == 0 && warnings == 0 && skipped == 0) {
            parts.push(format!("{} {verb}", plural(done, &noun)));
        }
        if failed > 0 {
            parts.push(format!("{failed} failed"));
        }
        if warnings > 0 {
            parts.push(plural(warnings, "warning"));
        }
        if skipped > 0 {
            parts.push(format!("{skipped} skipped"));
        }
        if cancelled > 0 {
            parts.push(format!("{cancelled} cancelled"));
        }
        format!("Finished: {}.", parts.join(", "))
    } else if job.cancel_requested {
        "Cancelling…".to_string()
    } else {
        let ended = done + failed + warnings + skipped;
        format!("{ended} of {total} done{}", if failed > 0 { format!(", {failed} failed") } else { String::new() })
    };

    let kind = job.kind;
    let cancel = move |_| async move {
        cancel_error.set(None);
        if let Err(e) = cancel_job(kind).await {
            cancel_error.set(Some(error_message(&e)));
        }
    };

    rsx! {
        section { class: "section-card", aria_labelledby: "job-progress-title",
            div { class: "section-card-header",
                h2 { id: "job-progress-title", class: "section-card-title", TitleIcon { name: IconName::Activity } "{title}" }
                p { class: "section-card-meta", aria_live: "polite", "{summary}" }
                if cancellable && !job.finished {
                    button {
                        class: "button button-secondary button-sm",
                        r#type: "button",
                        disabled: job.cancel_requested,
                        onclick: cancel,
                        "Cancel"
                    }
                }
            }
            if let Some(message) = cancel_error() {
                p { class: "field-error", role: "alert", "{message}" }
            }
            ul { class: "progress-list",
                for (index, item) in job.items.iter().enumerate() {
                    ProgressRow { key: "{index}", index, item: item.clone(), output_label: output_label.clone(), busy: !job.finished, on_action }
                }
            }
        }
    }
}

/// What an offered action does, and the button that starts it.
fn action_text(action: ItemAction) -> (&'static str, &'static str) {
    match action {
        ItemAction::RoboflowFork => (
            "Modeloid can copy this project into your Roboflow workspace, generate a version there, download it, and delete the copy again.",
            "Copy, generate and download",
        ),
    }
}

#[component]
fn ProgressRow(
    index: usize,
    item: JobItem,
    output_label: String,
    /// The job is still running: actions wait.
    busy: bool,
    on_action: Option<EventHandler<(usize, ItemAction)>>,
) -> Element {
    let (status_class, status_text) = match item.status {
        ItemStatus::Queued => ("is-queued", "Queued".to_string()),
        ItemStatus::Running => ("is-running", item.stage.as_ref().map_or_else(|| "Starting".into(), |s| s.label.clone())),
        ItemStatus::Done => ("is-done", "Done".to_string()),
        ItemStatus::Failed => ("is-failed", "Failed".to_string()),
        ItemStatus::Warning => ("is-warning", "Warning".to_string()),
        ItemStatus::Skipped => ("is-skipped", "Skipped".to_string()),
        ItemStatus::Cancelled => ("is-cancelled", "Cancelled".to_string()),
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
            // A done item keeps its last note (e.g. which format was used instead).
            if matches!(item.status, ItemStatus::Running | ItemStatus::Done) {
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
                // Red only for a real failure; warnings and skips carry their pill's colour.
                p { class: if item.status == ItemStatus::Failed { "field-error" } else { "progress-reason" }, "{error}" }
            }
            if let (Some(action), Some(handler)) = (item.action, on_action) {
                div { class: "progress-action",
                    p { class: "progress-note", "{action_text(action).0}" }
                    button {
                        class: "button button-secondary button-sm",
                        r#type: "button",
                        disabled: busy,
                        onclick: move |_| handler.call((index, action)),
                        "{action_text(action).1}"
                    }
                }
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
