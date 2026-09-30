//! Dataset downloads (Roboflow, Kaggle, Hugging Face, direct zip links): one
//! job item per link, run by `api::jobs`.
//! The page starts a job; its progress comes through the activity poll.

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

/// Whether `link` is a Hugging Face link (its downloads go file by file through the API).
pub fn is_hugging_face(link: &str) -> bool {
    let rest = link.trim().split_once("://").map_or(link, |(_, r)| r).to_ascii_lowercase();
    matches!(rest.split(['/', '?', '#']).next(), Some("huggingface.co" | "www.huggingface.co"))
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
/// A Roboflow project without a dataset version ends as a warning with
/// `ItemAction::RoboflowFork` offered; `download_forking` takes that offer.
#[post("/api/download/start")]
pub async fn start_download(links: Vec<String>, format: String) -> Result<Job, ServerFnError> {
    backend::start(links, format).map_err(ServerFnError::new)
}

/// Takes `ItemAction::RoboflowFork` for item `index` (`link`) of the latest
/// download job: copies the project into the user's Roboflow workspace,
/// generates a version, downloads it and deletes the copy. Runs inside that
/// job, so its other items stay listed.
#[post("/api/download/fork")]
pub async fn download_forking(index: usize, link: String, format: String) -> Result<Job, ServerFnError> {
    backend::fork(index, link, format).map_err(ServerFnError::new)
}

#[cfg(feature = "server")]
mod backend {
    use modeloid::{AlreadyExists, DownloadOptions, NoVersions, Settings, Source, Workspace};

    use crate::api::jobs::{self, ItemAction, ItemCtx, Job, JobKind};
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

    /// Downloads `link` as one job item.
    fn download(ctx: &ItemCtx, ws: &Workspace, link: &str, opts: &DownloadOptions) -> Result<Option<String>, String> {
        match ctx.block_on(ws.download(link, opts, ctx.reporter())) {
            Ok(dir) => Ok(Some(dir.display().to_string())),
            Err(e) => {
                if e.downcast_ref::<NoVersions>().is_some() {
                    ctx.offer(ItemAction::RoboflowFork);
                } else if e.downcast_ref::<AlreadyExists>().is_some() {
                    ctx.skip();
                }
                Err(e.to_string())
            }
        }
    }

    fn open() -> Result<Workspace, String> {
        let dir = workspace::current_dir().ok_or("Load or create a workspace first.")?;
        Workspace::load_from_dir(&dir).map_err(|e| format!("Couldn't open the workspace: {e}"))
    }

    pub fn fork(index: usize, link: String, format: String) -> Result<Job, String> {
        let format = format.trim().to_lowercase();
        validate_format(&format)?;
        let ws = open()?;
        let mut opts = Settings::load_default().download_options(format);
        opts.fork_unversioned = true;
        jobs::rerun_item(JobKind::Download, index, &link.clone(), move |ctx| download(ctx, &ws, &link, &opts))
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

        let ws = open()?;
        let opts = Settings::load_default().download_options(format);
        jobs::start(JobKind::Download, links.clone(), jobs::Touches::Nothing, move |ctx| download(ctx, &ws, &links[ctx.index()], &opts))
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
        assert!(super::is_hugging_face(" https://huggingface.co/datasets/a/b "));
        assert!(!super::is_hugging_face("https://example.com/huggingface.co"));
    }

    #[test]
    fn parses_link_lines() {
        let text = "  https://a.com/x \n\nhttp://b.com/y\nnot a link\n";
        assert_eq!(parse_links(text), vec!["https://a.com/x", "http://b.com/y", "not a link"]);
        assert_eq!(invalid_links(text), vec![(4, "not a link".to_string())]);
    }
}
