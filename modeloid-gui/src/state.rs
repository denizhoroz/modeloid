//! App-wide client state, shared through Dioxus context.

use dioxus::prelude::*;

use std::collections::HashMap;

use crate::api::activity::{activity_status, ActivitySnapshot};
use crate::api::appearance::Appearance;
use crate::api::error_message;
use crate::api::jobs::{ItemStatus, JobKind};
use crate::api::watch::workspace_changes;
use crate::api::workspace::{current_workspace, WorkspaceInfo};

/// The open workspace, or `None` when nothing is loaded.
/// `checked` turns true once the server has been asked, so pages can avoid
/// flashing "no workspace" while that request is in flight.
#[derive(Clone, Copy)]
pub struct WorkspaceState {
    pub workspace: Signal<Option<WorkspaceInfo>>,
    pub checked: Signal<bool>,
    /// Bumped when the workspace's datasets change (e.g. a download finished).
    pub datasets_version: Signal<u64>,
    /// Datasets selected in the datasets panel, in the order they were
    /// selected. Every tool takes its input from here. Cleared when the
    /// workspace changes; datasets that disappear are dropped from it.
    pub selected_datasets: Signal<Vec<u32>>,
    /// Whether the selection was last changed with the checkboxes (or Select
    /// all), adding to it, rather than replaced by a click on a row.
    pub batch_selected: Signal<bool>,
}

/// Call once near the root. Asks the server which workspace is open, so the
/// choice survives page reloads, and follows changes made outside the app.
pub fn provide_workspace_state() {
    let state = use_context_provider(|| WorkspaceState {
        workspace: Signal::new(None),
        checked: Signal::new(false),
        datasets_version: Signal::new(0),
        selected_datasets: Signal::new(Vec::new()),
        batch_selected: Signal::new(false),
    });
    let (mut workspace, mut checked) = (state.workspace, state.checked);
    let (mut datasets_version, mut selected) = (state.datasets_version, state.selected_datasets);

    use_future(move || async move {
        if let Ok(current) = current_workspace().await {
            workspace.set(current);
        }
        checked.set(true);
    });

    // The server checks the workspace folder every few seconds; on a change,
    // reload the workspace (it may be gone) and the datasets.
    use_future(move || async move {
        let mut since = 0;
        while let Ok(version) = workspace_changes(since).await {
            if since != 0 && version != since {
                if let Ok(current) = current_workspace().await {
                    if *workspace.peek() != current {
                        workspace.set(current);
                    }
                }
                *datasets_version.write() += 1;
            }
            since = version;
        }
    });

    // A selection belongs to one workspace.
    let mut selected_in = use_signal(|| None::<String>);
    use_effect(move || {
        let path = workspace.read().as_ref().map(|w| w.path.clone());
        if *selected_in.peek() != path {
            selected_in.set(path);
            selected.set(Vec::new());
        }
    });
}

pub fn use_workspace() -> Signal<Option<WorkspaceInfo>> {
    use_context::<WorkspaceState>().workspace
}

/// Read to re-run when datasets change; bump with `*signal.write() += 1`.
pub fn use_datasets_version() -> Signal<u64> {
    use_context::<WorkspaceState>().datasets_version
}

/// Whether the open workspace has been fetched from the server yet.
pub fn use_workspace_checked() -> Signal<bool> {
    use_context::<WorkspaceState>().checked
}

/// Datasets selected in the datasets panel, in selection order.
pub fn use_selected_datasets() -> Signal<Vec<u32>> {
    use_context::<WorkspaceState>().selected_datasets
}

/// See `WorkspaceState::batch_selected`.
pub fn use_batch_selected() -> Signal<bool> {
    use_context::<WorkspaceState>().batch_selected
}

/// The dataset single-dataset tools work on: the most recently selected one.
pub fn use_primary_dataset() -> Memo<Option<u32>> {
    let selected = use_selected_datasets();
    use_memo(move || selected.read().last().copied())
}

/// Chosen theme, font and text size. Provided in `App`, above the router,
/// from what the server has saved.
#[derive(Clone, Copy)]
pub struct AppearanceState(pub Signal<Appearance>);

pub fn use_appearance() -> Signal<Appearance> {
    use_context::<AppearanceState>().0
}

/// What the server is busy with: jobs (for tool pages' progress) and
/// background loads (for the status bar). One long-poll feeds the whole app;
/// browsers allow only 6 open requests per server, and a poll per job starved
/// page loads.
#[derive(Clone, Copy)]
pub struct ActivityState {
    pub snapshot: Signal<Option<ActivitySnapshot>>,
    /// Set when polling stopped (the server went away).
    pub error: Signal<Option<String>>,
}

/// Call once, in the app layout, after `provide_workspace_state`.
/// Every item a job finishes refreshes the datasets, since tools add or change
/// them, and so does the end of a job (the server renumbers datasets then).
pub fn provide_activity_state() {
    let state = use_context_provider(|| ActivityState { snapshot: Signal::new(None), error: Signal::new(None) });
    let (mut snapshot, mut error) = (state.snapshot, state.error);
    let mut datasets_version = use_datasets_version();

    use_future(move || async move {
        let mut since = 0;
        // Done items of each kind's latest job and whether it finished; `None` before the first answer.
        let mut done_seen: Option<HashMap<JobKind, (usize, bool)>> = None;
        loop {
            match activity_status(since).await {
                Ok(latest) => {
                    since = latest.version;
                    let done: HashMap<JobKind, (usize, bool)> = latest
                        .jobs
                        .iter()
                        .map(|j| (j.kind, (j.items.iter().filter(|i| i.status == ItemStatus::Done).count(), j.finished)))
                        .collect();
                    if done_seen.as_ref().is_some_and(|seen| *seen != done) {
                        *datasets_version.write() += 1;
                    }
                    done_seen = Some(done);
                    snapshot.set(Some(latest));
                }
                Err(e) => {
                    error.set(Some(error_message(&e)));
                    break;
                }
            }
        }
    });
}

pub fn use_activity() -> ActivityState {
    use_context::<ActivityState>()
}
