//! Notices changes made to the open workspace outside Modeloid (a dataset
//! folder deleted, renamed or copied in by hand, `dataset.toml` edited, the
//! workspace itself moved). A server task checks `Workspace::fingerprint`
//! every few seconds; pages long-poll `workspace_changes(since)`.

use dioxus::prelude::*;

/// Returns a change counter once it's above `since`, waiting up to ~20s.
/// The first call (`since = 0`) answers at once with the current counter.
#[post("/api/workspace/changes")]
pub async fn workspace_changes(since: u64) -> Result<u64, ServerFnError> {
    Ok(backend::changes(since).await)
}

#[cfg(feature = "server")]
mod backend {
    use std::pin::pin;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Once;
    use std::time::Duration;

    use modeloid::Workspace;
    use tokio::sync::Notify;
    use tokio::time::{interval, timeout, MissedTickBehavior};

    use crate::api::workspace;

    const CHECK_EVERY: Duration = Duration::from_secs(2);
    const POLL_TIMEOUT: Duration = Duration::from_secs(20);

    /// Starts at 1 so a first `since = 0` call returns straight away.
    static VERSION: AtomicU64 = AtomicU64::new(1);
    static CHANGED: Notify = Notify::const_new();
    static START: Once = Once::new();

    /// Current fingerprint: workspace folder + its disk state (none when closed).
    fn fingerprint() -> Option<(std::path::PathBuf, u64)> {
        let dir = workspace::current_dir()?;
        let print = Workspace::fingerprint(&dir);
        Some((dir, print))
    }

    fn start_checker() {
        START.call_once(|| {
            tokio::spawn(async {
                let mut last = tokio::task::spawn_blocking(fingerprint).await.ok().flatten();
                let mut ticks = interval(CHECK_EVERY);
                ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
                loop {
                    ticks.tick().await;
                    let Ok(now) = tokio::task::spawn_blocking(fingerprint).await else { continue };
                    if now != last {
                        last = now;
                        VERSION.fetch_add(1, Ordering::SeqCst);
                        CHANGED.notify_waiters();
                    }
                }
            });
        });
    }

    pub async fn changes(since: u64) -> u64 {
        start_checker();
        let mut changed = pin!(CHANGED.notified());
        changed.as_mut().enable();
        let version = VERSION.load(Ordering::SeqCst);
        if version > since {
            return version;
        }
        let _ = timeout(POLL_TIMEOUT, changed).await;
        VERSION.load(Ordering::SeqCst)
    }
}
