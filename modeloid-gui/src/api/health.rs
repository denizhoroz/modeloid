//! Health Check: run the check (a job), read its report, apply fixes (a job),
//! and rescan datasets after manual edits (a job).

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

/// The last Health Check report of dataset `id`, if it was ever checked.
#[post("/api/health/report")]
pub async fn health_report(id: u32) -> Result<Option<HealthView>, ServerFnError> {
    backend::report(id).await.map_err(ServerFnError::new)
}

/// Starts checking dataset `id` (`JobKind::HealthCheck`).
#[post("/api/health/check")]
pub async fn start_health_check(id: u32) -> Result<Job, ServerFnError> {
    backend::check(id).map_err(ServerFnError::new)
}

/// Starts applying `fixes` to dataset `id` (`JobKind::HealthFix`).
#[post("/api/health/fix")]
pub async fn start_health_fix(id: u32, fixes: HealthFixes, output: OutputTarget) -> Result<Job, ServerFnError> {
    backend::fix(id, fixes, output).map_err(ServerFnError::new)
}

/// Rebuilds every dataset's `dataset.toml` and adds folders that have none
/// (`JobKind::Rescan`).
#[post("/api/datasets/rescan")]
pub async fn start_rescan() -> Result<Job, ServerFnError> {
    backend::rescan().map_err(ServerFnError::new)
}

#[cfg(feature = "server")]
mod backend {
    use modeloid::{Silent, Workspace};

    use super::{HealthFixes, HealthView, SHOWN};
    use crate::api::datasets::{resolve_output, OutputTarget};
    use crate::api::jobs::{self, Job, JobKind};
    use crate::api::{activity, workspace};

    fn open() -> Result<Workspace, String> {
        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))
    }

    fn name_of(ws: &Workspace, id: u32) -> Result<String, String> {
        ws.find_dataset(id).map(|(_, d)| d.name().to_string()).map_err(|e| e.to_string())
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

    pub fn check(id: u32) -> Result<Job, String> {
        let ws = open()?;
        let name = name_of(&ws, id)?;
        jobs::start(JobKind::HealthCheck, vec![name], move |ctx| {
            let report = ws.health_check(id, ctx.reporter().as_ref()).map_err(|e| e.to_string())?;
            let problems = report.counts().problems();
            Ok(Some(if problems == 0 { "No problems found".into() } else { format!("{problems} findings") }))
        })
    }

    pub fn fix(id: u32, fixes: HealthFixes, output: OutputTarget) -> Result<Job, String> {
        if !fixes.any() {
            return Err("Choose at least one fix.".into());
        }
        let ws = open()?;
        let name = name_of(&ws, id)?;
        let output = resolve_output(output, &[id])?;
        let (_, fresh) = ws.health_report(id).map_err(|e| e.to_string())?.ok_or("Run Health Check first.")?;
        if !fresh {
            return Err("The dataset changed since it was checked. Run Health Check again.".into());
        }
        jobs::start(JobKind::HealthFix, vec![name], move |ctx| {
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

        jobs::start(JobKind::Rescan, labels, move |ctx| match ids.get(ctx.index()) {
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
