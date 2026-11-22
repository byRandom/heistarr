//! Synology Download Station client (DSM 6 and 7 Web API).
//!
//! API paths are discovered with `SYNO.API.Info`. Tasks are created with
//! `SYNO.DownloadStation2.Task` when available (DSM 7, returns the task id)
//! and with `SYNO.DownloadStation.Task` otherwise; listing and deleting use
//! `SYNO.DownloadStation.Task`, which both versions have.

use crate::net::{Net, redact};
use anyhow::{Context, Result, bail};
use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::Mutex;

#[derive(Default)]
pub struct Session {
    inner: Mutex<Option<Cached>>,
}

#[derive(Clone)]
struct Cached {
    key: String,
    sid: String,
    paths: HashMap<String, ApiInfo>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiInfo {
    path: String,
    #[serde(rename = "maxVersion")]
    max_version: u32,
}

pub struct Ds<'a> {
    pub net: &'a Net,
    pub session: &'a Session,
    pub url: &'a str,
    pub username: &'a str,
    pub password: &'a str,
    /// Shared folder path where tasks are saved, e.g. `data/torrents`.
    /// Empty: Download Station's default folder.
    pub destination: &'a str,
}

#[derive(Clone, Debug)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub status: String,
    pub size: u64,
    pub downloaded: u64,
    /// `data/torrents` (shared folder relative, no leading slash).
    pub destination: String,
    pub uri: String,
    pub error: Option<String>,
}

impl Task {
    pub fn progress(&self) -> f32 {
        if self.is_complete() {
            1.0
        } else if self.size == 0 {
            0.0
        } else {
            (self.downloaded as f64 / self.size as f64).min(1.0) as f32
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self.status.as_str(), "finished" | "seeding")
    }

    pub fn is_error(&self) -> bool {
        self.status == "error"
    }

    /// Where the task's files are, as an absolute path on the NAS
    /// (`/data/torrents/Title`); map it with the remote/local path setting.
    pub fn content_path(&self) -> String {
        format!("/{}/{}", self.destination.trim_matches('/'), self.title)
    }
}

/// Synology error codes worth a readable message.
fn explain(api: &str, code: i64) -> String {
    let m = match (api, code) {
        (_, 101) => "invalid parameter",
        (_, 102) => "API not available: is Download Station installed?",
        (_, 103) => "method not available",
        (_, 104) => "version not supported",
        (_, 105) => "permission denied: the user needs Download Station access",
        (_, 106) => "session timed out",
        (_, 107) => "session interrupted by a duplicate login",
        (_, 119) => "invalid session",
        ("SYNO.API.Auth", 400) => "wrong username or password",
        ("SYNO.API.Auth", 401) => "account disabled",
        ("SYNO.API.Auth", 402) => "permission denied",
        ("SYNO.API.Auth", 403 | 404 | 406) => {
            "two-step verification is on: use an account without it for Heistarr"
        }
        ("SYNO.API.Auth", 407) => "too many failed logins: the IP is blocked for a while",
        (_, 400) => "file upload failed",
        (_, 401) => "maximum number of tasks reached",
        (_, 402) => "destination denied: check the shared folder and the user's permissions",
        (_, 403) => "destination does not exist",
        (_, 404) => "invalid task id",
        (_, 405) => "invalid task action",
        (_, 406) => "no default destination: set the destination folder in Settings",
        (_, 408) => "file does not exist",
        _ => "",
    };
    if m.is_empty() {
        format!("Synology {api} error {code}")
    } else {
        format!("Synology: {m} ({code})")
    }
}

fn session_expired(code: i64) -> bool {
    matches!(code, 106 | 107 | 119)
}

impl Ds<'_> {
    fn base(&self) -> String {
        self.url.trim().trim_end_matches('/').to_string()
    }

    async fn call_raw(&self, path: &str, params: &[(&str, String)], post: bool) -> Result<Value> {
        let url = format!("{}/webapi/{path}", self.base());
        let r = if post {
            self.net
                .send(Method::POST, &url, |r| r.form(params))
                .await?
        } else {
            self.net
                .send(Method::GET, &url, |r| r.query(params))
                .await?
        };
        let r = crate::net::ok(r, &url).await?;
        let v: Value = r
            .json()
            .await
            .with_context(|| format!("{}: not a Synology answer", redact(&url)))?;
        Ok(v)
    }

    async fn connect(&self) -> Result<Cached> {
        if self.base().is_empty() {
            bail!("no Synology URL: add it in Settings");
        }
        let info = self
            .call_raw(
                "query.cgi",
                &[
                    ("api", "SYNO.API.Info".into()),
                    ("version", "1".into()),
                    ("method", "query".into()),
                    (
                        "query",
                        "SYNO.API.Auth,SYNO.DownloadStation.Task,SYNO.DownloadStation2.Task".into(),
                    ),
                ],
                false,
            )
            .await?;
        if info["success"] != Value::Bool(true) {
            bail!(
                "the Synology did not answer the API query: is the URL right (e.g. http://nas:5000)?"
            );
        }
        let paths: HashMap<String, ApiInfo> = serde_json::from_value(info["data"].clone())
            .context("Synology: unexpected API list")?;
        if !paths.contains_key("SYNO.DownloadStation.Task") {
            bail!("Download Station is not installed or not running on the Synology");
        }
        let auth = paths
            .get("SYNO.API.Auth")
            .context("Synology: no auth API")?;
        let v = self
            .call_raw(
                &auth.path,
                &[
                    ("api", "SYNO.API.Auth".into()),
                    ("version", auth.max_version.min(6).to_string()),
                    ("method", "login".into()),
                    ("account", self.username.to_string()),
                    ("passwd", self.password.to_string()),
                    ("session", "DownloadStation".into()),
                    ("format", "sid".into()),
                ],
                true,
            )
            .await?;
        if v["success"] != Value::Bool(true) {
            bail!(
                "{}",
                explain("SYNO.API.Auth", v["error"]["code"].as_i64().unwrap_or(0))
            );
        }
        let sid = v["data"]["sid"]
            .as_str()
            .context("Synology: no session id")?
            .to_string();
        Ok(Cached {
            key: self.key(),
            sid,
            paths,
        })
    }

    fn key(&self) -> String {
        format!("{}|{}", self.base(), self.username)
    }

    async fn cached(&self, fresh: bool) -> Result<Cached> {
        let mut g = self.session.inner.lock().await;
        if !fresh
            && let Some(c) = g.as_ref()
            && c.key == self.key()
        {
            return Ok(c.clone());
        }
        let c = self.connect().await?;
        *g = Some(c.clone());
        Ok(c)
    }

    /// Calls `api`/`method`, logging in again once if the session expired.
    async fn call(
        &self,
        api: &str,
        version: u32,
        method: &str,
        params: &[(&str, String)],
        post: bool,
    ) -> Result<Value> {
        for fresh in [false, true] {
            let c = self.cached(fresh).await?;
            let info = c
                .paths
                .get(api)
                .with_context(|| format!("Synology: {api} not available"))?;
            let mut p: Vec<(&str, String)> = vec![
                ("api", api.into()),
                ("version", version.min(info.max_version).to_string()),
                ("method", method.into()),
                ("_sid", c.sid.clone()),
            ];
            p.extend_from_slice(params);
            let v = self.call_raw(&info.path, &p, post).await?;
            if v["success"] == Value::Bool(true) {
                return Ok(v["data"].clone());
            }
            let code = v["error"]["code"].as_i64().unwrap_or(0);
            if session_expired(code) && !fresh {
                continue;
            }
            bail!("{}", explain(api, code));
        }
        unreachable!()
    }

    pub async fn test(&self) -> Result<String> {
        self.cached(true).await?;
        let d = self
            .call(
                "SYNO.DownloadStation.Task",
                1,
                "list",
                &[("limit", "1".into())],
                false,
            )
            .await?;
        Ok(format!(
            "Connected to Download Station ({} tasks)",
            d["total"].as_i64().unwrap_or(0)
        ))
    }

    /// Adds a `.torrent` URL or magnet link. Returns the task id when the
    /// NAS reports it (DSM 7).
    pub async fn add(&self, uri: &str) -> Result<Option<String>> {
        let c = self.cached(false).await?;
        let dest = self.destination.trim().trim_matches('/').to_string();
        if c.paths.contains_key("SYNO.DownloadStation2.Task") {
            let mut p = vec![
                ("type", "\"url\"".to_string()),
                ("url", serde_json::to_string(&[uri])?),
                ("create_list", "false".into()),
            ];
            // DS2 requires a destination; an empty one means the default.
            p.push(("destination", serde_json::to_string(&dest)?));
            match self
                .call("SYNO.DownloadStation2.Task", 2, "create", &p, true)
                .await
            {
                Ok(d) => {
                    return Ok(d["task_id"]
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(|v| v.as_str())
                        .map(str::to_string));
                }
                Err(e) => {
                    tracing::info!("DownloadStation2 create failed ({e:#}); trying the v1 API")
                }
            }
        }
        let mut p = vec![("uri", uri.to_string())];
        if !dest.is_empty() {
            p.push(("destination", dest));
        }
        self.call("SYNO.DownloadStation.Task", 1, "create", &p, true)
            .await?;
        Ok(None)
    }

    pub async fn tasks(&self) -> Result<Vec<Task>> {
        let d = self
            .call(
                "SYNO.DownloadStation.Task",
                1,
                "list",
                &[("additional", "detail,transfer".into())],
                false,
            )
            .await?;
        let list = d["tasks"].as_array().cloned().unwrap_or_default();
        Ok(list
            .iter()
            .map(|t| Task {
                id: t["id"].as_str().unwrap_or_default().to_string(),
                title: t["title"].as_str().unwrap_or_default().to_string(),
                status: t["status"].as_str().unwrap_or_default().to_string(),
                size: t["size"].as_u64().unwrap_or(0),
                downloaded: t["additional"]["transfer"]["size_downloaded"]
                    .as_u64()
                    .unwrap_or(0),
                destination: t["additional"]["detail"]["destination"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                uri: t["additional"]["detail"]["uri"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                error: t["status_extra"]["error_detail"]
                    .as_str()
                    .map(str::to_string),
            })
            .collect())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        self.call(
            "SYNO.DownloadStation.Task",
            1,
            "delete",
            &[("id", id.to_string()), ("force_complete", "false".into())],
            true,
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_paths_and_progress() {
        let t = Task {
            id: "dbid_1".into(),
            title: "Heat.1995.1080p".into(),
            status: "downloading".into(),
            size: 200,
            downloaded: 50,
            destination: "data/torrents".into(),
            uri: String::new(),
            error: None,
        };
        assert_eq!(t.content_path(), "/data/torrents/Heat.1995.1080p");
        assert_eq!(t.progress(), 0.25);
        assert!(explain("SYNO.API.Auth", 400).contains("wrong username"));
    }
}
