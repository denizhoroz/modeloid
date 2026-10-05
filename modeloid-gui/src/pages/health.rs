//! Dataset Manager > Health Check: find broken images, bad boxes, unknown
//! classes, orphan label files and duplicates in the selected datasets, then
//! fix them into new datasets (or over the old ones). One dataset or several:
//! checking and fixing run as one job with an item per dataset.

use dioxus::prelude::*;
use modeloid_types::health::{BoxIssue, DuplicateGroup, HealthCounts, ImageIssue, ImageRef};
use modeloid_types::Split;

use crate::api::datasets::{validate_dataset_name, OutputTarget};
use crate::components::{IconName, TitleIcon};
use crate::api::error_message;
use crate::api::explore::thumb_url;
use crate::api::health::{health_report, health_summaries, start_health_check, start_health_fix, HealthFixes, HealthSummary, HealthView};
use crate::api::jobs::JobKind;
use crate::components::{set_if_newer, use_job, InfoTip, JobProgress, NoWorkspace, OutputChoice, OutputSelect, SelectDatasetHint};
use crate::state::{use_datasets_version, use_selected_datasets, use_workspace, use_workspace_checked};
use crate::util::{format_count, plural};

#[component]
pub fn HealthCheck() -> Element {
    let workspace = use_workspace();
    let checked = use_workspace_checked();
    let selected = use_selected_datasets();
    // A new selection is a new page: its own report, forms and job history.
    let key = selected.read().iter().map(u32::to_string).collect::<Vec<_>>().join("-");

    rsx! {
        document::Title { "Health Check · Modeloid" }
        div { class: "page page-wide",
            p { class: "page-eyebrow", "Dataset Manager" }
            h1 { "Health Check" }
            p { class: "page-lead",
                "Finds corrupt images, broken boxes, unknown classes, label files without images and duplicate images, including duplicates that leak across splits. Select several datasets to check and fix them together."
            }
            match workspace() {
                Some(_) if !selected.read().is_empty() => rsx! { HealthPage { key: "{key}", ids: selected() } },
                Some(_) => rsx! { SelectDatasetHint {} },
                None if !checked() => rsx! {},
                None => rsx! { NoWorkspace { text: "Health Check looks at a workspace's datasets." } },
            }
        }
    }
}

/// Health of the selected datasets `ids` (selection order).
#[component]
fn HealthPage(ids: Vec<u32>) -> Element {
    let datasets_version = use_datasets_version();
    let (check_job, check_poll_error) = use_job(JobKind::HealthCheck);
    let (fix_job, _) = use_job(JobKind::HealthFix);
    let mut start_error = use_signal(|| None::<String>);
    // A finished fix job is only shown when it was started here, for this
    // selection: `use_job` also knows the last job of any earlier selection.
    let fix_started_here = use_signal(|| false);
    // The dataset whose report is shown in full: the one selected last, until another row is picked.
    let focus = use_signal(|| ids.last().copied().unwrap_or_default());
    let many = ids.len() > 1;

    // Re-read when a check or a fix finishes, or datasets change.
    let jobs_finished = use_memo(move || {
        let version = |job: Signal<Option<crate::api::jobs::Job>>| job.read().as_ref().filter(|j| j.finished).map(|j| j.version);
        (version(check_job), version(fix_job))
    });
    let ids_for_summaries = ids.clone();
    let summaries = use_resource(move || {
        let ids = ids_for_summaries.clone();
        async move {
            let _ = (jobs_finished(), datasets_version());
            health_summaries(ids).await
        }
    });
    let view = use_resource(move || async move {
        let _ = (jobs_finished(), datasets_version());
        health_report(focus()).await
    });

    let rows: Vec<HealthSummary> = summaries.read().as_ref().and_then(|r| r.as_ref().ok()).cloned().unwrap_or_default();
    let name_of = |id: u32| rows.iter().find(|r| r.id == id).map_or(format!("Dataset #{id}"), |r| r.name.clone());
    let checking = check_job.read().as_ref().is_some_and(|j| !j.finished);
    let fixing = fix_job.read().as_ref().is_some_and(|j| !j.finished);
    let any_checked = rows.iter().any(|r| r.checked.is_some());
    let focus_name = name_of(focus());

    let ids_for_check = ids.clone();
    let run_check = move |_| {
        let ids = ids_for_check.clone();
        async move {
            start_error.set(None);
            match start_health_check(ids).await {
                Ok(job) => set_if_newer(check_job, job),
                Err(e) => start_error.set(Some(error_message(&e))),
            }
        }
    };

    // Datasets a fix can run on: checked, and unchanged since.
    let fixable: Vec<HealthSummary> = rows.iter().filter(|r| r.checked.as_ref().is_some_and(|c| c.fresh)).cloned().collect();
    let total = HealthCounts::sum(fixable.iter().filter_map(|r| r.checked.as_ref().map(|c| &c.counts)));
    // Keyed by the check times: new reports start from their own recommended fixes.
    let fix_key: String = fixable.iter().filter_map(|r| r.checked.as_ref().map(|c| c.at.as_str())).collect::<Vec<_>>().join("|");

    rsx! {
        div { class: "explore-head",
            div { class: "explore-title",
                h2 { class: "explore-name", if many { "{ids.len()} datasets" } else { "{focus_name}" } }
                if !many {
                    if let Some(Ok(Some(v))) = &*view.read() {
                        span { class: "section-card-meta", "Checked {short_time(&v.report.checked_at)}" }
                    }
                }
            }
            div { class: "explore-actions",
                button {
                    class: "button button-sm",
                    r#type: "button",
                    disabled: checking || fixing,
                    onclick: run_check,
                    if many {
                        if any_checked { "Check all again" } else { "Check all" }
                    } else if any_checked {
                        "Check again"
                    } else {
                        "Run Health Check"
                    }
                }
            }
        }

        if let Some(message) = start_error() {
            p { class: "form-error", role: "alert", "{message}" }
        }
        if let Some(message) = check_poll_error() {
            p { class: "form-error", role: "alert", "Lost track of the check: {message}. Reload the page." }
        }
        if let Some(job) = check_job().filter(|j| !j.finished) {
            JobProgress { job, title: "Checking", noun: "dataset", verb: "checked", output_label: "Result:" }
        }

        if many {
            SummaryTable { rows: rows.clone(), ids: ids.clone(), focus }
        }

        match &*view.read() {
            None => rsx! { p { class: "field-hint", "Loading…" } },
            Some(Err(e)) => rsx! { p { class: "form-error", role: "alert", "{error_message(e)}" } },
            Some(Ok(None)) => rsx! {
                div { class: "empty-state health-empty",
                    p { if many { "{focus_name} hasn't been checked yet." } else { "This dataset hasn't been checked yet." } }
                    p { class: "field-hint", "Every image is decoded, so large datasets take a while. You can keep working meanwhile." }
                }
            },
            Some(Ok(Some(v))) => rsx! {
                Report { id: focus(), name: many.then(|| focus_name.clone()), view: v.clone() }
            },
        }

        if total.problems() > 0 || total.unlabeled_images > 0 {
            FixForm { key: "{fix_key}", datasets: fixable, total, disabled: fixing || checking, started: fix_started_here }
        }

        if let Some(job) = fix_job().filter(|j| !j.finished || fix_started_here()) {
            JobProgress { job, title: "Fixing", noun: "dataset", verb: "fixed" }
        }
    }
}

/// The selected datasets with their health; a row click shows that dataset's report.
#[component]
fn SummaryTable(rows: Vec<HealthSummary>, ids: Vec<u32>, focus: Signal<u32>) -> Element {
    let mut focus = focus;
    // Selection order; a dataset whose summary hasn't arrived yet still gets a row.
    let ordered: Vec<HealthSummary> = ids
        .iter()
        .map(|&id| rows.iter().find(|r| r.id == id).cloned().unwrap_or(HealthSummary { id, name: format!("Dataset #{id}"), checked: None }))
        .collect();
    let checked: Vec<&HealthCounts> = ordered.iter().filter_map(|r| r.checked.as_ref().map(|c| &c.counts)).collect();
    let overall = HealthCounts::sum(checked.iter().copied());

    rsx! {
        section { class: "section-card", aria_labelledby: "health-datasets-title",
            div { class: "section-card-header",
                h2 { id: "health-datasets-title", class: "section-card-title", TitleIcon { name: IconName::Database } "Datasets" }
                span { class: "section-card-meta",
                    if checked.is_empty() {
                        "none checked yet"
                    } else {
                        "{checked.len()} of {ordered.len()} checked · {overall.health_percent()}% healthy overall"
                    }
                }
            }
            div { class: "table-scroll",
                table { class: "data-table health-table",
                    thead {
                        tr {
                            th { scope: "col", "Dataset" }
                            th { class: "num", scope: "col", "Health" }
                            th { class: "num", scope: "col", "Problems" }
                            th { class: "num", scope: "col", "Images" }
                            th { scope: "col", "Checked" }
                        }
                    }
                    tbody {
                        for row in ordered.iter() {
                            {
                                let id = row.id;
                                let is_focus = focus() == id;
                                rsx! {
                                    tr { key: "{id}", class: if is_focus { "is-selected" }, onclick: move |_| focus.set(id),
                                        th { class: "name", scope: "row",
                                            button {
                                                class: "row-pick",
                                                r#type: "button",
                                                title: "Show the report of {row.name}",
                                                aria_pressed: if is_focus { "true" } else { "false" },
                                                onclick: move |_| focus.set(id),
                                                "{row.name}"
                                            }
                                        }
                                        match &row.checked {
                                            Some(c) => rsx! {
                                                td { class: "num", "{c.counts.health_percent()}%" }
                                                td { class: "num muted", "{format_count(c.counts.problems() as u32)}" }
                                                td { class: "num muted", "{format_count(c.counts.images as u32)}" }
                                                td { class: "muted",
                                                    "{short_time(&c.at)}"
                                                    if !c.fresh {
                                                        span { class: "tag tag-warning", "changed since" }
                                                    }
                                                }
                                            },
                                            None => rsx! {
                                                td { class: "num muted", "–" }
                                                td { class: "num muted", "–" }
                                                td { class: "num muted", "–" }
                                                td { class: "muted", "not checked" }
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The big number: the share of images without a problem.
#[component]
fn HealthScore(counts: HealthCounts, checked: u32) -> Element {
    let percent = counts.health_percent();
    let (level, word) = match percent {
        100 => ("is-good", "No problems found"),
        95..=99 => ("is-good", "Healthy"),
        80..=94 => ("is-fair", "Needs a look"),
        _ => ("is-poor", "Needs fixing"),
    };
    let detail = match counts.affected_images {
        0 if counts.problems() == 0 => format!("All {} images are fine.", format_count(counts.images as u32)),
        0 => "No image is affected, but there are findings below.".to_string(),
        n => format!("{} of {} images have a problem.", format_count(n as u32), format_count(counts.images as u32)),
    };

    rsx! {
        div { class: "health-score {level}", role: "group", aria_label: "Overall health",
            div { class: "health-score-main",
                span { class: "health-score-value", "{percent}%" }
                dl { class: "health-score-facts",
                    div { dt { "Images checked" } dd { "{format_count(checked)}" } }
                    div { dt { "Problems" } dd { "{format_count(counts.problems() as u32)}" } }
                }
            }
            div { class: "health-score-text",
                span { class: "health-score-label", "Health · {word}" }
                span { class: "health-score-detail", "{detail}" }
            }
        }
    }
}

/// One dataset's report: score, tiles and findings. `name` heads it when
/// several datasets are selected.
#[component]
fn Report(id: u32, name: Option<String>, view: HealthView) -> Element {
    let HealthView { report, counts, fresh } = view;
    // Previews only line up with a report that matches the dataset.
    let thumbs = fresh;
    let classes = report.classes.clone();

    rsx! {
        if let Some(name) = &name {
            div { class: "explore-head health-report-head",
                div { class: "explore-title",
                    h2 { class: "explore-name", "{name}" }
                    span { class: "section-card-meta", "Checked {short_time(&report.checked_at)}" }
                }
            }
        }
        if !fresh {
            div { class: "notice notice-warning", role: "status",
                p { "The dataset changed since this check. Check again before fixing." }
            }
        }

        HealthScore { counts: counts.clone(), checked: report.image_count }

        div { class: "findings",
            Finding { title: "Corrupt or unreadable images", count: counts.corrupt_images, problem: true,
                hint: "These files can't be decoded; training would fail or skip them.",
                ImageIssues { id, items: report.corrupt_images.clone(), total: counts.corrupt_images, thumbs: false }
            }
            Finding { title: "Duplicates across splits", count: counts.leak_groups, problem: true,
                hint: "The same image in training and evaluation splits makes scores look better than they are.",
                Duplicates { id, groups: report.duplicates.iter().filter(|g| g.crosses_splits()).cloned().collect::<Vec<_>>(), total: counts.leak_groups, thumbs }
            }
            Finding { title: "Identical images", count: counts.exact_groups, problem: true,
                hint: "Byte-for-byte copies. Groups also listed above appear here too.",
                Duplicates { id, groups: report.duplicates.iter().filter(|g| g.exact).cloned().collect::<Vec<_>>(), total: counts.exact_groups, thumbs }
            }
            Finding { title: "Look-alike images", count: counts.near_groups, problem: true,
                hint: "Nearly the same picture (resized, re-encoded or a burst shot). Worth a look before removing.",
                Duplicates { id, groups: report.duplicates.iter().filter(|g| !g.exact).cloned().collect::<Vec<_>>(), total: counts.near_groups, thumbs }
            }
            Finding { title: "Empty or zero-area boxes", count: counts.degenerate_boxes, problem: true,
                hint: "Boxes without width or height teach the model nothing.",
                BoxIssues { id, items: report.degenerate_boxes.clone(), total: counts.degenerate_boxes, classes: classes.clone(), thumbs }
            }
            Finding { title: "Boxes outside the image", count: counts.out_of_bounds, problem: true,
                hint: "Boxes reaching past the image edge; usually a conversion or rounding error.",
                BoxIssues { id, items: report.out_of_bounds.clone(), total: counts.out_of_bounds, classes: classes.clone(), thumbs }
            }
            Finding { title: "Unknown class ids", count: counts.unknown_classes, problem: true,
                hint: "Objects whose class id isn't in data.yaml's class list.",
                BoxIssues { id, items: report.unknown_classes.clone(), total: counts.unknown_classes, classes: classes.clone(), thumbs }
            }
            Finding { title: "Image size differs from labels", count: counts.size_mismatches, problem: true,
                hint: "The annotation file assumes another image size, so boxes may be misplaced. Not fixed automatically.",
                ImageIssues { id, items: report.size_mismatches.clone(), total: counts.size_mismatches, thumbs }
            }
            Finding { title: "Label files without images", count: counts.orphan_labels, problem: true,
                hint: "Labels whose image is missing. A fix leaves them out.",
                Paths { items: report.orphan_labels.clone(), total: counts.orphan_labels }
            }
            Finding { title: "Unreadable label lines", count: counts.label_warnings, problem: true,
                hint: "Lines or entries that couldn't be parsed and were skipped.",
                Paths { items: report.label_warnings.clone(), total: counts.label_warnings }
            }
            Finding { title: "Images without labels", count: counts.unlabeled_images, problem: false,
                hint: "Background images can help against false positives, so these are kept unless you choose to remove them.",
                ImageList { id, items: report.unlabeled_images.clone(), total: counts.unlabeled_images, thumbs }
            }
        }
    }
}

/// One kind of finding: a collapsible card with its count.
#[component]
fn Finding(#[props(into)] title: String, count: usize, problem: bool, #[props(into)] hint: String, children: Element) -> Element {
    let status = match (count, problem) {
        (0, _) => "finding is-clear",
        (_, true) => "finding is-problem",
        (_, false) => "finding is-info",
    };
    rsx! {
        details { class: "{status}", open: count > 0 && problem,
            summary { class: "finding-summary",
                span { class: "finding-count", "{format_count(count as u32)}" }
                span { class: "finding-title", "{title}" }
            }
            if count > 0 {
                p { class: "field-hint", "{hint}" }
                {children}
            } else {
                p { class: "field-hint", "None found." }
            }
        }
    }
}

fn split_name(split: Option<Split>) -> &'static str {
    Split::label(split)
}

#[component]
fn More(shown: usize, total: usize) -> Element {
    rsx! {
        if total > shown {
            p { class: "field-hint", "…and {format_count((total - shown) as u32)} more." }
        }
    }
}

#[component]
fn Thumb(id: u32, image: ImageRef, show: bool) -> Element {
    rsx! {
        if show {
            img { class: "finding-thumb", src: thumb_url(id, image.index), alt: "", loading: "lazy" }
        }
    }
}

#[component]
fn ImageIssues(id: u32, items: Vec<ImageIssue>, total: usize, thumbs: bool) -> Element {
    let shown = items.len();
    rsx! {
        ul { class: "finding-list",
            for item in items {
                li { key: "{item.image.path}",
                    Thumb { id, image: item.image.clone(), show: thumbs }
                    span { class: "finding-item",
                        code { "{item.image.path}" }
                        span { class: "finding-detail", "{item.detail}" }
                    }
                }
            }
        }
        More { shown, total }
    }
}

#[component]
fn ImageList(id: u32, items: Vec<ImageRef>, total: usize, thumbs: bool) -> Element {
    let shown = items.len();
    rsx! {
        ul { class: "finding-list finding-grid",
            for image in items {
                li { key: "{image.path}", title: "{image.path}",
                    Thumb { id, image: image.clone(), show: thumbs }
                    span { class: "finding-item",
                        code { "{image.path}" }
                        span { class: "finding-detail", "{split_name(image.split)}" }
                    }
                }
            }
        }
        More { shown, total }
    }
}

#[component]
fn BoxIssues(id: u32, items: Vec<BoxIssue>, total: usize, classes: Vec<String>, thumbs: bool) -> Element {
    let shown = items.len();
    let describe = |item: &BoxIssue| {
        let class = classes.get(item.class).cloned().unwrap_or_else(|| format!("class {}", item.class));
        format!("object {} ({class}): {}", item.object + 1, item.detail)
    };
    rsx! {
        ul { class: "finding-list",
            for item in items.iter() {
                li { key: "{item.image.path}-{item.object}",
                    Thumb { id, image: item.image.clone(), show: thumbs }
                    span { class: "finding-item",
                        code { "{item.image.path}" }
                        span { class: "finding-detail", {describe(item)} }
                    }
                }
            }
        }
        More { shown, total }
    }
}

#[component]
fn Paths(items: Vec<String>, total: usize) -> Element {
    let shown = items.len();
    rsx! {
        ul { class: "finding-list finding-paths",
            for (i, item) in items.iter().enumerate() {
                li { key: "{i}", code { "{item}" } }
            }
        }
        More { shown, total }
    }
}

#[component]
fn Duplicates(id: u32, groups: Vec<DuplicateGroup>, total: usize, thumbs: bool) -> Element {
    let shown = groups.len();
    rsx! {
        ul { class: "finding-list",
            for (g, group) in groups.iter().enumerate() {
                li { key: "{g}", class: "duplicate-group",
                    div { class: "duplicate-tags",
                        span { class: "tag", if group.exact { "identical" } else { "look-alike" } }
                        if group.crosses_splits() {
                            span { class: "tag tag-warning", "across splits" }
                        }
                    }
                    div { class: "duplicate-members",
                        for (i, image) in group.images.iter().enumerate() {
                            div { key: "{image.path}", class: "duplicate-member", title: "{image.path}",
                                Thumb { id, image: image.clone(), show: thumbs }
                                span { class: "finding-detail",
                                    if i == 0 { "kept · " } else { "" }
                                    "{split_name(image.split)}"
                                }
                                code { class: "duplicate-path", "{image.path}" }
                            }
                        }
                    }
                }
            }
        }
        More { shown, total }
    }
}

/// Choose fixes and where the fixed datasets go. `datasets` are the ones a
/// fix can run on (checked, unchanged since); `total` their findings together.
#[component]
fn FixForm(datasets: Vec<HealthSummary>, total: HealthCounts, disabled: bool, started: Signal<bool>) -> Element {
    let mut started = started;
    let recommended = HealthFixes::recommended(&total);
    let mut fixes = use_signal(move || recommended);
    // One dataset: a name of its own, or overwrite it.
    let output = use_signal(OutputChoice::default);
    // Several: each new name is the old one plus this ending, or overwrite them all.
    let mut ending = use_signal(|| "-fixed".to_string());
    let mut overwrite_all = use_signal(|| false);
    let mut confirmed = use_signal(|| false);
    let mut submitted = use_signal(|| false);
    let mut form_error = use_signal(|| None::<String>);
    let (fix_job, _) = use_job(JobKind::HealthFix);

    let many = datasets.len() > 1;
    let first_name = datasets.first().map(|d| d.name.clone()).unwrap_or_default();
    let suggested = format!("{first_name}-fixed");

    let options: Vec<(&'static str, String, usize, fn(&mut HealthFixes) -> &mut bool)> = vec![
        ("Remove corrupt images", "can't be decoded".into(), total.corrupt_images, |f| &mut f.remove_corrupt),
        ("Remove duplicates across splits", "keeps the training copy".into(), total.leak_groups, |f| &mut f.remove_leaks),
        ("Remove identical copies", "keeps one per group".into(), total.exact_groups, |f| &mut f.remove_exact_duplicates),
        ("Remove look-alikes", "keeps one per group; check the list first".into(), total.near_groups, |f| &mut f.remove_near_duplicates),
        ("Drop empty or zero-area boxes", String::new(), total.degenerate_boxes, |f| &mut f.drop_degenerate),
        ("Clip boxes to the image", String::new(), total.out_of_bounds, |f| &mut f.clip_out_of_bounds),
        ("Drop objects with unknown classes", String::new(), total.unknown_classes, |f| &mut f.drop_unknown_classes),
        ("Remove images without labels", "only if you don't want background images".into(), total.unlabeled_images, |f| &mut f.remove_unlabeled),
    ];

    // Where each dataset's result goes, or what is wrong with the choice.
    let datasets_for_targets = datasets.clone();
    let suggested_for_targets = suggested.clone();
    let targets = move || -> Result<Vec<(u32, OutputTarget)>, String> {
        if !many {
            let target = output.read().resolve(&suggested_for_targets, true)?;
            return Ok(datasets_for_targets.iter().map(|d| (d.id, target.clone())).collect());
        }
        if overwrite_all() {
            if !confirmed() {
                return Err("Confirm that the source datasets will be replaced.".into());
            }
            return Ok(datasets_for_targets.iter().map(|d| (d.id, OutputTarget::Overwrite)).collect());
        }
        let ending = ending();
        if ending.trim().is_empty() {
            return Err("Enter an ending for the new names, e.g. -fixed.".into());
        }
        datasets_for_targets
            .iter()
            .map(|d| {
                let name = format!("{}{}", d.name, ending.trim());
                validate_dataset_name(&name).map(|()| (d.id, OutputTarget::New { name }))
            })
            .collect()
    };
    let output_error = if many && submitted() { targets().err() } else { None };

    let submit = move |e: FormEvent| {
        let targets = targets();
        async move {
            e.prevent_default();
            submitted.set(true);
            form_error.set(None);
            let Ok(targets) = targets else { return };
            match start_health_fix(targets, fixes()).await {
                Ok(job) => {
                    started.set(true);
                    set_if_newer(fix_job, job);
                    submitted.set(false);
                }
                Err(e) => form_error.set(Some(error_message(&e))),
            }
        }
    };

    rsx! {
        form { class: "section-stack health-fix", novalidate: true, onsubmit: submit,
            section { class: "section-card", aria_labelledby: "fix-title",
                div { class: "section-card-header",
                    h2 { id: "fix-title", class: "section-card-title", TitleIcon { name: IconName::Wrench } "Fix" }
                    if many {
                        span { class: "section-card-meta", "{plural(datasets.len(), \"checked dataset\")} · counts are for all of them" }
                    }
                }
                p { class: "field-hint",
                    "Fixing writes a dataset again from its images and labels, in the same format. Label files without images and other files (READMEs, extra exports) are left out."
                    if many { " A dataset with nothing to fix is skipped." }
                }
                div { class: "fix-options",
                    for (label, detail, count, field) in options {
                        {
                            // Read from a copy: writing the signal during render would loop.
                            let on = count > 0 && *field(&mut fixes());
                            let note = if detail.is_empty() {
                                format!("({})", format_count(count as u32))
                            } else {
                                format!("({}; {detail})", format_count(count as u32))
                            };
                            rsx! {
                                label { key: "{label}", class: if count == 0 { "check is-disabled" } else { "check" },
                                    input {
                                        r#type: "checkbox",
                                        disabled: count == 0 || disabled,
                                        checked: on,
                                        onchange: move |e| *field(&mut fixes.write()) = e.checked(),
                                    }
                                    span {
                                        "{label} "
                                        span { class: "finding-detail", "{note}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if many {
                fieldset { class: "section-card", disabled,
                    legend { class: "section-card-title", TitleIcon { name: IconName::Save } "Output" }
                    div { class: "choice-list",
                        label { class: if !overwrite_all() { "choice is-selected" } else { "choice" },
                            input { r#type: "radio", name: "health-output", checked: !overwrite_all(), onchange: move |_| overwrite_all.set(false) }
                            span { class: "choice-body",
                                span { class: "choice-label",
                                    "Save as new datasets "
                                    InfoTip { label: "About saving as new", text: "The sources stay as they are." }
                                }
                            }
                        }
                        label { class: if overwrite_all() { "choice is-selected" } else { "choice" },
                            input { r#type: "radio", name: "health-output", checked: overwrite_all(), onchange: move |_| overwrite_all.set(true) }
                            span { class: "choice-body",
                                span { class: "choice-label", "Overwrite sources" }
                                span { class: "choice-detail", "Replaces the {datasets.len()} checked datasets." }
                            }
                        }
                    }
                    if overwrite_all() {
                        label { class: "check",
                            input { r#type: "checkbox", checked: confirmed(), onchange: move |e| confirmed.set(e.checked()) }
                            span { "I understand these " strong { "{datasets.len()} datasets" } " will be replaced. This can't be undone." }
                        }
                    } else {
                        div { class: "field",
                            label { class: "field-label", r#for: "health-ending", "Name ending" }
                            input {
                                id: "health-ending",
                                class: "input",
                                r#type: "text",
                                spellcheck: false,
                                autocomplete: "off",
                                value: "{ending}",
                                aria_invalid: if output_error.is_some() { "true" },
                                oninput: move |e| ending.set(e.value()),
                            }
                            p { class: "field-hint", "Each new dataset is named after its source plus this ending, e.g. {first_name}{ending}." }
                        }
                    }
                    if let Some(message) = &output_error {
                        p { class: "field-error", role: "alert", "{message}" }
                    }
                }
            } else {
                OutputSelect { choice: output, suggested_name: suggested, source_name: Some(first_name), show_errors: submitted(), disabled }
            }

            if let Some(message) = form_error() {
                p { class: "form-error", role: "alert", "{message}" }
            }
            div { class: "form-actions",
                button {
                    class: "button",
                    r#type: "submit",
                    disabled: disabled || !fixes().any(),
                    if disabled { "Working…" } else if many { "Apply fixes to {datasets.len()} datasets" } else { "Apply fixes" }
                }
                if !fixes().any() {
                    span { class: "form-actions-note", "Choose at least one fix." }
                } else {
                    span { class: "form-actions-note",
                        match count_selected(&fixes()) {
                            1 => "1 fix selected".to_string(),
                            n => format!("{n} fixes selected"),
                        }
                    }
                }
            }
        }
    }
}

fn count_selected(f: &HealthFixes) -> usize {
    [
        f.remove_corrupt,
        f.remove_leaks,
        f.remove_exact_duplicates,
        f.remove_near_duplicates,
        f.drop_degenerate,
        f.clip_out_of_bounds,
        f.drop_unknown_classes,
        f.remove_unlabeled,
    ]
    .iter()
    .filter(|on| **on)
    .count()
}

/// "2026-09-28 14:03" from an RFC 3339 time.
fn short_time(rfc3339: &str) -> String {
    rfc3339.get(..16).map(|s| s.replace('T', " ")).unwrap_or_else(|| rfc3339.to_string())
}
