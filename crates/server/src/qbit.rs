//! qBittorrent Web UI API (v2) client.

use crate::net::{Net, redact};
use anyhow::{Context, Result, bail};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use tokio::sync::Mutex;

/// Login cookie cache, kept in the app state so we log in once.
#[derive(Default)]
pub struct Session {
    sid: Mutex<Option<(String, String)>>, // (base url + user, SID)
}

pub struct Qbit<'a> {
    pub net: &'a Net,
    pub session: &'a Session,
    pub url: &'a str,
    pub username: &'a str,
    pub password: &'a str,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Torrent {
    pub hash: String,
    pub name: String,
    pub progress: f32,
    pub state: String,
    #[serde(default)]
    pub content_path: String,
    #[serde(default)]
    pub save_path: String,
    #[serde(default)]
    pub tags: String,
}

impl Torrent {
    pub fn is_complete(&self) -> bool {
        self.progress >= 1.0
            || matches!(
                self.state.as_str(),
                "uploading"
                    | "stalledUP"
                    | "pausedUP"
                    | "stoppedUP"
                    | "queuedUP"
                    | "forcedUP"
                    | "checkingUP"
            )
    }

    pub fn is_error(&self) -> bool {
        matches!(self.state.as_str(), "error" | "missingFiles")
    }

    pub fn is_stalled(&self) -> bool {
        matches!(self.state.as_str(), "stalledDL" | "metaDL")
    }
}

/// The tag that ties a torrent to one of our downloads.
pub fn tag_for(download_id: i64) -> String {
    format!("heistarr-{download_id}")
}

impl Qbit<'_> {
    fn base(&self) -> String {
        self.url.trim().trim_end_matches('/').to_string()
    }

    async fn login(&self) -> Result<String> {
        let base = self.base();
        if base.is_empty() {
            bail!("no qBittorrent URL: add it in Settings");
        }
        let url = format!("{base}/api/v2/auth/login");
        let form = [
            ("username", self.username.to_string()),
            ("password", self.password.to_string()),
        ];
        let r = self
            .net
            .send(Method::POST, &url, |r| {
                r.header("Referer", &base).form(&form)
            })
            .await?;
        let st = r.status();
        if st == StatusCode::FORBIDDEN {
            bail!(
                "qBittorrent refused the login: too many failed attempts, the IP is banned for a while"
            );
        }
        let sid = r
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|v| {
                v.strip_prefix("SID=")
                    .map(|rest| rest.split(';').next().unwrap_or("").to_string())
            });
        let body = r.text().await.unwrap_or_default();
        match sid {
            Some(sid) if st.is_success() => Ok(sid),
            // Auth bypassed for this client (localhost/whitelist): no cookie.
            None if st.is_success() && body.trim() == "Ok." => Ok(String::new()),
            _ => bail!("qBittorrent login failed: wrong username or password"),
        }
    }

    async fn sid(&self, fresh: bool) -> Result<String> {
        let key = format!("{}|{}", self.base(), self.username);
        let mut g = self.session.sid.lock().await;
        if !fresh
            && let Some((k, sid)) = g.as_ref()
            && *k == key
        {
            return Ok(sid.clone());
        }
        let sid = self.login().await?;
        *g = Some((key, sid.clone()));
        Ok(sid)
    }

    /// Calls `path`, logging in (again) when the session is missing or expired.
    async fn call(
        &self,
        method: Method,
        path: &str,
        build: impl Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response> {
        let base = self.base();
        let url = format!("{base}{path}");
        for fresh in [false, true] {
            let sid = self.sid(fresh).await?;
            let r = self
                .net
                .send(method.clone(), &url, |r| {
                    let r = build(r.header("Referer", &base));
                    if sid.is_empty() {
                        r
                    } else {
                        r.header("Cookie", format!("SID={sid}"))
                    }
                })
                .await?;
            if r.status() == StatusCode::FORBIDDEN && !fresh {
                continue;
            }
            return crate::net::ok(r, &url).await;
        }
        bail!("{}: not authorised", redact(&url))
    }

    /// Logs in again (ignoring the cached session) and reads the version:
    /// what the "Test" button does.
    pub async fn test(&self) -> Result<String> {
        self.sid(true).await?;
        self.version().await
    }

    pub async fn version(&self) -> Result<String> {
        Ok(self
            .call(Method::GET, "/api/v2/app/version", |r| r)
            .await?
            .text()
            .await?)
    }

    pub async fn ensure_category(&self, category: &str) -> Result<()> {
        let base = self.base();
        let url = format!("{base}/api/v2/torrents/createCategory");
        let sid = self.sid(false).await?;
        // 409 means it already exists.
        let r = self
            .net
            .send(Method::POST, &url, |r| {
                let r = r
                    .header("Referer", &base)
                    .form(&[("category", category), ("savePath", "")]);
                if sid.is_empty() {
                    r
                } else {
                    r.header("Cookie", format!("SID={sid}"))
                }
            })
            .await?;
        if !r.status().is_success() && r.status() != StatusCode::CONFLICT {
            tracing::warn!("qBittorrent createCategory: HTTP {}", r.status());
        }
        Ok(())
    }

    pub async fn add(&self, download_url: &str, category: &str, tag: &str) -> Result<()> {
        if !category.is_empty() {
            self.ensure_category(category).await?;
        }
        let r = self
            .call(Method::POST, "/api/v2/torrents/add", |r| {
                let form = reqwest::multipart::Form::new()
                    .text("urls", download_url.to_string())
                    .text("category", category.to_string())
                    .text("tags", tag.to_string());
                r.multipart(form)
            })
            .await?;
        let body = r.text().await.unwrap_or_default();
        if body.trim() == "Fails." {
            bail!("qBittorrent could not add the torrent");
        }
        Ok(())
    }

    pub async fn torrents(&self, category: &str) -> Result<Vec<Torrent>> {
        let r = self
            .call(Method::GET, "/api/v2/torrents/info", |r| {
                if category.is_empty() {
                    r
                } else {
                    r.query(&[("category", category)])
                }
            })
            .await?;
        r.json()
            .await
            .context("qBittorrent: unexpected torrent list")
    }

    pub async fn delete(&self, hash: &str, delete_files: bool) -> Result<()> {
        self.call(Method::POST, "/api/v2/torrents/delete", |r| {
            r.form(&[
                ("hashes", hash),
                ("deleteFiles", if delete_files { "true" } else { "false" }),
            ])
        })
        .await?;
        Ok(())
    }
}
