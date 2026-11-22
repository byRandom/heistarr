//! `heistarr`: the server binary.
//!
//! Environment:
//! - `HEISTARR_DATA`  data folder (database, master key, logs, cache). Default `./data`.
//! - `HEISTARR_PORT`  default 6464. `HEISTARR_BIND` default `0.0.0.0`.
//! - `HEISTARR_WEB`   built web UI folder. Default `./web` next to the binary, else `crates/web/dist`.
//! - `HEISTARR_MASTER_KEY` base64 key for stored secrets (else `master.key` in the data folder).
//! - `HEISTARR_SECURE_COOKIE=1` when served over HTTPS (reverse proxy).
//! - `RUST_LOG` log filter, default `info`.

use anyhow::{Context, Result};
use heistarr_server::{crypto, db, jobs, routes, state};
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn web_dir() -> PathBuf {
    if let Some(w) = env("HEISTARR_WEB") {
        return w.into();
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
        && dir.join("web/index.html").exists()
    {
        return dir.join("web");
    }
    PathBuf::from("crates/web/dist")
}

#[tokio::main]
async fn main() -> Result<()> {
    let data_dir = PathBuf::from(env("HEISTARR_DATA").unwrap_or_else(|| "data".into()));
    std::fs::create_dir_all(data_dir.join("logs"))
        .with_context(|| format!("creating {}", data_dir.display()))?;

    // Logs: stdout plus a daily-rotated file, 14 files kept.
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("heistarr")
        .filename_suffix("log")
        .max_log_files(14)
        .build(data_dir.join("logs"))
        .context("log file")?;
    let (file, _guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer())
        .with(fmt::layer().with_ansi(false).with_writer(file))
        .init();

    let cipher = crypto::Cipher::load(&data_dir)?;
    let db = db::Db::open(&data_dir.join("heistarr.db"), cipher)?;
    let cfg = state::Config {
        web_dir: web_dir(),
        tmdb_api: env("HEISTARR_TMDB_API")
            .unwrap_or_else(|| heistarr_server::tmdb::DEFAULT_API.into()),
        tmdb_images: env("HEISTARR_TMDB_IMAGES")
            .unwrap_or_else(|| heistarr_server::tmdb::DEFAULT_IMAGES.into()),
        secure_cookie: env("HEISTARR_SECURE_COOKIE").is_some_and(|v| v == "1" || v == "true"),
        data_dir,
    };
    if !cfg.web_dir.join("index.html").exists() {
        tracing::warn!(
            "no web UI at {} (set HEISTARR_WEB); the API still works",
            cfg.web_dir.display()
        );
    }
    let app = state::AppState::new(db, cfg);
    jobs::spawn(app.clone());

    let port: u16 = env("HEISTARR_PORT")
        .map(|p| p.parse())
        .transpose()
        .context("HEISTARR_PORT")?
        .unwrap_or(6464);
    let bind = env("HEISTARR_BIND").unwrap_or_else(|| "0.0.0.0".into());
    let addr: SocketAddr = format!("{bind}:{port}").parse().context("HEISTARR_BIND")?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("listening on {addr}"))?;
    tracing::info!(
        "Heistarr {} listening on http://{addr}",
        env!("CARGO_PKG_VERSION")
    );
    axum::serve(
        listener,
        routes::router(app).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("signal");
            tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
        }
        #[cfg(not(unix))]
        ctrl_c.await.ok();
        tracing::info!("shutting down");
    })
    .await?;
    Ok(())
}
