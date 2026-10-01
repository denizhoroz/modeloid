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
pub mod explore;
#[cfg(feature = "server")]
pub mod files;
pub mod health;
pub mod import;
pub mod jobs;
pub mod parameters;
pub mod settings;
pub mod system;
pub mod watch;
pub mod workspace;

use dioxus::prelude::ServerFnError;

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
