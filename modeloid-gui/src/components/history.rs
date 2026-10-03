//! The sidebar's History button and the overlay it opens next to it: the open
//! workspace's processes, newest first ("Dataset Manager > Datasets >
//! Download", when, how it ended, and what each item made or why it failed).
//! Pages only show the job they started; finished ones live here.

use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::history::{format_duration, workspace_history, HistoryEntry, Outcome};
use crate::components::{Icon, IconName};
use crate::state::{use_activity, use_workspace};
use crate::util::{format_count, plural};

fn outcome_class(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "is-done",
        Outcome::Skipped => "is-skipped",
        Outcome::Cancelled => "is-cancelled",
        Outcome::Warning => "is-warning",
        Outcome::Failed => "is-failed",
    }
}

/// "1 process", "3 processes".
fn count_processes(n: usize) -> String {
    if n == 1 { "1 process".into() } else { format!("{} processes", format_count(n as u64)) }
}

/// Whether the History overlay is open. Provided by `AppLayout`, which draws
/// the overlay outside the sidebar (the sidebar scrolls and is its own layer).
#[derive(Clone, Copy)]
pub struct HistoryOpen(pub Signal<bool>);

/// "History" in the sidebar, above Settings; toggles the overlay.
#[component]
pub fn HistoryButton() -> Element {
    let HistoryOpen(mut open) = use_context::<HistoryOpen>();

    rsx! {
        div { class: "sidebar-history",
            button {
                class: if open() { "sidebar-link is-active" } else { "sidebar-link" },
                r#type: "button",
                aria_haspopup: "dialog",
                aria_expanded: if open() { "true" } else { "false" },
                aria_controls: "history-overlay",
                onclick: move |_| open.toggle(),
                Icon { name: IconName::History, size: 16 }
                span { "History" }
            }
        }
    }
}

/// The overlay while it's open; place once, in the app shell.
#[component]
pub fn HistoryLayer() -> Element {
    let HistoryOpen(mut open) = use_context::<HistoryOpen>();
    rsx! {
        if open() {
            HistoryOverlay { on_close: move |_| open.set(false) }
        }
    }
}

#[component]
fn HistoryOverlay(on_close: EventHandler<()>) -> Element {
    let workspace = use_workspace();
    let activity = use_activity();
    // Changes only when a job ends (not on every progress update): read the history again then.
    let ended = use_memo(move || {
        activity.snapshot.read().as_ref().map(|s| s.jobs.iter().filter(|j| j.finished).map(|j| j.version).max())
    });
    let history = use_resource(move || async move {
        let _ = ended();
        let _ = workspace.read().as_ref().map(|w| w.path.clone());
        workspace_history().await
    });
    let has_workspace = workspace.read().is_some();

    rsx! {
        // Clicking anywhere outside closes it.
        div { class: "history-backdrop", onclick: move |_| on_close.call(()) }
        div {
            id: "history-overlay",
            class: "history-overlay",
            role: "dialog",
            aria_labelledby: "history-title",
            tabindex: "-1",
            onmounted: move |e| async move {
                let _ = e.set_focus(true).await;
            },
            onkeydown: move |e| {
                if e.key() == Key::Escape {
                    on_close.call(());
                }
            },
            div { class: "history-header",
                h2 { id: "history-title", class: "section-card-title", "History" }
                if let Some(Ok(rows)) = &*history.read() {
                    span { class: "section-card-meta", "{count_processes(rows.len())}" }
                }
                button {
                    class: "icon-button icon-button-sm history-close",
                    r#type: "button",
                    aria_label: "Close history",
                    onclick: move |_| on_close.call(()),
                    "×"
                }
            }
            div { class: "history-body",
                match &*history.read() {
                    None => rsx! { p { class: "field-hint", "Loading…" } },
                    Some(Err(e)) => rsx! { p { class: "form-error", role: "alert", "{error_message(e)}" } },
                    Some(Ok(_)) if !has_workspace => rsx! { p { class: "field-hint", "Open a workspace to see its history." } },
                    Some(Ok(rows)) if rows.is_empty() => rsx! {
                        p { class: "field-hint", "Nothing yet. What you do in this workspace shows up here." }
                    },
                    Some(Ok(rows)) => rsx! {
                        ol { class: "history-list",
                            for (index, row) in rows.iter().enumerate() {
                                HistoryRowView { key: "{index}-{row.entry.finished}", entry: row.entry.clone(), when: row.when.clone() }
                            }
                        }
                    },
                }
            }
        }
    }
}

#[component]
fn HistoryRowView(entry: HistoryEntry, when: String) -> Element {
    let outcome = entry.outcome();
    let mut meta = vec![when];
    if entry.seconds() > 0 {
        meta.push(format!("took {}", format_duration(entry.seconds())));
    }
    if entry.items.len() > 1 {
        meta.push(plural(entry.items.len(), "item"));
    }
    let meta = meta.join(" · ");

    rsx! {
        li { class: "history-entry {outcome_class(outcome)}",
            div { class: "history-entry-head",
                p { class: "history-path",
                    span { class: "history-crumb", "{entry.category}" }
                    span { class: "history-sep", aria_hidden: "true", ">" }
                    span { class: "history-crumb", "{entry.section}" }
                    span { class: "history-sep", aria_hidden: "true", ">" }
                    strong { class: "history-process", "{entry.process}" }
                }
                span { class: "status-pill", "{outcome.label()}" }
            }
            p { class: "progress-note", "{meta}" }
            ul { class: "history-items",
                for (index, item) in entry.items.iter().enumerate() {
                    li { key: "{index}", class: "history-item",
                        span { class: "history-item-label", title: "{item.label}", "{item.label}" }
                        // The entry's pill says it when there's one item.
                        if entry.items.len() > 1 && item.outcome != Outcome::Done {
                            span { class: "history-item-outcome {outcome_class(item.outcome)}", "{item.outcome.label()}" }
                        }
                        if let Some(detail) = &item.detail {
                            p { class: if item.outcome == Outcome::Failed { "history-item-detail field-error" } else { "history-item-detail" }, "{detail}" }
                        }
                    }
                }
            }
        }
    }
}
