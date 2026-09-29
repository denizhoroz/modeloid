//! App-wide client state, shared through Dioxus context.

use dioxus::prelude::*;

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
}

/// Call once near the root. Asks the server which workspace is open, so the
/// choice survives page reloads, and follows changes made outside the app.
pub fn provide_workspace_state() {
    let state = use_context_provider(|| WorkspaceState {
        workspace: Signal::new(None),
        checked: Signal::new(false),
        datasets_version: Signal::new(0),
        selected_datasets: Signal::new(Vec::new()),
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

/// The dataset single-dataset tools work on: the most recently selected one.
pub fn use_primary_dataset() -> Memo<Option<u32>> {
    let selected = use_selected_datasets();
    use_memo(move || selected.read().last().copied())
}
