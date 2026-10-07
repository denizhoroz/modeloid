//! Health Check: run the check (a job, one item per dataset), read reports,
//! apply fixes (a job, one item per dataset), and rescan datasets after
//! manual edits (a job).

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

pub use modeloid_types::health::{HealthCounts, HealthFixes, HealthReport};

use crate::api::datasets::OutputTarget;
use crate::api::jobs::Job;

/// Longest list per finding sent to the page; counts are always complete.
#[cfg_attr(not(feature = "server"), allow(dead_code))]
const SHOWN: usize = 200;

/// A report for the page: lists cut to `SHOWN`, full counts, and whether it
/// still matches the dataset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthView {
    pub report: HealthReport,
    pub counts: HealthCounts,
    pub fresh: bool,
}

/// A dataset's last check, without the lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checked {
    /// RFC 3339 time of the check.
    pub at: String,
    /// The dataset is unchanged since.
    pub fresh: bool,
    pub counts: HealthCounts,
}

/// One dataset in the health overview; `checked` is `None` until it was checked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthSummary {
    pub id: u32,
    pub name: String,
    pub checked: Option<Checked>,
}

/// The last Health Check report of dataset `id`, if it was ever checked.
#[post("/api/health/report")]
pub async fn health_report(id: u32) -> Result<Option<HealthView>, ServerFnError> {
    backend::report(id).await.map_err(ServerFnError::new)
}

/// Name and last check of each dataset in `ids`, in that order. Datasets that
/// no longer exist are left out.
#[post("/api/health/summaries")]
pub async fn health_summaries(ids: Vec<u32>) -> Result<Vec<HealthSummary>, ServerFnError> {
    backend::summaries(ids).await.map_err(ServerFnError::new)
}

/// Starts checking the datasets `ids`, one after another (`JobKind::HealthCheck`).
#[post("/api/health/check")]
pub async fn start_health_check(ids: Vec<u32>) -> Result<Job, ServerFnError> {
    crate::api::blocking(move || backend::check(ids)).await
}

/// Starts applying `fixes` to each dataset in `targets`, writing it to its
/// output (`JobKind::HealthFix`). A dataset that wasn't checked, changed
/// since, or has nothing these fixes apply to is skipped.
#[post("/api/health/fix")]
pub async fn start_health_fix(targets: Vec<(u32, OutputTarget)>, fixes: HealthFixes) -> Result<Job, ServerFnError> {
    crate::api::blocking(move || backend::fix(targets, fixes)).await
}

/// Rebuilds every dataset's `dataset.toml` and adds folders that have none
/// (`JobKind::Rescan`).
#[post("/api/datasets/rescan")]
pub async fn start_rescan() -> Result<Job, ServerFnError> {
    crate::api::blocking(backend::rescan).await
}

#[cfg(feature = "server")]
mod backend {
    use modeloid::{Silent, Workspace};

    use super::{Checked, HealthFixes, HealthSummary, HealthView, SHOWN};
    use crate::api::datasets::{resolve_output, OutputTarget};
    use crate::api::jobs::{self, Job, JobKind};
    use crate::api::activity;

    use crate::api::workspace::open;

    fn name_of(ws: &Workspace, id: u32) -> Result<String, String> {
        ws.find_dataset(id).map(|(_, d)| d.name().to_string()).map_err(|e| e.to_string())
    }

    /// Names of `ids`, or the first one that isn't a dataset.
    fn names_of(ws: &Workspace, ids: &[u32]) -> Result<Vec<String>, String> {
        if ids.is_empty() {
            return Err("Select at least one dataset.".into());
        }
        ids.iter().map(|&id| name_of(ws, id)).collect()
    }

    pub async fn report(id: u32) -> Result<Option<HealthView>, String> {
        activity::track("health-report", "Loading health report", move |ctx| {
            let ws = open()?;
            if let Ok(name) = name_of(&ws, id) {
                ctx.set_label(format!("Loading health report · {name}"));
            }
            let found = ws.health_report(id).map_err(|e| e.to_string())?;
            Ok(found.map(|(report, fresh)| HealthView { counts: report.counts(), report: report.truncated(SHOWN), fresh }))
        })
        .await
    }

    pub async fn summaries(ids: Vec<u32>) -> Result<Vec<HealthSummary>, String> {
        tokio::task::spawn_blocking(move || {
            let ws = open()?;
            let mut rows = Vec::new();
            for id in ids {
                let Ok(name) = name_of(&ws, id) else { continue };
                let found = ws.health_report(id).map_err(|e| e.to_string())?;
                let checked = found.map(|(report, fresh)| Checked { counts: report.counts(), at: report.checked_at, fresh });
                rows.push(HealthSummary { id, name, checked });
            }
            Ok(rows)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub fn check(ids: Vec<u32>) -> Result<Job, String> {
        let ws = open()?;
        let names = names_of(&ws, &ids)?;
        jobs::start(JobKind::HealthCheck, names, jobs::Touches::Datasets(ids.clone()), move |ctx| {
            let counts = ws.health_check(ids[ctx.index()], ctx.reporter().as_ref()).map_err(|e| e.to_string())?.counts();
            Ok(Some(match counts.problems() {
                0 => "No problems found".to_string(),
                n => format!("{}% healthy, {n} findings", counts.health_percent()),
            }))
        })
    }

    pub fn fix(targets: Vec<(u32, OutputTarget)>, fixes: HealthFixes) -> Result<Job, String> {
        if !fixes.any() {
            return Err("Choose at least one fix.".into());
        }
        let ws = open()?;
        let ids: Vec<u32> = targets.iter().map(|(id, _)| *id).collect();
        let names = names_of(&ws, &ids)?;
        // Catch bad output names now instead of as failed items.
        for (id, target) in &targets {
            resolve_output(target.clone(), &[*id], false)?;
        }

        jobs::start(JobKind::HealthFix, names, jobs::Touches::Datasets(ids), move |ctx| {
            let (id, target) = targets[ctx.index()].clone();
            let output = resolve_output(target, &[id], false)?;
            // Reasons to leave a dataset alone end the item as skipped, not failed.
            let skip = |why: &str| {
                ctx.skip();
                Err(why.to_string())
            };
            let Some((report, fresh)) = ws.health_report(id).map_err(|e| e.to_string())? else {
                return skip("Not checked yet.");
            };
            if !fresh {
                return skip("Changed since it was checked. Check it again first.");
            }
            let fixes = fixes.restricted_to(&report.counts());
            if !fixes.any() {
                return skip("Nothing to fix.");
            }
            let (dir, s) = ws.apply_health_fixes(id, &fixes, &output, ctx.reporter().as_ref()).map_err(|e| e.to_string())?;
            ctx.reporter().message(&format!(
                "{} images removed, {} objects dropped, {} boxes clipped",
                s.removed_images, s.dropped_objects, s.clipped_objects
            ));
            Ok(Some(dir.display().to_string()))
        })
    }

    pub fn rescan() -> Result<Job, String> {
        let ws = open()?;
        let datasets = ws.list_datasets().map_err(|e| e.to_string())?;
        let ids: Vec<u32> = datasets.iter().map(|(_, d)| d.id()).collect();
        let mut labels: Vec<String> = datasets.iter().map(|(_, d)| d.name().to_string()).collect();
        labels.push("New folders".into());

        jobs::start(JobKind::Rescan, labels, jobs::Touches::All, move |ctx| match ids.get(ctx.index()) {
            Some(&id) => {
                let progress = ctx.reporter().start("Scanning", modeloid::Unit::Files);
                let result = ws.rescan_dataset(id, progress.as_ref());
                progress.finish();
                let d = result.map_err(|e| e.to_string())?;
                Ok(Some(format!("{} images, {} labels", d.image_count(), d.label_count())))
            }
            None => {
                let added = ws.scan_datasets(&Silent).map_err(|e| e.to_string())?;
                Ok(Some(if added.is_empty() {
                    "none found".into()
                } else {
                    added.iter().map(|d| d.name().to_string()).collect::<Vec<_>>().join(", ")
                }))
            }
        })
    }
}
