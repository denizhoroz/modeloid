//! Dataset Manager > Split & Balance, two tabs on the dataset selected in the
//! panel. Split: deal the images into train / val / test. Balance: leave
//! classes out, cut dominant classes down, take a random subset. Both show
//! the exact resulting counts before anything is written.

mod balance;
mod split;

use dioxus::prelude::*;

use crate::api::balance::{balance_preview, start_balance, BalancePlan, BalancePreview};
use crate::components::{IconName, TitleIcon};
use crate::api::datasets::list_datasets;
use crate::api::error_message;
use crate::api::jobs::JobKind;
use crate::components::{InfoTip, NoWorkspace, set_if_newer, use_job, JobProgress, OutputChoice, OutputSelect, PrimaryNote, SelectDatasetHint, WarningTip};
use crate::state::{use_datasets_version, use_primary_dataset, use_selected_datasets, use_workspace, use_workspace_checked};
use crate::util::format_count;

use balance::BalanceTab;
use split::SplitTab;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tab {
    Split,
    Balance,
}

#[component]
pub fn SplitBalance() -> Element {
    let workspace = use_workspace();
    let checked = use_workspace_checked();
    let primary = use_primary_dataset();
    let selected = use_selected_datasets();

    rsx! {
        document::Title { "Split & Balance · Modeloid" }
        div { class: "page page-wide",
            p { class: "page-eyebrow", "Dataset Manager" }
            h1 { "Split & Balance" }
            p { class: "page-lead",
                "Split a dataset into train, val and test, or balance it: leave classes out, cut dominant classes down, take a random subset."
            }
            match (workspace(), primary()) {
                (Some(_), Some(id)) => rsx! { Tabs { key: "{id}", id, selected_count: selected.read().len() } },
                (Some(_), None) => rsx! { SelectDatasetHint {} },
                (None, _) if !checked() => rsx! {},
                (None, _) => rsx! { NoWorkspace { text: "Split & Balance works on a workspace's datasets." } },
            }
        }
    }
}

#[component]
fn Tabs(id: u32, selected_count: usize) -> Element {
    let datasets_version = use_datasets_version();
    let mut tab = use_signal(|| Tab::Split);
    let datasets = use_resource(move || async move {
        let _ = datasets_version();
        list_datasets().await
    });
    let name = datasets
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .and_then(|rows| rows.iter().find(|r| r.id == id).map(|r| r.name.clone()))
        .unwrap_or_else(|| format!("Dataset #{id}"));

    rsx! {
        PrimaryNote { count: selected_count }
        div { class: "explore-head",
            div { class: "explore-title",
                h2 { class: "explore-name", "{name}" }
            }
        }
        div { class: "segmented segmented-tabs", role: "tablist", aria_label: "Split or balance",
            for (value, label, slug) in [(Tab::Split, "Split", "split"), (Tab::Balance, "Balance", "balance")] {
                button {
                    key: "{slug}",
                    id: "tab-{slug}",
                    class: if tab() == value { "segmented-option is-selected" } else { "segmented-option" },
                    r#type: "button",
                    role: "tab",
                    aria_selected: if tab() == value { "true" } else { "false" },
                    aria_controls: "panel-{slug}",
                    onclick: move |_| tab.set(value),
                    "{label}"
                }
            }
        }
        // Both stay mounted so each tab keeps its settings and progress.
        div { id: "panel-split", role: "tabpanel", aria_labelledby: "tab-split", hidden: tab() != Tab::Split,
            SplitTab { id, name: name.clone() }
        }
        div { id: "panel-balance", role: "tabpanel", aria_labelledby: "tab-balance", hidden: tab() != Tab::Balance,
            BalanceTab { id, name }
        }
    }
}

/// Preview of `plan`, re-read when the plan or the datasets change. The last
/// good one stays while the next loads; the second value is a failed read.
fn use_preview(plan: Signal<BalancePlan>) -> (Signal<Option<BalancePreview>>, Memo<Option<String>>) {
    let datasets_version = use_datasets_version();
    let mut preview = use_signal(|| None::<BalancePreview>);
    let latest = use_resource(move || async move {
        let _ = datasets_version();
        balance_preview(plan()).await
    });
    use_effect(move || {
        if let Some(Ok(p)) = &*latest.read() {
            preview.set(Some(p.clone()));
        }
    });
    let error = use_memo(move || match &*latest.read() {
        Some(Err(e)) => Some(error_message(e)),
        _ => None,
    });
    (preview, error)
}

/// Seed of the random choices; empty uses the workspace's (`used` shows it).
#[component]
fn SeedField(plan: Signal<BalancePlan>, used: u64, #[props(into)] hint: String) -> Element {
    let mut plan = plan;
    let typed = plan.read().seed.map(|s| s.to_string()).unwrap_or_default();
    rsx! {
        div { class: "option-row",
            label { class: "field field-inline",
                span { class: "field-label",
                    "Seed "
                    InfoTip { label: "About the seed", text: "Using seed {used}; empty uses the workspace seed. {hint}" }
                }
                input {
                    class: "input input-narrow",
                    r#type: "number",
                    min: "0",
                    placeholder: "{used}",
                    value: "{typed}",
                    oninput: move |e| {
                        let text = e.value();
                        match text.trim().parse() {
                            Ok(n) => plan.write().seed = Some(n),
                            Err(_) if text.trim().is_empty() => plan.write().seed = None,
                            Err(_) => {}
                        }
                    },
                }
            }
        }
    }
}

/// Counts before and after, per class and split.
#[component]
fn ResultCard(preview: BalancePreview, #[props(into)] id: String) -> Element {
    let total = |counts: &[u32; 4]| counts.iter().sum::<u32>();
    let no_split = preview.images_after[3] > 0;
    let rows: Vec<_> = preview.classes.iter().filter(|c| total(&c.before) > 0).collect();
    let removed: Vec<String> = [
        (preview.removed_by_classes, "with their classes left out"),
        (preview.removed_by_filter, "by the class filter"),
        (preview.removed_by_subset, "by the subset"),
        (preview.removed_by_cap, "by the threshold"),
    ]
    .iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, what)| format!("{} {what}", format_count(*count)))
    .collect();
    let removed_text = format!("Images left out: {}.", removed.join(", "));
    // Classes a split with images has no labels of.
    let missing: Vec<String> = [(1, "val"), (2, "test")]
        .iter()
        .filter(|(slot, _)| preview.images_after[*slot] > 0)
        .flat_map(|(slot, split)| {
            rows.iter().filter(|c| total(&c.after) > 0 && c.after[*slot] == 0).map(move |c| format!("\"{}\" has no labels in {split}.", c.name))
        })
        .collect();

    rsx! {
        section { class: "section-card", aria_labelledby: "{id}",
            div { class: "section-card-header",
                h2 { id: "{id}", class: "section-card-title", TitleIcon { name: IconName::Chart } "Result" }
                span { class: "section-card-aside",
                    span { class: "section-card-meta",
                        "{format_count(preview.total_before())} → {format_count(preview.total_after())} images"
                    }
                    WarningTip { errors: preview.problems.clone(), warnings: missing }
                }
            }
            div { class: "table-scroll",
                table { class: "data-table result-table",
                    thead {
                        tr {
                            th { scope: "col", "Labels per class" }
                            th { class: "num", scope: "col", "Before" }
                            th { class: "num", scope: "col", "After" }
                            th { class: "num", scope: "col", "Train" }
                            th { class: "num", scope: "col", "Val" }
                            th { class: "num", scope: "col", "Test" }
                            if no_split {
                                th { class: "num", scope: "col", "No split" }
                            }
                        }
                    }
                    tbody {
                        tr { class: "total-row",
                            th { class: "name", scope: "row", "Images" }
                            Counts { before: total(&preview.images_before), after: preview.images_after, no_split }
                        }
                        for class in rows.iter() {
                            tr { key: "{class.name}",
                                th { class: "name", scope: "row", title: "{class.name}",
                                    "{class.name}"
                                    if total(&class.after) == 0 && preview.can_run() {
                                        span { class: "tag tag-warning", "gone" }
                                    }
                                }
                                Counts { before: total(&class.before), after: class.after, no_split }
                            }
                        }
                    }
                }
            }
            if !removed.is_empty() {
                p { class: "field-hint", "{removed_text}" }
            }
        }
    }
}

/// The number cells of a result row.
#[component]
fn Counts(before: u32, after: [u32; 4], no_split: bool) -> Element {
    let total: u32 = after.iter().sum();
    rsx! {
        td { class: "num muted", "{format_count(before)}" }
        td { class: "num", "{format_count(total)}" }
        for (slot, count) in after.iter().enumerate().take(if no_split { 4 } else { 3 }) {
            td { key: "{slot}", class: "num muted", if *count > 0 { "{format_count(*count)}" } else { "–" } }
        }
    }
}

/// Output choice and the button that starts a `kind` job for `plan`.
#[component]
fn RunCard(
    plan: BalancePlan,
    can_run: bool,
    /// Input dataset name.
    name: String,
    /// Ending of the suggested output name, e.g. "split".
    #[props(into)]
    suffix: String,
    kind: JobKind,
    /// Button text, e.g. "Split".
    #[props(into)]
    action: String,
) -> Element {
    let output = use_signal(OutputChoice::default);
    let mut submitted = use_signal(|| false);
    let mut form_error = use_signal(|| None::<String>);
    let (job, _) = use_job(kind);
    let running = job.read().as_ref().is_some_and(|j| !j.finished);
    let suggested = format!("{name}-{suffix}");

    let suggested_for_run = suggested.clone();
    let submit = move |e: FormEvent| {
        let plan = plan.clone();
        let suggested = suggested_for_run.clone();
        async move {
            e.prevent_default();
            submitted.set(true);
            form_error.set(None);
            let Ok(target) = output.read().resolve(&suggested, true) else { return };
            match start_balance(plan, target, kind).await {
                Ok(started) => {
                    set_if_newer(job, started);
                    submitted.set(false);
                }
                Err(e) => form_error.set(Some(error_message(&e))),
            }
        }
    };

    rsx! {
        form { class: "section-stack", novalidate: true, onsubmit: submit,
            OutputSelect {
                choice: output,
                suggested_name: suggested,
                source_name: Some(name),
                show_errors: submitted(),
                disabled: running,
                id: "{suffix}-output",
            }
            if let Some(message) = form_error() {
                p { class: "form-error", role: "alert", "{message}" }
            }
            div { class: "form-actions",
                button { class: "button", r#type: "submit", disabled: !can_run || running,
                    if running { "Working…" } else { "{action}" }
                }
                if !can_run {
                    span { class: "form-actions-note", "Resolve the problems in Result first." }
                }
            }
        }
        if let Some(current) = job() {
            JobProgress { job: current, title: "Writing", noun: "dataset", verb: "written" }
        }
    }
}
