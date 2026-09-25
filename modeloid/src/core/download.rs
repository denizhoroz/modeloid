use std::{env, error::Error, path::Path, time::Duration};

use serde_json::Value;
use url::Url;

use crate::core::io;

const ROBOFLOW_API: &str = "https://api.roboflow.com";
const ROBOFLOW_KEY_ENV: &str = "ROBOFLOW_API_KEY";
const EXPORT_POLL_ATTEMPTS: u32 = 20;
const EXPORT_POLL_INTERVAL: Duration = Duration::from_secs(3);

pub struct DownloadOptions {
    pub api_key: Option<String>,
    pub format: String,
}

#[derive(Debug, PartialEq)]pub enum Source {
    Roboflow {
        workspace: String,
        project: String,
        /// `None` means the latest version, resolved by `Downloader::inspect`.
        version: Option<u32>,
    },
}

impl Source {
    pub fn from_link(link: &str) -> Result<Self, Box<dyn Error>> {
        let url = Url::parse(link)?;
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
            _ => Err(format!("unsupported dataset source: {host}").into()),
        }
    }

    fn roboflow(ws: &str, project: &str, version: Option<&str>) -> Result<Self, Box<dyn Error>> {
        let version = version
            .map(|v| v.parse().map_err(|_| format!("invalid Roboflow version: {v}")))
            .transpose()?;
        Ok(Source::Roboflow {
            workspace: ws.to_string(),
            project: project.to_string(),
            version,
        })
    }

    pub fn name(&self, opts: &DownloadOptions) -> String {
        match self {
            Source::Roboflow { project, version: Some(v), .. } => format!("{project}-v{v}-{}", opts.format),
            Source::Roboflow { project, version: None, .. } => format!("{project}-latest-{}", opts.format),
        }
    }
}

/// Metadata the source reports about a dataset; empty strings when unknown.
#[derive(Debug, Default, PartialEq)]
pub struct SourceInfo {
    pub dataset_type: String,
    pub license: String,
}

pub struct Downloader {
    client: reqwest::Client,
}

impl Downloader {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("modeloid/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .build()?;
        Ok(Downloader { client })
    }

    /// Fetches project metadata and replaces an unspecified version with the latest one available.
    pub async fn inspect(&self, source: Source, opts: &DownloadOptions) -> Result<(Source, SourceInfo), Box<dyn Error>> {
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
                        eprintln!("Using latest version: {latest}");
                        latest
                    }
                };
                let info = roboflow_info(&body);
                Ok((Source::Roboflow { workspace, project, version: Some(version) }, info))
            }
        }
    }

    /// Returns a direct URL to the dataset archive.
    pub async fn resolve(&self, source: &Source, opts: &DownloadOptions) -> Result<String, Box<dyn Error>> {
        match source {
            Source::Roboflow { workspace, project, version } => {
                let version = version.ok_or("Roboflow version not resolved, call inspect first")?;
                let api_key = roboflow_api_key(opts)?;
                let url = format!("{ROBOFLOW_API}/{workspace}/{project}/{version}/{}", opts.format);
                self.roboflow_export_link(&url, &api_key).await
            }
        }
    }

    async fn roboflow_get(&self, url: &str, api_key: &str) -> Result<Value, Box<dyn Error>> {
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
    async fn roboflow_export_link(&self, url: &str, api_key: &str) -> Result<String, Box<dyn Error>> {
        for attempt in 1..=EXPORT_POLL_ATTEMPTS {
            let body = self.roboflow_get(url, api_key).await?;

            if let Some(link) = body["export"]["link"].as_str() {
                return Ok(link.to_string());
            }

            eprintln!("Waiting for Roboflow export ({attempt}/{EXPORT_POLL_ATTEMPTS})...");
            tokio::time::sleep(EXPORT_POLL_INTERVAL).await;
        }
        Err("Roboflow export was not ready in time, try again later".into())
    }

    /// Downloads `url` to `dest`, writing to `dest.part` until complete.
    pub async fn download_file(&self, url: &str, dest: &Path) -> Result<(), Box<dyn Error>> {
        io::download(&self.client, url, dest).await?;
        Ok(())
    }
}

fn roboflow_api_key(opts: &DownloadOptions) -> Result<String, Box<dyn Error>> {
    match &opts.api_key {
        Some(key) => Ok(key.clone()),
        None => env::var(ROBOFLOW_KEY_ENV)
            .map_err(|_| format!("Roboflow API key missing: pass --api-key or set {ROBOFLOW_KEY_ENV}").into()),
    }
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
    fn rejects_unknown_host() {
        assert!(Source::from_link("https://example.com/a/b/c").is_err());
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
