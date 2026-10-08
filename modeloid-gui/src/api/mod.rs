//! Server functions. Their bodies run on the server (the user's machine),
//! which is where the `modeloid` core and the filesystem live.

pub mod activity;
pub mod appearance;
pub mod balance;
pub mod classes;
pub mod convert;
pub mod datasets;
pub mod dialogs;
pub mod download;
pub mod edit;
pub mod explore;
#[cfg(feature = "server")]
pub mod files;
#[cfg(feature = "server")]
pub mod guard;
pub mod health;
pub mod history;
pub mod import;
pub mod jobs;
pub mod label;
pub mod parameters;
pub mod pyenv;
pub mod settings;
pub mod system;
pub mod train;
pub mod transform;
pub mod tune;
pub mod watch;
pub mod workspace;

use dioxus::prelude::ServerFnError;

/// Runs `work` (disk, TOML, the keyring) on a thread meant for blocking, so the
/// async threads stay free for other requests and the long-polls.
#[cfg(feature = "server")]
pub async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, ServerFnError> {
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result.map_err(ServerFnError::new),
        Err(e) => Err(ServerFnError::new(format!("The server task failed: {e}"))),
    }
}

/// Text to show the user for a failed server function: just the server's
/// message, without the "error running server function" wrapper.
pub fn error_message(error: &ServerFnError) -> String {
    match error {
        ServerFnError::ServerError { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_message_strips_wrapper() {
        assert_eq!(error_message(&ServerFnError::new("boom")), "boom");
    }
}
