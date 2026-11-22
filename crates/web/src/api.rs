//! Calls to the Heistarr server. Every state-changing call carries the
//! `X-Heistarr` header the server requires (CSRF protection).

use gloo_net::http::{Request, RequestBuilder, Response};
use heistarr_core::api::ApiError;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// An error to show the user, plus whether the session is gone.
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

fn net_err(e: gloo_net::Error) -> Error {
    Error {
        status: 0,
        message: format!("cannot reach the server: {e}"),
    }
}

async fn finish(r: Response) -> Result<Response> {
    if r.ok() {
        return Ok(r);
    }
    let status = r.status();
    let message = match r.json::<ApiError>().await {
        Ok(e) => e.error,
        Err(_) => format!("HTTP {status}"),
    };
    if status == 401 {
        crate::session_lost();
    }
    Err(Error { status, message })
}

fn url(path: &str) -> String {
    format!("/api{path}")
}

fn with_csrf(r: RequestBuilder) -> RequestBuilder {
    r.header("X-Heistarr", "1")
}

pub async fn get<T: DeserializeOwned>(path: &str) -> Result<T> {
    let r = Request::get(&url(path)).send().await.map_err(net_err)?;
    let r = finish(r).await?;
    r.json().await.map_err(net_err)
}

async fn send_json<B: Serialize>(r: RequestBuilder, body: &B) -> Result<Response> {
    let r = with_csrf(r)
        .json(body)
        .map_err(net_err)?
        .send()
        .await
        .map_err(net_err)?;
    finish(r).await
}

pub async fn post<B: Serialize, T: DeserializeOwned>(path: &str, body: &B) -> Result<T> {
    send_json(Request::post(&url(path)), body)
        .await?
        .json()
        .await
        .map_err(net_err)
}

/// POST that expects no answer body (204/202).
pub async fn post_unit<B: Serialize>(path: &str, body: &B) -> Result<()> {
    send_json(Request::post(&url(path)), body).await.map(|_| ())
}

pub async fn put<B: Serialize, T: DeserializeOwned>(path: &str, body: &B) -> Result<T> {
    send_json(Request::put(&url(path)), body)
        .await?
        .json()
        .await
        .map_err(net_err)
}

pub async fn delete(path: &str) -> Result<()> {
    let r = with_csrf(Request::delete(&url(path)))
        .send()
        .await
        .map_err(net_err)?;
    finish(r).await.map(|_| ())
}

pub fn poster_url(path: &Option<String>, size: &str) -> Option<String> {
    path.as_ref().map(|p| format!("/api/poster/{size}{p}"))
}

pub fn enc(s: &str) -> String {
    js_sys_encode(s)
}

fn js_sys_encode(s: &str) -> String {
    // Percent-encode everything but unreserved characters.
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
