//! Dataset downloads from Roboflow, Kaggle, Hugging Face and direct zip links.
//! `Downloader::inspect` checks a link and reads metadata, `Downloader::fetch`
//! downloads it into a folder.

use std::{
    env, fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use reqwest::{StatusCode, header};
use serde_json::Value;
use url::Url;

use crate::core::{
    io::{self, unless_cancelled},
    labels::LabelFormat,
    progress::{Progress, Reporter, Unit, check_cancel},
};
use crate::models::settings::{HF_TOKEN_ENV, KAGGLE_KEY_ENV, KAGGLE_USERNAME_ENV, KaggleCredentials, ROBOFLOW_KEY_ENV};

const ROBOFLOW_API: &str = "https://api.roboflow.com";
const KAGGLE_API: &str = "https://www.kaggle.com/api/v1";
const HF_HOST: &str = "https://huggingface.co";
const ROBOFLOW_UNIVERSE: &str = "https://universe.roboflow.com";
const EXPORT_POLL_ATTEMPTS: u32 = 20;
const EXPORT_POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Copying a project and generating a version: asked about this often, for this long at most.
const ROBOFLOW_POLL_INTERVAL: Duration = Duration::from_secs(3);
const ROBOFLOW_POLL_LIMIT: Duration = Duration::from_secs(30 * 60);

/// Export format and credentials. Missing credentials fall back to env vars
/// (and `~/.kaggle/kaggle.json`) when needed; see `Settings::download_options`.
#[derive(Debug, Clone, Default)]
pub struct DownloadOptions {
    /// Roboflow export format, e.g. "coco", "yolov8". Other sources ignore it.
    pub format: String,
    /// Roboflow API key.
    pub api_key: Option<String>,
    pub kaggle: Option<KaggleCredentials>,
    /// Only needed for private or gated Hugging Face datasets.
    pub hf_token: Option<String>,
    /// A Roboflow project without a dataset version: copy it into the key's
    /// own workspace, generate a version there, download that, and delete the
    /// copy again. Off: such a project fails with `NoVersions`.
    pub fork_unversioned: bool,
}

/// A Roboflow project that has images and annotations but no generated
/// dataset version, so there is nothing to export. `DownloadOptions::fork_unversioned`
/// downloads it anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoVersions {
    pub workspace: String,
    pub project: String,
}

impl std::fmt::Display for NoVersions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Roboflow project {}/{} has no dataset version, only images and annotations", self.workspace, self.project)
    }
}

impl std::error::Error for NoVersions {}

/// The dataset a download would create is already in the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlreadyExists {
    pub path: PathBuf,
}

impl std::fmt::Display for AlreadyExists {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "dataset already exists: {}", self.path.display())
    }
}

impl std::error::Error for AlreadyExists {}

/// A Roboflow project, by workspace and project url names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoboflowProject {
    pub workspace: String,
    pub project: String,
}

/// What `Downloader::inspect` found out about a link.
#[derive(Debug, PartialEq)]
pub struct Inspected {
    /// Where to download from: the link's source with the version filled in,
    /// or the copy made for a project without versions.
    pub source: Source,
    pub info: SourceInfo,
    /// Roboflow export format actually used: the requested one, or another
    /// when the project's type doesn't offer it.
    pub format: String,
    /// Folder name for the dataset.
    pub name: String,
    /// A copy made in the user's Roboflow workspace, to delete afterwards
    /// (`Downloader::cleanup`).
    pub cleanup: Option<RoboflowProject>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Roboflow {
        workspace: String,
        project: String,
        /// `None` means the latest version, resolved by `Downloader::inspect`.
        version: Option<u32>,
    },
    Kaggle {
        owner: String,
        dataset: String,
    },
    HuggingFace {
        /// `<owner>/<name>`.
        repo: String,
        revision: String,
    },
    /// Any other http(s) link; must return a zip file.
    Url(String),
}

impl Source {
    pub fn from_link(link: &str) -> Result<Self, crate::Error> {
        let url = Url::parse(link.trim()).map_err(|e| format!("invalid link: {e}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("links must start with https:// or http://".into());
        }
        let host = url.host_str().ok_or("link has no host")?;
        let segments: Vec<&str> = url
            .path_segments()
            .map(|s| s.filter(|p| !p.is_empty()).collect())
            .unwrap_or_default();

        match host {
            "universe.roboflow.com" => match segments.as_slice() {
                [ws, project, "dataset", version, ..] => Self::roboflow(ws, project, Some(version)),
                [ws, project, ..] => Self::roboflow(ws, project, None),
                _ => Err("expected universe.roboflow.com/<workspace>/<project>[/dataset/<version>]".into()),
            },
            "app.roboflow.com" => match segments.as_slice() {
                [ws, project, version, ..] => Self::roboflow(ws, project, Some(version)),
                [ws, project] => Self::roboflow(ws, project, None),
                _ => Err("expected app.roboflow.com/<workspace>/<project>[/<version>]".into()),
            },
            "www.kaggle.com" | "kaggle.com" => match segments.as_slice() {
                ["datasets", owner, dataset, ..] => {
                    Ok(Source::Kaggle { owner: owner.to_string(), dataset: dataset.to_string() })
                }
                _ => Err("expected kaggle.com/datasets/<owner>/<dataset>".into()),
            },
            "huggingface.co" | "www.huggingface.co" => match segments.as_slice() {
                ["datasets", owner, name, rest @ ..] => {
                    let revision = match rest {
                        ["tree" | "blob", revision, ..] => revision.to_string(),
                        _ => "main".to_string(),
                    };
                    Ok(Source::HuggingFace { repo: format!("{owner}/{name}"), revision })
                }
                _ => Err("expected huggingface.co/datasets/<owner>/<name>".into()),
            },
            _ => Ok(Source::Url(url.to_string())),
        }
    }

    fn roboflow(ws: &str, project: &str, version: Option<&str>) -> Result<Self, crate::Error> {
        let version = version
            .map(|v| v.parse().map_err(|_| format!("invalid Roboflow version: {v}")))
            .transpose()?;
        Ok(Source::Roboflow {
            workspace: ws.to_string(),
            project: project.to_string(),
            version,
        })
    }

    /// Folder name for the downloaded dataset.
    pub fn name(&self, opts: &DownloadOptions) -> String {
        let name = match self {
            Source::Roboflow { project, version: Some(v), .. } => return roboflow_name(project, *v, &opts.format),
            Source::Roboflow { project, version: None, .. } => format!("{project}-latest-{}", opts.format),
            Source::Kaggle { dataset, .. } => dataset.clone(),
            Source::HuggingFace { repo, .. } => repo.rsplit('/').next().unwrap_or(repo).to_string(),
            Source::Url(url) => {
                let url = Url::parse(url).ok();
                let file = url.as_ref().and_then(|u| u.path_segments()?.rev().find(|s| !s.is_empty()).map(String::from));
                let host = url.as_ref().and_then(|u| u.host_str().map(String::from));
                let file = file.or(host).unwrap_or_default();
                file.strip_suffix(".zip").unwrap_or(&file).to_string()
            }
        };
        folder_name(&name)
    }

    /// e.g. "Roboflow", for messages.
    pub fn label(&self) -> &'static str {
        match self {
            Source::Roboflow { .. } => "Roboflow",
            Source::Kaggle { .. } => "Kaggle",
            Source::HuggingFace { .. } => "Hugging Face",
            Source::Url(_) => "link",
        }
    }
}

fn roboflow_name(project: &str, version: u32, format: &str) -> String {
    folder_name(&format!("{project}-v{version}-{format}"))
}

/// Makes `name` safe as a folder name; "dataset" if nothing is left.
fn folder_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if r#"<>:"/\|?*%"#.contains(c) || c.is_control() { '-' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').to_string();
    if cleaned.is_empty() { "dataset".into() } else { cleaned }
}

/// Metadata the source reports about a dataset; empty strings when unknown.
#[derive(Debug, Default, PartialEq)]
pub struct SourceInfo {
    pub dataset_type: String,
    pub license: String,
}

pub struct Downloader {
    client: reqwest::Client,
    reporter: Arc<dyn Reporter>,
}

impl Downloader {
    pub fn new(reporter: Arc<dyn Reporter>) -> Result<Self, crate::Error> {
        let client = client_builder().build()?;
        Ok(Downloader { client, reporter })
    }

    /// Checks the dataset exists (and credentials work), reads its metadata and
    /// settles what exactly is downloaded. For Roboflow: the latest version when
    /// the link names none, a copy with a fresh version when the project has
    /// none (`DownloadOptions::fork_unversioned`), and an export format the
    /// project's type offers.
    pub async fn inspect(&self, source: Source, opts: &DownloadOptions) -> Result<Inspected, crate::Error> {
        let plain = |source: Source, info: SourceInfo| {
            let name = source.name(opts);
            Inspected { source, info, format: opts.format.clone(), name, cleanup: None }
        };
        match source {
            Source::Roboflow { workspace, project, version } => {
                let api_key = roboflow_api_key(opts)?;
                let url = format!("{ROBOFLOW_API}/{workspace}/{project}");
                let body = self.roboflow_get(&url, &api_key).await?;
                let info = roboflow_info(&body);

                let listed = version.or_else(|| {
                    let latest = latest_roboflow_version(&body)?;
                    self.reporter.message(&format!("Using latest version: {latest}"));
                    Some(latest)
                });
                // Where the version to download is: the project itself, or a copy made now.
                let mut cleanup = None;
                let settled: Result<(RoboflowProject, u32, String), crate::Error> = async {
                    let mut at = RoboflowProject { workspace: workspace.clone(), project: project.clone() };
                    let version = match listed {
                        Some(version) => version,
                        None if !opts.fork_unversioned => return Err(NoVersions { workspace: workspace.clone(), project: project.clone() }.into()),
                        None => {
                            let own = self.roboflow_own_workspace(&api_key).await?;
                            if own != workspace {
                                let copy = self.roboflow_fork(&own, &at, &api_key).await?;
                                at = RoboflowProject { workspace: own, project: copy };
                                cleanup = Some(at.clone());
                            }
                            self.roboflow_generate(&at, &api_key).await?
                        }
                    };
                    let format = self.roboflow_format(&at, version, &opts.format, &info.dataset_type, &api_key).await?;
                    Ok((at, version, format))
                }
                .await;

                match settled {
                    Ok((at, version, format)) => Ok(Inspected {
                        // Named after the project in the link, also when a copy is downloaded.
                        name: roboflow_name(&project, version, &format),
                        source: Source::Roboflow { workspace: at.workspace, project: at.project, version: Some(version) },
                        info,
                        format,
                        cleanup,
                    }),
                    Err(e) => {
                        if let Some(copy) = &cleanup {
                            self.roboflow_delete(copy, &api_key).await;
                        }
                        Err(e)
                    }
                }
            }
            Source::Kaggle { owner, dataset } => {
                let creds = kaggle_credentials(opts)?;
                let url = format!("{KAGGLE_API}/datasets/view/{owner}/{dataset}");
                let resp = self.client.get(&url).basic_auth(&creds.username, Some(&creds.key)).send().await?;
                let body = match resp.status() {
                    s if s.is_success() => resp.json::<Value>().await.unwrap_or(Value::Null),
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                        return Err("Kaggle rejected the credentials. Check your username and key.".into());
                    }
                    StatusCode::NOT_FOUND => return Err(format!("Kaggle dataset {owner}/{dataset} not found").into()),
                    s => return Err(format!("Kaggle API returned {s}").into()),
                };
                let license = body["licenseName"].as_str().or_else(|| body["licenses"][0]["name"].as_str());
                let info = SourceInfo { license: license.unwrap_or_default().to_string(), ..Default::default() };
                Ok(plain(Source::Kaggle { owner, dataset }, info))
            }
            Source::HuggingFace { repo, revision } => {
                let body = self.hf_info(&repo, &revision, opts).await?;
                let license = match &body["cardData"]["license"] {
                    Value::String(l) => l.clone(),
                    Value::Array(ls) => ls.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
                    _ => String::new(),
                };
                Ok(plain(Source::HuggingFace { repo, revision }, SourceInfo { license, ..Default::default() }))
            }
            Source::Url(url) => Ok(plain(Source::Url(url), SourceInfo::default())),
        }
    }

    /// Deletes the copy `inspect` made in the user's Roboflow workspace, if
    /// any. Call it once the download ended, however it ended. A copy that
    /// can't be deleted is reported as a message, not as an error.
    pub async fn cleanup(&self, inspected: &Inspected, opts: &DownloadOptions) {
        if let (Some(copy), Ok(api_key)) = (&inspected.cleanup, roboflow_api_key(opts)) {
            self.roboflow_delete(copy, &api_key).await;
        }
    }

    /// Downloads the dataset's files into `out` (created). Archives are first
    /// saved as `archive` (removed by the caller) and extracted.
    pub async fn fetch(&self, source: &Source, opts: &DownloadOptions, archive: &Path, out: &Path) -> Result<(), crate::Error> {
        match source {
            Source::Roboflow { .. } => {
                let url = self.resolve(source, opts).await?;
                self.download_file(&url, archive).await?;
                io::extract_zip(archive, out, self.reporter.as_ref()).await?;
            }
            Source::Kaggle { owner, dataset } => {
                let url = self.kaggle_archive_url(owner, dataset, opts).await?;
                self.download_file(&url, archive).await?;
                self.extract_unwrapped(archive, out).await?;
            }
            Source::Url(url) => {
                self.download_file(url, archive).await?;
                if !io::is_zip(archive)? {
                    return Err("the link didn't return a zip file. Link directly to a .zip download.".into());
                }
                self.extract_unwrapped(archive, out).await?;
            }
            Source::HuggingFace { repo, revision } => self.hf_download(repo, revision, opts, out).await?,
        }
        Ok(())
    }

    /// Returns a direct URL to a Roboflow dataset archive.
    pub async fn resolve(&self, source: &Source, opts: &DownloadOptions) -> Result<String, crate::Error> {
        match source {
            Source::Roboflow { workspace, project, version } => {
                let version = version.ok_or("Roboflow version not resolved, call inspect first")?;
                let api_key = roboflow_api_key(opts)?;
                let url = format!("{ROBOFLOW_API}/{workspace}/{project}/{version}/{}", opts.format);
                self.roboflow_export_link(&url, &api_key).await
            }
            other => Err(format!("no single archive link for {} datasets", other.label()).into()),
        }
    }

    async fn extract_unwrapped(&self, archive: &Path, out: &Path) -> Result<(), crate::Error> {
        io::extract_zip(archive, out, self.reporter.as_ref()).await?;
        let out = out.to_path_buf();
        tokio::task::spawn_blocking(move || io::unwrap_single_dir(&out)).await??;
        Ok(())
    }

    /// One Roboflow API call: the status and the JSON answer (`Null` when it isn't JSON).
    async fn roboflow_call(&self, request: reqwest::RequestBuilder, api_key: &str) -> Result<(StatusCode, Value), crate::Error> {
        let stop = || self.reporter.is_cancelled();
        let resp = unless_cancelled(stop, request.query(&[("api_key", api_key)]).send()).await??;
        let status = resp.status();
        Ok((status, unless_cancelled(stop, resp.json()).await?.unwrap_or(Value::Null)))
    }

    /// Waits `time` between polls; Stop ends the wait at once.
    async fn pause(&self, time: Duration) -> Result<(), crate::Error> {
        Ok(unless_cancelled(|| self.reporter.is_cancelled(), tokio::time::sleep(time)).await?)
    }

    /// The answer of a Roboflow API call, or its error message.
    async fn roboflow_send(&self, request: reqwest::RequestBuilder, api_key: &str) -> Result<Value, crate::Error> {
        let (status, body) = self.roboflow_call(request, api_key).await?;
        if !status.is_success() {
            return Err(format!("Roboflow API returned {status}: {}", roboflow_error(&body)).into());
        }
        Ok(body)
    }

    async fn roboflow_get(&self, url: &str, api_key: &str) -> Result<Value, crate::Error> {
        self.roboflow_send(self.client.get(url), api_key).await
    }

    /// The workspace the API key belongs to.
    async fn roboflow_own_workspace(&self, api_key: &str) -> Result<String, crate::Error> {
        let body = self.roboflow_get(ROBOFLOW_API, api_key).await?;
        Ok(body["workspace"].as_str().ok_or("Roboflow didn't say which workspace the API key belongs to")?.to_string())
    }

    /// Copies `source` (someone's public project) into workspace `own`, with
    /// its images and annotations. Returns the copy's project name.
    async fn roboflow_fork(&self, own: &str, source: &RoboflowProject, api_key: &str) -> Result<String, crate::Error> {
        self.reporter.message(&format!("Copying {}/{} into your Roboflow workspace {own}", source.workspace, source.project));
        let link = format!("{ROBOFLOW_UNIVERSE}/{}/{}", source.workspace, source.project);
        let request = self.client.post(format!("{ROBOFLOW_API}/{own}/projects/fork")).json(&serde_json::json!({ "url": link }));
        let started = self.roboflow_send(request, api_key).await?;
        let task = started["taskId"].as_str().ok_or("Roboflow didn't start copying the project")?;
        let url = format!("{ROBOFLOW_API}/{own}/asynctasks/{task}");

        let progress = self.reporter.start("Copying project", Unit::Files);
        let result = async {
            let deadline = tokio::time::Instant::now() + ROBOFLOW_POLL_LIMIT;
            let mut counted = 0;
            loop {
                // Cancelling stops waiting; Roboflow finishes the copy, and only then can it be deleted.
                if self.reporter.is_cancelled() {
                    self.reporter.message("Cancelled while Roboflow was copying the project: the copy may still appear in your Roboflow workspace");
                }
                check_cancel(self.reporter.is_cancelled())?;
                match fork_task_state(&self.roboflow_get(&url, api_key).await?) {
                    TaskState::Done(project) => return Ok(project),
                    TaskState::Failed(why) => return Err(format!("Roboflow couldn't copy the project: {why}").into()),
                    TaskState::Running { done, total } => {
                        if total > 0 {
                            progress.set_total(total);
                        }
                        progress.add(done.saturating_sub(counted));
                        counted = counted.max(done);
                    }
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err("Roboflow took too long to copy the project; it may still appear in your Roboflow workspace".into());
                }
                self.pause(ROBOFLOW_POLL_INTERVAL).await?;
            }
        }
        .await;
        progress.finish();
        result
    }

    /// Generates a version of `project` as it is (no preprocessing, no
    /// augmentation) and waits until it is ready. Returns its number.
    async fn roboflow_generate(&self, project: &RoboflowProject, api_key: &str) -> Result<u32, crate::Error> {
        let base = format!("{ROBOFLOW_API}/{}/{}", project.workspace, project.project);
        let settings = serde_json::json!({ "preprocessing": {}, "augmentation": {} });
        let started = self.roboflow_send(self.client.post(format!("{base}/generate")).json(&settings), api_key).await?;
        let version = number(&started["version"]).ok_or("Roboflow didn't say which version it is generating")?;
        self.reporter.message(&format!("Generating version {version}"));

        let progress = self.reporter.start("Generating version", Unit::Steps);
        progress.set_total(100);
        let result = async {
            let deadline = tokio::time::Instant::now() + ROBOFLOW_POLL_LIMIT;
            let mut counted = 0;
            loop {
                check_cancel(self.reporter.is_cancelled())?;
                let request = self.client.get(format!("{base}/{version}")).query(&[("nocache", "true")]);
                let (generating, share) = generation_state(&self.roboflow_send(request, api_key).await?);
                let percent = (share * 100.0) as u64;
                progress.add(percent.saturating_sub(counted));
                counted = counted.max(percent);
                if !generating {
                    return Ok(version);
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err("Roboflow took too long to generate the version".into());
                }
                self.pause(ROBOFLOW_POLL_INTERVAL).await?;
            }
        }
        .await;
        progress.finish();
        result
    }

    /// The export format to use for `version`: `requested`, or, when the
    /// project's type doesn't offer it, one it does (see `pick_format`).
    async fn roboflow_format(
        &self,
        project: &RoboflowProject,
        version: u32,
        requested: &str,
        project_type: &str,
        api_key: &str,
    ) -> Result<String, crate::Error> {
        let url = format!("{ROBOFLOW_API}/{}/{}/{version}/{requested}", project.workspace, project.project);
        let (status, body) = self.roboflow_call(self.client.get(&url), api_key).await?;
        if status.is_success() {
            return Ok(requested.to_string());
        }
        let error = roboflow_error(&body);
        let fallback = parse_allowed_formats(error).and_then(|allowed| pick_format(requested, &allowed));
        match fallback {
            Some(format) if format != requested => {
                let whose = if project_type.is_empty() { "this project".to_string() } else { format!("{project_type} projects") };
                self.reporter.message(&format!("{requested} isn't available for {whose}; using {format}"));
                Ok(format)
            }
            _ => Err(format!("Roboflow API returned {status}: {error}").into()),
        }
    }

    /// Deletes a copy made by `roboflow_fork` (it goes to Roboflow's trash).
    /// Failing is reported as a message: the download itself is what counts.
    async fn roboflow_delete(&self, copy: &RoboflowProject, api_key: &str) {
        let url = format!("{ROBOFLOW_API}/{}/{}", copy.workspace, copy.project);
        let note = match self.roboflow_send(self.client.delete(&url), api_key).await {
            Ok(_) => format!("Deleted the copy {}/{} from your Roboflow workspace", copy.workspace, copy.project),
            Err(e) => format!("Couldn't delete the copy {}/{} from your Roboflow workspace ({e}); delete it there by hand", copy.workspace, copy.project),
        };
        self.reporter.message(&note);
    }

    // Roboflow generates exports on demand, so the link may not exist on the first request.
    async fn roboflow_export_link(&self, url: &str, api_key: &str) -> Result<String, crate::Error> {
        // Started lazily so a ready export shows no progress at all.
        let mut progress = None;
        let result = async {
            for _ in 0..EXPORT_POLL_ATTEMPTS {
                check_cancel(self.reporter.is_cancelled())?;
                let body = self.roboflow_get(url, api_key).await?;

                if let Some(link) = body["export"]["link"].as_str() {
                    return Ok(link.to_string());
                }

                progress
                    .get_or_insert_with(|| {
                        let p = self.reporter.start("Exporting", Unit::Steps);
                        p.set_total(u64::from(EXPORT_POLL_ATTEMPTS));
                        p
                    })
                    .add(1);
                self.pause(EXPORT_POLL_INTERVAL).await?;
            }
            Err("Roboflow export was not ready in time, try again later".into())
        }
        .await;

        if let Some(p) = progress {
            p.finish();
        }
        result
    }

    /// Kaggle answers the download endpoint with a redirect to a signed storage
    /// link; that link is fetched without credentials.
    async fn kaggle_archive_url(&self, owner: &str, dataset: &str, opts: &DownloadOptions) -> Result<String, crate::Error> {
        let creds = kaggle_credentials(opts)?;
        let client = client_builder().redirect(reqwest::redirect::Policy::none()).build()?;
        let url = format!("{KAGGLE_API}/datasets/download/{owner}/{dataset}");
        let resp = client.get(&url).basic_auth(&creds.username, Some(&creds.key)).send().await?;
        match resp.status() {
            s if s.is_redirection() => {
                let location = resp.headers().get(header::LOCATION).and_then(|l| l.to_str().ok());
                Ok(location.ok_or("Kaggle sent a redirect without a location")?.to_string())
            }
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                Err("Kaggle refused the download. Check your credentials, and accept the dataset's rules on kaggle.com if it has any.".into())
            }
            s => Err(format!("Kaggle download returned {s}").into()),
        }
    }

    async fn hf_info(&self, repo: &str, revision: &str, opts: &DownloadOptions) -> Result<Value, crate::Error> {
        let mut url = Url::parse(HF_HOST)?;
        url.path_segments_mut().map_err(|_| "bad Hugging Face URL")?.extend(["api", "datasets"]).extend(repo.split('/')).extend(["revision", revision]);
        url.query_pairs_mut().append_pair("blobs", "true");

        let mut request = self.client.get(url);
        if let Some(token) = hf_token(opts) {
            request = request.bearer_auth(token);
        }
        let resp = request.send().await?;
        match resp.status() {
            s if s.is_success() => Ok(resp.json().await?),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => Err(format!(
                "Hugging Face dataset {repo} (revision {revision}) not found or not accessible. Private and gated datasets need a token."
            )
            .into()),
            s => Err(format!("Hugging Face API returned {s}").into()),
        }
    }

    /// Downloads every file of a Hugging Face dataset repo into `out`, then
    /// extracts any zip files in place.
    async fn hf_download(&self, repo: &str, revision: &str, opts: &DownloadOptions, out: &Path) -> Result<(), crate::Error> {
        let body = self.hf_info(repo, revision, opts).await?;
        let files: Vec<(String, Option<u64>)> = body["siblings"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|f| Some((f["rfilename"].as_str()?.to_string(), f["size"].as_u64())))
            .filter(|(name, _)| name != ".gitattributes")
            .collect();
        if files.is_empty() {
            return Err(format!("Hugging Face dataset {repo} has no files").into());
        }

        let mut builder = client_builder();
        if let Some(token) = hf_token(opts) {
            let mut headers = header::HeaderMap::new();
            let mut value = header::HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| "invalid Hugging Face token")?;
            value.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, value);
            builder = builder.default_headers(headers);
        }
        let client = builder.build()?;

        let progress = self.reporter.start("Downloading", Unit::Bytes);
        if files.iter().all(|(_, size)| size.is_some()) {
            progress.set_total(files.iter().filter_map(|(_, s)| *s).sum());
        }
        let result = async {
            fs::create_dir_all(out)?;
            for (i, (name, _)) in files.iter().enumerate() {
                check_cancel(self.reporter.is_cancelled())?;
                self.reporter.message(&format!("File {} of {}: {name}", i + 1, files.len()));
                let dest = safe_join(out, name).ok_or_else(|| format!("unsafe file name in dataset: {name}"))?;
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut url = Url::parse(HF_HOST)?;
                url.path_segments_mut()
                    .map_err(|_| "bad Hugging Face URL")?
                    .push("datasets")
                    .extend(repo.split('/'))
                    .extend(["resolve", revision])
                    .extend(name.split('/'));
                io::download_into(&client, url.as_str(), &dest, &AddOnly(progress.as_ref())).await?;
            }
            Ok::<_, crate::Error>(())
        }
        .await;
        progress.finish();
        result?;

        // `data/train.zip` → `data/train/`, so split names in zip names survive.
        for (name, _) in files.iter().filter(|(n, _)| n.to_ascii_lowercase().ends_with(".zip")) {
            let Some(zip) = safe_join(out, name) else { continue };
            let dir = zip.with_extension("");
            if dir.exists() {
                continue; // a folder of that name came with the repo; leave the zip alone
            }
            io::extract_zip(&zip, &dir, self.reporter.as_ref()).await?;
            io::unwrap_single_dir(&dir)?;
            io::retry_busy(&zip, || fs::remove_file(&zip))?;
        }
        Ok(())
    }

    /// Downloads `url` to `dest`, writing to `dest.part` until complete.
    pub async fn download_file(&self, url: &str, dest: &Path) -> Result<(), crate::Error> {
        io::download(&self.client, url, dest, self.reporter.as_ref()).await?;
        Ok(())
    }
}

fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(concat!("modeloid/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
}

/// `dir/rel` for a `/`-separated relative path, or `None` if it would leave `dir`.
fn safe_join(dir: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel);
    rel.components().all(|c| matches!(c, Component::Normal(_))).then(|| dir.join(rel))
}

/// Forwards byte counts to a shared progress, ignoring per-file totals.
struct AddOnly<'a>(&'a dyn Progress);

impl Progress for AddOnly<'_> {
    fn set_total(&self, _total: u64) {}
    fn add(&self, n: u64) {
        self.0.add(n);
    }
    fn finish(&self) {}
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

fn roboflow_api_key(opts: &DownloadOptions) -> Result<String, crate::Error> {
    match &opts.api_key {
        Some(key) => Ok(key.clone()),
        None => env::var(ROBOFLOW_KEY_ENV)
            .map_err(|_| format!("Roboflow API key missing: save one in settings or set {ROBOFLOW_KEY_ENV}").into()),
    }
}

fn kaggle_credentials(opts: &DownloadOptions) -> Result<KaggleCredentials, crate::Error> {
    opts.kaggle
        .clone()
        .or_else(KaggleCredentials::from_env)
        .or_else(|| KaggleCredentials::from_kaggle_json().map(|(c, _)| c))
        .ok_or_else(|| {
            format!("Kaggle credentials missing: save them in settings, set {KAGGLE_USERNAME_ENV} and {KAGGLE_KEY_ENV}, or add ~/.kaggle/kaggle.json").into()
        })
}

fn hf_token(opts: &DownloadOptions) -> Option<String> {
    opts.hf_token.clone().or_else(|| env::var(HF_TOKEN_ENV).ok().filter(|t| !t.is_empty()))
}

/// The message in a failed Roboflow answer.
fn roboflow_error(body: &Value) -> &str {
    body["error"]["message"].as_str().or_else(|| body["error"].as_str()).or_else(|| body["message"].as_str()).unwrap_or("no details")
}

/// The formats a Roboflow "invalid format" message lists after "Please use one of:".
fn parse_allowed_formats(message: &str) -> Option<Vec<String>> {
    let (_, list) = message.split_once("Please use one of:")?;
    let formats: Vec<String> = list.split(',').map(|f| f.trim().trim_end_matches('.').to_string()).filter(|f| !f.is_empty()).collect();
    (!formats.is_empty()).then_some(formats)
}

/// The format to export instead of `requested`, out of those the project
/// allows: one of the same family (YOLO, COCO, VOC), else any the label
/// reader knows, else class folders, else the first one.
fn pick_format(requested: &str, allowed: &[String]) -> Option<String> {
    if allowed.iter().any(|f| f == requested) {
        return Some(requested.to_string());
    }
    let family = LabelFormat::parse(requested);
    let readable = || allowed.iter().filter(|f| LabelFormat::parse(f).is_some());
    readable()
        .find(|f| family.is_some() && LabelFormat::parse(f) == family)
        .or_else(|| readable().next())
        .or_else(|| allowed.iter().find(|f| *f == "folder"))
        .or_else(|| allowed.first())
        .cloned()
}

/// A whole number Roboflow sends as a number or as text ("1").
fn number(value: &Value) -> Option<u32> {
    value.as_u64().and_then(|n| u32::try_from(n).ok()).or_else(|| value.as_str()?.parse().ok())
}

/// Where a Roboflow fork task stands.
#[derive(Debug, PartialEq)]
enum TaskState {
    Running { done: u64, total: u64 },
    /// Finished; the copy's project name.
    Done(String),
    Failed(String),
}

fn fork_task_state(body: &Value) -> TaskState {
    let count = |key: &str| body["progress"][key].as_u64().unwrap_or(0);
    match body["status"].as_str() {
        Some("completed") => match body["result"]["datasetUrl"].as_str() {
            Some(project) => TaskState::Done(project.to_string()),
            None => TaskState::Failed("Roboflow didn't name the copy".into()),
        },
        Some("running" | "pending" | "queued" | "created") => TaskState::Running { done: count("current"), total: count("total") },
        status => {
            let why = body["error"]["message"].as_str().or_else(|| body["error"].as_str()).or_else(|| body["result"]["error"].as_str());
            TaskState::Failed(why.or(status).unwrap_or("no details").to_string())
        }
    }
}

/// Whether a version is still being generated, and how far it is (0..1).
fn generation_state(body: &Value) -> (bool, f64) {
    let version = &body["version"];
    let generating = version["generating"].as_bool().unwrap_or(false);
    (generating, version["progress"].as_f64().unwrap_or(if generating { 0.0 } else { 1.0 }).clamp(0.0, 1.0))
}

fn roboflow_info(body: &Value) -> SourceInfo {
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    SourceInfo {
        dataset_type: text(&body["project"]["type"]),
        license: text(&body["project"]["license"]),
    }
}

// Version ids look like "<workspace>/<project>/<n>"; `project.versions` is a count fallback.
fn latest_roboflow_version(body: &Value) -> Option<u32> {
    let from_list = body["versions"].as_array().and_then(|versions| {
        versions
            .iter()
            .filter_map(|v| v["id"].as_str()?.rsplit('/').next()?.parse().ok())
            .max()
    });
    from_list.or_else(|| {
        body["project"]["versions"]
            .as_u64()
            .filter(|&n| n > 0)
            .and_then(|n| u32::try_from(n).ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rf(ws: &str, project: &str, version: Option<u32>) -> Source {
        Source::Roboflow {
            workspace: ws.into(),
            project: project.into(),
            version,
        }
    }

    #[test]
    fn parses_universe_link() {
        let s = Source::from_link("https://universe.roboflow.com/acme/cats/dataset/3").unwrap();
        assert_eq!(s, rf("acme", "cats", Some(3)));
    }

    #[test]
    fn parses_universe_link_with_trailing_parts() {
        let s = Source::from_link("https://universe.roboflow.com/acme/cats/dataset/3/download/coco").unwrap();
        assert_eq!(s, rf("acme", "cats", Some(3)));
    }

    #[test]
    fn parses_universe_project_link_without_version() {
        let s = Source::from_link("https://universe.roboflow.com/roboflow-gw7yv/self-driving-car").unwrap();
        assert_eq!(s, rf("roboflow-gw7yv", "self-driving-car", None));
    }

    #[test]
    fn parses_app_link() {
        let s = Source::from_link("https://app.roboflow.com/acme/cats/7").unwrap();
        assert_eq!(s, rf("acme", "cats", Some(7)));
        let s = Source::from_link("https://app.roboflow.com/acme/cats").unwrap();
        assert_eq!(s, rf("acme", "cats", None));
    }

    #[test]
    fn parses_other_links_as_zip_urls() {
        let s = Source::from_link("https://example.com/files/cats.zip?dl=1").unwrap();
        assert_eq!(s, Source::Url("https://example.com/files/cats.zip?dl=1".into()));
        assert_eq!(s.name(&DownloadOptions::default()), "cats");
        let s = Source::from_link("https://example.com/").unwrap();
        assert_eq!(s.name(&DownloadOptions::default()), "example.com");
        assert!(Source::from_link("ftp://example.com/a.zip").is_err());
    }

    #[test]
    fn parses_kaggle_link() {
        let s = Source::from_link("https://www.kaggle.com/datasets/andrewmvd/car-plate-detection/data").unwrap();
        assert_eq!(s, Source::Kaggle { owner: "andrewmvd".into(), dataset: "car-plate-detection".into() });
        assert_eq!(s.name(&DownloadOptions::default()), "car-plate-detection");
        assert!(Source::from_link("https://www.kaggle.com/competitions/titanic").is_err());
    }

    #[test]
    fn parses_hugging_face_link() {
        let s = Source::from_link("https://huggingface.co/datasets/detection-datasets/coco").unwrap();
        assert_eq!(s, Source::HuggingFace { repo: "detection-datasets/coco".into(), revision: "main".into() });
        assert_eq!(s.name(&DownloadOptions::default()), "coco");
        let s = Source::from_link("https://huggingface.co/datasets/a/b/tree/v2/data").unwrap();
        assert_eq!(s, Source::HuggingFace { repo: "a/b".into(), revision: "v2".into() });
        assert!(Source::from_link("https://huggingface.co/bert-base-uncased").is_err());
    }

    #[test]
    fn makes_safe_names_and_paths() {
        assert_eq!(folder_name("a:b/c?"), "a-b-c-");
        assert_eq!(folder_name(" .. "), "dataset");
        assert!(safe_join(Path::new("out"), "data/train.zip").is_some());
        assert!(safe_join(Path::new("out"), "../evil").is_none());
        assert!(safe_join(Path::new("out"), "/abs").is_none());
    }

    #[test]
    fn rejects_bad_links() {
        assert!(Source::from_link("https://universe.roboflow.com/acme").is_err());
        assert!(Source::from_link("https://universe.roboflow.com/acme/cats/dataset/latest").is_err());
    }

    #[test]
    fn picks_latest_version() {
        let body = serde_json::json!({
            "project": { "versions": 2 },
            "versions": [{ "id": "acme/cats/1" }, { "id": "acme/cats/5" }, { "id": "acme/cats/3" }]
        });
        assert_eq!(latest_roboflow_version(&body), Some(5));

        let body = serde_json::json!({ "project": { "versions": 4 } });
        assert_eq!(latest_roboflow_version(&body), Some(4));

        let body = serde_json::json!({ "project": { "versions": 0 }, "versions": [] });
        assert_eq!(latest_roboflow_version(&body), None);
    }

    #[test]
    fn falls_back_to_a_format_the_project_allows() {
        let message = "yolov8 is an invalid format for project type semantic-segmentation. Please use one of: coco-segmentation, png-mask-semantic.";
        let allowed = parse_allowed_formats(message).unwrap();
        assert_eq!(allowed, ["coco-segmentation", "png-mask-semantic"]);
        assert_eq!(pick_format("yolov8", &allowed).as_deref(), Some("coco-segmentation"));
        assert_eq!(parse_allowed_formats("Unsupported request."), None);

        let list = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        assert_eq!(pick_format("yolov8", &list(&["coco", "yolov5pytorch", "voc"])).as_deref(), Some("yolov5pytorch"), "same family first");
        assert_eq!(pick_format("coco", &list(&["multiclass", "folder"])).as_deref(), Some("folder"));
        assert_eq!(pick_format("coco", &list(&["coco", "voc"])).as_deref(), Some("coco"));
        assert_eq!(pick_format("coco", &list(&["clip", "multiclass"])).as_deref(), Some("clip"));
        assert_eq!(pick_format("coco", &[]), None);
    }

    #[test]
    fn reads_fork_and_generation_progress() {
        let running = serde_json::json!({ "status": "running", "progress": { "total": 2886, "current": 778 } });
        assert_eq!(fork_task_state(&running), TaskState::Running { done: 778, total: 2886 });
        let done = serde_json::json!({ "status": "completed", "result": { "forked": true, "datasetUrl": "car-parts-fd94l" } });
        assert_eq!(fork_task_state(&done), TaskState::Done("car-parts-fd94l".into()));
        let failed = serde_json::json!({ "status": "failed", "error": "Source project not found" });
        assert_eq!(fork_task_state(&failed), TaskState::Failed("Source project not found".into()));
        assert_eq!(fork_task_state(&serde_json::json!({})), TaskState::Failed("no details".into()));

        assert_eq!(generation_state(&serde_json::json!({ "version": { "generating": true, "progress": 0.3 } })), (true, 0.3));
        assert_eq!(generation_state(&serde_json::json!({ "version": { "generating": false, "progress": 1 } })), (false, 1.0));
        assert_eq!(generation_state(&serde_json::json!({ "version": { "generating": true } })), (true, 0.0));
        assert_eq!((number(&serde_json::json!("1")), number(&serde_json::json!(4)), number(&serde_json::json!("x"))), (Some(1), Some(4), None));
    }

    #[test]
    fn no_versions_is_a_typed_error() {
        let error: crate::Error = NoVersions { workspace: "acme".into(), project: "cats".into() }.into();
        assert!(error.to_string().contains("acme/cats has no dataset version"));
        assert_eq!(error.downcast_ref::<NoVersions>().map(|e| e.project.as_str()), Some("cats"));
        assert_eq!(roboflow_name("car parts?", 1, "coco"), "car parts--v1-coco");
    }

    #[test]
    fn reads_project_info() {
        let body = serde_json::json!({ "project": { "type": "object-detection", "license": "CC BY 4.0" } });
        let info = roboflow_info(&body);
        assert_eq!(info.dataset_type, "object-detection");
        assert_eq!(info.license, "CC BY 4.0");
        assert_eq!(roboflow_info(&serde_json::json!({})), SourceInfo::default());
    }
}
