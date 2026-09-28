//! Dataset downloads (Roboflow, Kaggle, Hugging Face, direct zip links): one
//! job item per link, run by `api::jobs`.
//! The page starts a job, then long-polls `job_status(JobKind::Download)`.

use dioxus::prelude::*;

use crate::api::jobs::Job;
use crate::api::settings::ServiceId;

/// The service a link downloads from, if it needs credentials. A quick check
/// for the form; the server parses links properly.
pub fn link_service(link: &str) -> Option<ServiceId> {
    let rest = link.trim().split_once("://").map_or(link, |(_, r)| r).to_ascii_lowercase();
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    match host {
        "universe.roboflow.com" | "app.roboflow.com" => Some(ServiceId::Roboflow),
        "kaggle.com" | "www.kaggle.com" => Some(ServiceId::Kaggle),
        _ => None,
    }
}

/// Link lines that don't look like URLs, as (line number, text). Empty lines are ignored.
pub fn invalid_links(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .map(|(i, line)| (i + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty())
        .filter(|(_, line)| !(line.starts_with("https://") || line.starts_with("http://")))
        .map(|(n, line)| (n, line.to_string()))
        .collect()
}

/// Non-empty, trimmed link lines.
pub fn parse_links(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

/// Starts downloading `links` in order into the workspace's `datasets/` folder.
#[post("/api/download/start")]
pub async fn start_download(links: Vec<String>, format: String) -> Result<Job, ServerFnError> {
    backend::start(links, format).map_err(ServerFnError::new)
}

#[cfg(feature = "server")]
mod backend {
    use modeloid::{Settings, Source, Workspace};

    use crate::api::jobs::{self, Job, JobKind};
    use crate::api::workspace;

    const MAX_LINKS: usize = 50;

    fn validate_format(format: &str) -> Result<(), String> {
        let ok = !format.is_empty()
            && format.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if ok {
            Ok(())
        } else {
            Err(format!("'{format}' isn't a valid format name. Use lowercase letters, digits and dashes."))
        }
    }

    pub fn start(links: Vec<String>, format: String) -> Result<Job, String> {
        let links: Vec<String> = links.iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        if links.is_empty() {
            return Err("Enter at least one link.".into());
        }
        if links.len() > MAX_LINKS {
            return Err(format!("Up to {MAX_LINKS} links at a time."));
        }
        let format = format.trim().to_lowercase();
        validate_format(&format)?;
        // Catch malformed links now instead of as failed items.
        for (i, link) in links.iter().enumerate() {
            Source::from_link(link).map_err(|e| {
                if links.len() == 1 { format!("{e}.") } else { format!("Link {}: {e}.", i + 1) }
            })?;
        }

        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        let ws = Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))?;
        let opts = Settings::load_default().download_options(format);
        jobs::start(JobKind::Download, links.clone(), move |ctx| {
            let link = &links[ctx.index()];
            ctx.block_on(ws.download(link, &opts, ctx.reporter()))
                .map(|dir| Some(dir.display().to_string()))
                .map_err(|e| e.to_string())
        })
    }

    #[cfg(test)]
    mod tests {
        use super::validate_format;

        #[test]
        fn validates_formats() {
            assert!(validate_format("coco").is_ok());
            assert!(validate_format("yolov8-obb").is_ok());
            assert!(validate_format("").is_err());
            assert!(validate_format("COCO").is_err());
            assert!(validate_format("a/b").is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{invalid_links, parse_links};

    #[test]
    fn finds_link_services() {
        use super::link_service;
        use crate::api::settings::ServiceId;
        assert_eq!(link_service("https://universe.roboflow.com/a/b"), Some(ServiceId::Roboflow));
        assert_eq!(link_service("https://www.kaggle.com/datasets/a/b"), Some(ServiceId::Kaggle));
        assert_eq!(link_service("https://huggingface.co/datasets/a/b"), None);
        assert_eq!(link_service("https://example.com/kaggle.com"), None);
    }

    #[test]
    fn parses_link_lines() {
        let text = "  https://a.com/x \n\nhttp://b.com/y\nnot a link\n";
        assert_eq!(parse_links(text), vec!["https://a.com/x", "http://b.com/y", "not a link"]);
        assert_eq!(invalid_links(text), vec![(4, "not a link".to_string())]);
    }
}
