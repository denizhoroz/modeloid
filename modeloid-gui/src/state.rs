//! App-wide client state, shared through Dioxus context.

use dioxus::prelude::*;

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
}

/// Call once near the root. Asks the server which workspace is open,
/// so the choice survives page reloads.
pub fn provide_workspace_state() {
    let state = use_context_provider(|| WorkspaceState {
        workspace: Signal::new(None),
        checked: Signal::new(false),
        datasets_version: Signal::new(0),
    });
    let (mut workspace, mut checked) = (state.workspace, state.checked);

    use_future(move || async move {
        if let Ok(current) = current_workspace().await {
            workspace.set(current);
        }
        checked.set(true);
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
