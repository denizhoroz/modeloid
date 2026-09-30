//! Split & Balance: preview a plan on a dataset's labels and run it
//! (`JobKind::Split` for the Split tab, `JobKind::Balance` for Balance).

use dioxus::prelude::*;

pub use modeloid_types::balance::{BalancePlan, BalancePreview, Resplit};

use crate::api::datasets::OutputTarget;
use crate::api::jobs::{Job, JobKind};

/// What `plan` does to its input, counted exactly (fast once its labels are read).
#[post("/api/balance/preview")]
pub async fn balance_preview(plan: BalancePlan) -> Result<BalancePreview, ServerFnError> {
    backend::preview(plan).await.map_err(ServerFnError::new)
}

/// Starts writing the split or balanced dataset as a `kind` job
/// (`JobKind::Split` or `JobKind::Balance`).
#[post("/api/balance/run")]
pub async fn start_balance(plan: BalancePlan, output: OutputTarget, kind: JobKind) -> Result<Job, ServerFnError> {
    backend::run(plan, output, kind).map_err(ServerFnError::new)
}

#[cfg(feature = "server")]
mod backend {
    use modeloid::Workspace;

    use super::{BalancePlan, BalancePreview, Resplit};
    use crate::api::datasets::{resolve_output, OutputTarget};
    use crate::api::explore::cached_labels;
    use crate::api::jobs::{self, Job, JobKind};
    use crate::api::{activity, workspace};

    fn open() -> Result<Workspace, String> {
        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))
    }

    fn name_of(ws: &Workspace, id: u32) -> Result<String, String> {
        ws.find_dataset(id).map(|(_, d)| d.name().to_string()).map_err(|e| e.to_string())
    }

    pub async fn preview(plan: BalancePlan) -> Result<BalancePreview, String> {
        // A slot per tab, so one tab's preview doesn't cancel the other's.
        let (slot, label) = match plan.split {
            Resplit::Keep => ("balance-preview", "Previewing balance"),
            Resplit::Ratios { .. } => ("split-preview", "Previewing split"),
        };
        activity::track(slot, label, move |ctx| {
            let ws = open()?;
            if let Ok(name) = name_of(&ws, plan.input) {
                ctx.set_label(format!("{label} · {name}"));
            }
            let (_, set) = cached_labels(&ws, plan.input, &ctx.progress())?;
            Ok(ws.balance_preview_of(&plan, &set))
        })
        .await
    }

    pub fn run(plan: BalancePlan, output: OutputTarget, kind: JobKind) -> Result<Job, String> {
        if !matches!(kind, JobKind::Split | JobKind::Balance) {
            return Err("Not a Split & Balance job.".into());
        }
        let ws = open()?;
        let name = name_of(&ws, plan.input)?;
        let output = resolve_output(output, &[plan.input])?;
        if let Some(problem) = plan.problems().into_iter().next() {
            return Err(problem);
        }
        jobs::start(kind, vec![name], move |ctx| {
            let reporter = ctx.reporter();
            let (dir, s) = ws.apply_balance(&plan, &output, reporter.as_ref()).map_err(|e| e.to_string())?;
            let [train, val, test, none] = s.images_after;
            reporter.message(&match none {
                0 => format!("{train} train, {val} val, {test} test"),
                _ => format!("{train} train, {val} val, {test} test, {none} without a split"),
            });
            Ok(Some(dir.display().to_string()))
        })
    }
}
