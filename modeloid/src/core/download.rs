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
    io,
    progress::{Progress, Reporter, Unit, check_cancel},
};
use crate::models::settings::{HF_TOKEN_ENV, KAGGLE_KEY_ENV, KAGGLE_USERNAME_ENV, KaggleCredentials, ROBOFLOW_KEY_ENV};

const ROBOFLOW_API: &str = "https://api.roboflow.com";
const KAGGLE_API: &str = "https://www.kaggle.com/api/v1";
const HF_HOST: &str = "https://huggingface.co";
const EXPORT_POLL_ATTEMPTS: u32 = 20;
const EXPORT_POLL_INTERVAL: Duration = Duration::from_secs(3);

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
            Source::Roboflow { project, version: Some(v), .. } => format!("{project}-v{v}-{}", opts.format),
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
    /// replaces an unspecified Roboflow version with the latest one.
    pub async fn inspect(&self, source: Source, opts: &DownloadOptions) -> Result<(Source, SourceInfo), crate::Error> {
        match source {
            Source::Roboflow { workspace, project, version } => {
                let api_key = roboflow_api_key(opts)?;
                let url = format!("{ROBOFLOW_API}/{workspace}/{project}");
                let body = self.roboflow_get(&url, &api_key).await?;

                let version = match version {
                    Some(v) => v,
                    None => {
                        let latest = latest_roboflow_version(&body)
                            .ok_or_else(|| format!("Roboflow project {workspace}/{project} has no versions"))?;
                        self.reporter.message(&format!("Using latest version: {latest}"));
                        latest
                    }
                };
                let info = roboflow_info(&body);
                Ok((Source::Roboflow { workspace, project, version: Some(version) }, info))
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
                Ok((Source::Kaggle { owner, dataset }, info))
            }
            Source::HuggingFace { repo, revision } => {
                let body = self.hf_info(&repo, &revision, opts).await?;
                let license = match &body["cardData"]["license"] {
                    Value::String(l) => l.clone(),
                    Value::Array(ls) => ls.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
                    _ => String::new(),
                };
                Ok((Source::HuggingFace { repo, revision }, SourceInfo { license, ..Default::default() }))
            }
            Source::Url(url) => Ok((Source::Url(url), SourceInfo::default())),
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

    async fn roboflow_get(&self, url: &str, api_key: &str) -> Result<Value, crate::Error> {
        let resp = self.client.get(url).query(&[("api_key", api_key)]).send().await?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);

        if !status.is_success() {
            let msg = body["error"]["message"]
                .as_str()
                .or_else(|| body["error"].as_str())
                .or_else(|| body["message"].as_str())
                .unwrap_or("no details");
            return Err(format!("Roboflow API returned {status}: {msg}").into());
        }
        Ok(body)
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
                tokio::time::sleep(EXPORT_POLL_INTERVAL).await;
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
    fn reads_project_info() {
        let body = serde_json::json!({ "project": { "type": "object-detection", "license": "CC BY 4.0" } });
        let info = roboflow_info(&body);
        assert_eq!(info.dataset_type, "object-detection");
        assert_eq!(info.license, "CC BY 4.0");
        assert_eq!(roboflow_info(&serde_json::json!({})), SourceInfo::default());
    }
}
