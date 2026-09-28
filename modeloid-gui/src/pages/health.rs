//! Dataset Manager > Health Check: find broken images, bad boxes, unknown
//! classes, orphan label files and duplicates, then fix them into a new
//! dataset (or over the old one).

use dioxus::prelude::*;
use modeloid_types::health::{BoxIssue, DuplicateGroup, ImageIssue, ImageRef};
use modeloid_types::Split;

use crate::api::datasets::list_datasets;
use crate::api::error_message;
use crate::api::explore::thumb_url;
use crate::api::health::{health_report, start_health_check, start_health_fix, HealthFixes, HealthView};
use crate::api::jobs::JobKind;
use crate::components::{set_if_newer, use_job, ChooseDataset, JobProgress, OutputChoice, OutputSelect, StatTile};
use crate::routes::Route;
use crate::state::{use_datasets_version, use_selected_dataset, use_workspace, use_workspace_checked};
use crate::util::format_count;

#[component]
pub fn HealthCheck() -> Element {
    let workspace = use_workspace();
    let checked = use_workspace_checked();
    let selected = use_selected_dataset();

    rsx! {
        document::Title { "Health Check · Modeloid" }
        div { class: "page page-wide",
            p { class: "page-eyebrow", "Dataset Manager" }
            h1 { "Health Check" }
            p { class: "page-lead",
                "Finds corrupt images, broken boxes, unknown classes, label files without images and duplicate images, including duplicates that leak across splits."
            }
            match (workspace(), selected()) {
                (Some(_), Some(id)) => rsx! { HealthDataset { key: "{id}", id } },
                (Some(_), None) => rsx! { ChooseDataset {} },
                (None, _) if !checked() => rsx! {},
                (None, _) => rsx! {
                    div { class: "notice",
                        p { "Health Check looks at a workspace's datasets. Create one, or load one from the sidebar." }
                        Link { class: "button", to: Route::CreateWorkspace {}, "Create workspace" }
                    }
                },
            }
        }
    }
}

#[component]
fn HealthDataset(id: u32) -> Element {
    let mut selected = use_selected_dataset();
    let datasets_version = use_datasets_version();
    let (check_job, check_poll_error) = use_job(JobKind::HealthCheck);
    let (fix_job, _) = use_job(JobKind::HealthFix);
    let mut start_error = use_signal(|| None::<String>);

    // Re-read the report when a check finishes or datasets change.
    let check_finished = use_memo(move || check_job.read().as_ref().filter(|j| j.finished).map(|j| j.version));
    let view = use_resource(move || async move {
        let _ = (check_finished(), datasets_version());
        health_report(id).await
    });
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

    let running = check_job.read().as_ref().is_some_and(|j| !j.finished) || fix_job.read().as_ref().is_some_and(|j| !j.finished);
    let has_report = matches!(&*view.read(), Some(Ok(Some(_))));

    let run_check = move |_| async move {
        start_error.set(None);
        match start_health_check(id).await {
            Ok(job) => set_if_newer(check_job, job),
            Err(e) => start_error.set(Some(error_message(&e))),
        }
    };

    rsx! {
        div { class: "explore-head",
            div { class: "explore-title",
                h2 { class: "explore-name", "{name}" }
                if let Some(Ok(Some(v))) = &*view.read() {
                    span { class: "section-card-meta", "Checked {short_time(&v.report.checked_at)}" }
                }
            }
            div { class: "explore-actions",
                button {
                    class: "button button-sm",
                    r#type: "button",
                    disabled: running,
                    onclick: run_check,
                    if has_report { "Check again" } else { "Run Health Check" }
                }
                button { class: "button button-ghost button-sm", r#type: "button", onclick: move |_| selected.set(None), "Change dataset" }
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

        match &*view.read() {
            None => rsx! { p { class: "field-hint", "Loading…" } },
            Some(Err(e)) => rsx! { p { class: "form-error", role: "alert", "{error_message(e)}" } },
            Some(Ok(None)) => rsx! {
                div { class: "empty-state health-empty",
                    p { "This dataset hasn't been checked yet." }
                    p { class: "field-hint", "Every image is decoded, so large datasets take a while. You can keep working meanwhile." }
                }
            },
            Some(Ok(Some(v))) => rsx! {
                Report { id, name: name.clone(), view: v.clone(), fix_job_running: fix_job.read().as_ref().is_some_and(|j| !j.finished) }
            },
        }

        if let Some(job) = fix_job() {
            JobProgress { job, title: "Fixing", noun: "dataset", verb: "fixed" }
        }
    }
}

#[component]
fn Report(id: u32, name: String, view: HealthView, fix_job_running: bool) -> Element {
    let HealthView { report, counts, fresh } = view;
    // Previews only line up with a report that matches the dataset.
    let thumbs = fresh;
    let classes = report.classes.clone();

    rsx! {
        if !fresh {
            div { class: "notice notice-warning", role: "status",
                p { "The dataset changed since this check. Check again before fixing." }
            }
        }

        div { class: "stat-tiles",
            StatTile { label: "Images checked", value: format_count(report.image_count) }
            StatTile {
                label: "Problems",
                value: format_count(counts.problems() as u32),
                detail: if counts.problems() == 0 { Some("all clear".to_string()) } else { None },
            }
            StatTile { label: "Duplicate groups", value: format_count((counts.exact_groups + counts.near_groups) as u32), detail: format!("{} across splits", counts.leak_groups) }
            StatTile { label: "Without labels", value: format_count(counts.unlabeled_images as u32), detail: "images".to_string() }
        }

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

        if counts.problems() > 0 || counts.unlabeled_images > 0 {
            // Keyed by check time: a new report starts from its own recommended fixes.
            FixForm { key: "{report.checked_at}", id, name, counts: counts.clone(), fresh, disabled: fix_job_running }
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

/// Choose fixes and where the fixed dataset goes.
#[component]
fn FixForm(id: u32, name: String, counts: modeloid_types::health::HealthCounts, fresh: bool, disabled: bool) -> Element {
    let recommended = HealthFixes::recommended(&counts);
    let mut fixes = use_signal(move || recommended);
    let output = use_signal(OutputChoice::default);
    let mut submitted = use_signal(|| false);
    let mut form_error = use_signal(|| None::<String>);
    let (fix_job, _) = use_job(JobKind::HealthFix);
    let suggested = format!("{name}-fixed");

    let options: Vec<(&'static str, String, usize, fn(&mut HealthFixes) -> &mut bool)> = vec![
        ("Remove corrupt images", "can't be decoded".into(), counts.corrupt_images, |f| &mut f.remove_corrupt),
        ("Remove duplicates across splits", "keeps the training copy".into(), counts.leak_groups, |f| &mut f.remove_leaks),
        ("Remove identical copies", "keeps one per group".into(), counts.exact_groups, |f| &mut f.remove_exact_duplicates),
        ("Remove look-alikes", "keeps one per group; check the list first".into(), counts.near_groups, |f| &mut f.remove_near_duplicates),
        ("Drop empty or zero-area boxes", String::new(), counts.degenerate_boxes, |f| &mut f.drop_degenerate),
        ("Clip boxes to the image", String::new(), counts.out_of_bounds, |f| &mut f.clip_out_of_bounds),
        ("Drop objects with unknown classes", String::new(), counts.unknown_classes, |f| &mut f.drop_unknown_classes),
        ("Remove images without labels", "only if you don't want background images".into(), counts.unlabeled_images, |f| &mut f.remove_unlabeled),
    ];

    let suggested_for_submit = suggested.clone();
    let submit = move |e: FormEvent| {
        let suggested = suggested_for_submit.clone();
        async move {
            e.prevent_default();
            submitted.set(true);
            form_error.set(None);
            let Ok(target) = output.read().resolve(&suggested, true) else { return };
            match start_health_fix(id, fixes(), target).await {
                Ok(job) => {
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
                h2 { id: "fix-title", class: "section-card-title", "Fix" }
                p { class: "field-hint",
                    "Fixing writes the dataset again from its images and labels, in the same format. Label files without images and other files (READMEs, extra exports) are left out."
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
            OutputSelect { choice: output, suggested_name: suggested, source_name: Some(name), show_errors: submitted(), disabled }

            if let Some(message) = form_error() {
                p { class: "form-error", role: "alert", "{message}" }
            }
            div { class: "form-actions",
                button {
                    class: "button",
                    r#type: "submit",
                    disabled: disabled || !fresh || !fixes().any(),
                    if disabled { "Fixing…" } else { "Apply fixes" }
                }
                if !fresh {
                    span { class: "form-actions-note", "Check again first: the dataset changed." }
                } else if !fixes().any() {
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
