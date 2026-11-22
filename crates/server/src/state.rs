//! Shared application state and the API error type.

use crate::db::Db;
use crate::net::Net;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use heistarr_core::api::ApiError;
use std::path::PathBuf;
use std::sync::Arc;

/// Where things live and which external endpoints to call. Tests point the
/// endpoints at fake servers.
#[derive(Clone, Debug)]
pub struct Config {
    pub data_dir: PathBuf,
    /// Built web UI (index.html, wasm, css). May be missing in tests.
    pub web_dir: PathBuf,
    pub tmdb_api: String,
    pub tmdb_images: String,
    /// Send the session cookie with `Secure` (behind an HTTPS proxy).
    pub secure_cookie: bool,
}

pub struct AppState {
    pub db: Db,
    pub net: Net,
    pub cfg: Config,
    pub qbit: crate::qbit::Session,
    pub synology: crate::synology::Session,
    pub limiter: crate::auth::Limiter,
    /// Held while deciding whether to grab, so two searches never grab the
    /// same movie twice.
    pub grab_lock: tokio::sync::Mutex<()>,
    /// Wakes the scheduler early (after a grab, to track it sooner).
    pub wake: tokio::sync::Notify,
}

pub type App = Arc<AppState>;

impl AppState {
    pub fn new(db: Db, cfg: Config) -> App {
        Arc::new(Self {
            db,
            net: Net::new(),
            cfg,
            qbit: Default::default(),
            synology: Default::default(),
            limiter: Default::default(),
            grab_lock: Default::default(),
            wake: Default::default(),
        })
    }
}

/// An error answered as `{"error": "..."}` with a status code.
#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub message: String,
}

impl AppError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn bad(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn not_found(what: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, format!("{what} not found"))
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        // `{:#}` shows the context chain on one line.
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: format!("{e:#}"),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if self.status.is_server_error() {
            tracing::warn!("{}: {}", self.status, self.message);
        }
        (
            self.status,
            Json(ApiError {
                error: self.message,
            }),
        )
            .into_response()
    }
}

pub type ApiResult<T> = Result<Json<T>, AppError>;
