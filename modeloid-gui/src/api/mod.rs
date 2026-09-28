//! Server functions. Their bodies run on the server (the user's machine),
//! which is where the `modeloid` core and the filesystem live.

pub mod datasets;
pub mod download;
pub mod jobs;
pub mod settings;
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
