//! The one HTTP client every outbound call goes through: timeouts, retries
//! for reads, and a log line per call (without query strings, which often
//! carry API keys).

use anyhow::{Context, Result, bail};
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use std::time::Duration;

#[derive(Clone)]
pub struct Net {
    client: reqwest::Client,
}

/// The URL as it may appear in logs: scheme, host and path only.
pub fn redact(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(u) => format!(
            "{}://{}{}",
            u.scheme(),
            u.host_str().unwrap_or("?"),
            u.path()
        ),
        Err(_) => "<bad url>".into(),
    }
}

impl Net {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("Heistarr/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .expect("HTTP client");
        Self { client }
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Sends a request built by `build`, retrying GETs up to twice on
    /// connection errors and 5xx/429 answers. Returns any final response,
    /// successful or not.
    pub async fn send(
        &self,
        method: Method,
        url: &str,
        build: impl Fn(RequestBuilder) -> RequestBuilder,
    ) -> Result<Response> {
        let attempts = if method == Method::GET { 3 } else { 1 };
        let shown = redact(url);
        let mut last_err = None;
        for attempt in 1..=attempts {
            if attempt > 1 {
                tokio::time::sleep(Duration::from_millis(500 * (1 << (attempt - 2)))).await;
            }
            let started = std::time::Instant::now();
            match build(self.client.request(method.clone(), url)).send().await {
                Ok(r) => {
                    let st = r.status();
                    tracing::debug!(target: "heistarr::net", %method, url = %shown, status = st.as_u16(), ms = started.elapsed().as_millis() as u64);
                    if attempt < attempts
                        && (st.is_server_error() || st == StatusCode::TOO_MANY_REQUESTS)
                    {
                        last_err = Some(anyhow::anyhow!("{shown}: HTTP {st}"));
                        continue;
                    }
                    return Ok(r);
                }
                Err(e) => {
                    let why = describe(&e);
                    tracing::debug!(target: "heistarr::net", %method, url = %shown, error = %why);
                    last_err = Some(anyhow::anyhow!("{shown}: {why}"));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("{shown}: request failed")))
    }

    /// GET and require a 2xx answer; returns the body.
    pub async fn get_bytes(
        &self,
        url: &str,
        build: impl Fn(RequestBuilder) -> RequestBuilder,
    ) -> Result<bytes::Bytes> {
        let r = ok(self.send(Method::GET, url, build).await?, url).await?;
        r.bytes().await.with_context(|| redact(url))
    }

    pub async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        build: impl Fn(RequestBuilder) -> RequestBuilder,
    ) -> Result<T> {
        let b = self.get_bytes(url, build).await?;
        serde_json::from_slice(&b).with_context(|| format!("{}: unexpected answer", redact(url)))
    }
}

impl Default for Net {
    fn default() -> Self {
        Self::new()
    }
}

/// Turns a non-2xx response into an error with a short body excerpt.
pub async fn ok(r: Response, url: &str) -> Result<Response> {
    let st = r.status();
    if st.is_success() {
        return Ok(r);
    }
    let body = r.text().await.unwrap_or_default();
    let excerpt: String = body.chars().take(200).collect();
    match st {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            bail!("{}: HTTP {st} (check the API key or password)", redact(url))
        }
        _ => bail!("{}: HTTP {st} {}", redact(url), excerpt.trim()),
    }
}

fn describe(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "timed out".into()
    } else if e.is_connect() {
        "cannot connect".into()
    } else {
        e.without_url_ref()
    }
}

trait WithoutUrlRef {
    fn without_url_ref(&self) -> String;
}

impl WithoutUrlRef for reqwest::Error {
    fn without_url_ref(&self) -> String {
        let mut s = self.to_string();
        if let Some(u) = self.url() {
            s = s.replace(u.as_str(), &redact(u.as_str()));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn redact_drops_query() {
        assert_eq!(
            super::redact("http://j:9117/api?apikey=SECRET&t=search"),
            "http://j/api"
        );
    }
}
