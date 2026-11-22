//! The HTTP API used by the web UI (`/api/...`), the poster proxy, health
//! check and the static web files.

use crate::auth::{self, CurrentUser, Owner};
use crate::client::Client;
use crate::db::secret_fields;
use crate::engine;
use crate::state::{ApiResult, App, AppError};
use crate::{mediaserver, tmdb, torznab};
use axum::Extension;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use heistarr_core::api::*;
use serde::Deserialize;
use std::net::SocketAddr;

pub fn router(app: App) -> Router {
    let api = Router::new()
        .route("/setup", get(setup_state).post(setup))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
        .route("/settings", get(get_settings).put(put_settings))
        .route("/settings/test/{what}", post(test_settings))
        .route("/indexers", get(list_indexers).post(save_indexer))
        .route("/indexers/test", post(test_indexer))
        .route("/indexers/{id}", delete(delete_indexer))
        .route("/catalog/search", get(catalog_search))
        .route("/movies", get(list_movies).post(add_movie))
        .route("/movies/{id}", get(movie_detail).delete(delete_movie))
        .route("/movies/{id}/search", post(search_movie))
        .route("/movies/{id}/releases", get(movie_releases))
        .route("/grab", post(grab))
        .route("/blocklist/{id}", delete(unblock))
        .route("/downloads", get(list_downloads))
        .route("/downloads/{id}/remove", post(remove_download))
        .route("/history", get(history))
        .route("/library/scan", post(scan_library))
        .route("/poster/{size}/{file}", get(poster))
        .fallback(|| async { AppError::not_found("API route") })
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            auth::csrf,
        ));

    let web_dir = app.cfg.web_dir.clone();
    let index = web_dir.join("index.html");
    let static_files = tower_http::services::ServeDir::new(&web_dir)
        .not_found_service(tower_http::services::ServeFile::new(index));

    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api", api)
        .fallback_service(static_files)
        .layer(
            tower_http::set_header::SetResponseHeaderLayer::if_not_present(
                header::X_FRAME_OPTIONS,
                HeaderValue::from_static("DENY"),
            ),
        )
        .layer(
            tower_http::set_header::SetResponseHeaderLayer::if_not_present(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
        )
        .layer(
            tower_http::set_header::SetResponseHeaderLayer::if_not_present(
                header::REFERRER_POLICY,
                HeaderValue::from_static("same-origin"),
            ),
        )
        .with_state(app)
}

// ---- accounts ----

async fn setup_state(State(app): State<App>) -> ApiResult<SetupState> {
    Ok(Json(SetupState {
        needs_owner: app.db.user_count()? == 0,
    }))
}

fn with_session(app: &App, user_id: i64, body: Me) -> Result<Response, AppError> {
    let token = auth::new_token();
    app.db.create_session(&auth::token_hash(&token), user_id)?;
    let mut r = Json(body).into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&auth::session_cookie(&token, app.cfg.secure_cookie))
            .map_err(|e| anyhow::anyhow!("{e}"))?,
    );
    Ok(r)
}

async fn setup(State(app): State<App>, Json(b): Json<Login>) -> Result<Response, AppError> {
    auth::validate_new_account(&b.username, &b.password)?;
    let username = b.username.trim().to_string();
    let hash = auth::hash_password(&b.password)?;
    let Some(id) = app.db.create_first_owner(&username, &hash)? else {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "the owner account already exists: sign in",
        ));
    };
    app.db
        .log("account", &format!("Owner account \"{username}\" created"));
    with_session(
        &app,
        id,
        Me {
            username,
            is_owner: true,
        },
    )
}

fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    let _ = headers; // X-Forwarded-For is spoofable; the peer is what we trust.
    peer.map(|p| p.ip().to_string())
        .unwrap_or_else(|| "unknown".into())
}

async fn login(
    State(app): State<App>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(b): Json<Login>,
) -> Result<Response, AppError> {
    let ip = format!("ip:{}", client_ip(&headers, peer.map(|Extension(c)| c.0)));
    let user_key = format!("user:{}", b.username.trim().to_lowercase());
    let keys = [ip.as_str(), user_key.as_str()];
    if app.limiter.blocked(&keys) {
        return Err(AppError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too many failed sign-ins: wait 15 minutes",
        ));
    }
    let user = app.db.user_by_name(b.username.trim())?;
    let hash = user
        .as_ref()
        .map(|u| u.password_hash.clone())
        .unwrap_or_else(|| auth::dummy_hash().to_string());
    let pw = b.password.clone();
    let ok = tokio::task::spawn_blocking(move || auth::verify_password(&pw, &hash))
        .await
        .unwrap_or(false);
    match user {
        Some(u) if ok => {
            app.limiter.clear(&keys);
            with_session(
                &app,
                u.id,
                Me {
                    username: u.username,
                    is_owner: u.is_owner,
                },
            )
        }
        _ => {
            app.limiter.fail(&keys);
            tracing::warn!("failed sign-in for \"{}\" from {ip}", b.username.trim());
            Err(AppError::new(
                StatusCode::UNAUTHORIZED,
                "wrong username or password",
            ))
        }
    }
}

async fn logout(State(app): State<App>, headers: HeaderMap) -> Result<Response, AppError> {
    if let Some(t) = auth::cookie_token(&headers) {
        app.db.delete_session(&auth::token_hash(&t))?;
    }
    let mut r = StatusCode::NO_CONTENT.into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&auth::clear_cookie()).expect("cookie"),
    );
    Ok(r)
}

async fn me(CurrentUser(u): CurrentUser) -> ApiResult<Me> {
    Ok(Json(Me {
        username: u.username,
        is_owner: u.is_owner,
    }))
}

// ---- settings ----

fn masked(mut s: Settings) -> Settings {
    for f in secret_fields(&mut s) {
        *f = mask_secret(f);
    }
    s
}

/// Settings from the UI, with masked secrets replaced by the stored ones.
fn unmask(app: &App, mut new: Settings) -> Result<Settings, AppError> {
    let mut old = app.db.settings()?;
    for (n, o) in secret_fields(&mut new)
        .into_iter()
        .zip(secret_fields(&mut old))
    {
        if is_masked(n) {
            *n = o.clone();
        }
    }
    Ok(new)
}

fn validate_settings(s: &Settings) -> Result<(), AppError> {
    for (name, url) in [
        ("qBittorrent URL", &s.qbit_url),
        ("Synology URL", &s.synology_url),
        ("media server URL", &s.media_server_url),
    ] {
        let u = url.trim();
        if !u.is_empty() && !(u.starts_with("http://") || u.starts_with("https://")) {
            return Err(AppError::bad(format!(
                "the {name} must start with http:// or https://"
            )));
        }
    }
    if !matches!(s.download_client.as_str(), "" | "qbittorrent" | "synology") {
        return Err(AppError::bad("unknown download client"));
    }
    if !matches!(
        s.media_server_kind.as_str(),
        "" | "jellyfin" | "emby" | "plex"
    ) {
        return Err(AppError::bad("unknown media server"));
    }
    if s.rules.min_resolution > s.rules.max_resolution {
        return Err(AppError::bad("the minimum resolution is above the maximum"));
    }
    Ok(())
}

async fn get_settings(State(app): State<App>, _o: Owner) -> ApiResult<Settings> {
    Ok(Json(masked(app.db.settings()?)))
}

async fn put_settings(
    State(app): State<App>,
    _o: Owner,
    Json(s): Json<Settings>,
) -> ApiResult<Settings> {
    let s = unmask(&app, s)?;
    validate_settings(&s)?;
    app.db.save_settings(&s)?;
    Ok(Json(masked(s)))
}

/// Tests the settings being edited (not yet saved): `tmdb`, `client` or `media`.
async fn test_settings(
    State(app): State<App>,
    _o: Owner,
    Path(what): Path<String>,
    Json(s): Json<Settings>,
) -> ApiResult<TestResult> {
    let s = unmask(&app, s)?;
    let r: anyhow::Result<String> = match what.as_str() {
        "tmdb" => tmdb::Tmdb {
            net: &app.net,
            base: &app.cfg.tmdb_api,
            key: &s.tmdb_key,
            language: "",
        }
        .test()
        .await
        .map(|()| "TMDB key works".to_string()),
        "client" => Client::new(&app, &s).0.test().await,
        "media" => mediaserver::test(&app.net, &s).await,
        _ => return Err(AppError::not_found("test")),
    };
    Ok(Json(match r {
        Ok(message) => TestResult { ok: true, message },
        Err(e) => TestResult {
            ok: false,
            message: format!("{e:#}"),
        },
    }))
}

// ---- indexers ----

async fn list_indexers(State(app): State<App>, _o: Owner) -> ApiResult<Vec<Indexer>> {
    let mut v = app.db.indexers()?;
    for i in &mut v {
        i.api_key = mask_secret(&i.api_key);
    }
    Ok(Json(v))
}

fn unmask_indexer(app: &App, mut i: Indexer) -> Result<Indexer, AppError> {
    if is_masked(&i.api_key) {
        i.api_key = app.db.indexer(i.id)?.map(|o| o.api_key).unwrap_or_default();
    }
    Ok(i)
}

async fn save_indexer(
    State(app): State<App>,
    _o: Owner,
    Json(i): Json<Indexer>,
) -> ApiResult<Indexer> {
    let mut i = unmask_indexer(&app, i)?;
    i.name = i.name.trim().to_string();
    i.url = i.url.trim().to_string();
    if i.name.is_empty() {
        return Err(AppError::bad("give the indexer a name"));
    }
    if !(i.url.starts_with("http://") || i.url.starts_with("https://")) {
        return Err(AppError::bad("the URL must start with http:// or https://"));
    }
    if i.id != 0 && app.db.indexer(i.id)?.is_none() {
        return Err(AppError::not_found("indexer"));
    }
    i.id = app.db.save_indexer(&i)?;
    i.api_key = mask_secret(&i.api_key);
    Ok(Json(i))
}

async fn test_indexer(
    State(app): State<App>,
    _o: Owner,
    Json(i): Json<Indexer>,
) -> ApiResult<TestResult> {
    let i = unmask_indexer(&app, i)?;
    Ok(Json(match torznab::test(&app.net, &i).await {
        Ok(()) => TestResult {
            ok: true,
            message: "Indexer answered".into(),
        },
        Err(e) => TestResult {
            ok: false,
            message: format!("{e:#}"),
        },
    }))
}

async fn delete_indexer(
    State(app): State<App>,
    _o: Owner,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    app.db.delete_indexer(id)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- catalog and library ----

#[derive(Deserialize)]
struct CatalogQuery {
    q: String,
    year: Option<i32>,
}

async fn catalog_search(
    State(app): State<App>,
    _u: CurrentUser,
    Query(q): Query<CatalogQuery>,
) -> ApiResult<Vec<CatalogMovie>> {
    let text = q.q.trim();
    if text.is_empty() {
        return Ok(Json(vec![]));
    }
    Ok(Json(engine::catalog_search(&app, text, q.year).await?))
}

async fn list_movies(State(app): State<App>, _u: CurrentUser) -> ApiResult<Vec<Movie>> {
    Ok(Json(app.db.movies()?))
}

async fn add_movie(
    State(app): State<App>,
    _o: Owner,
    Json(b): Json<AddMovie>,
) -> Result<(StatusCode, Json<Movie>), AppError> {
    let m = engine::add_movie(&app, b.tmdb_id, b.search_now).await?;
    Ok((StatusCode::CREATED, Json(m)))
}

async fn movie_detail(
    State(app): State<App>,
    _u: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<MovieDetail> {
    let movie = app
        .db
        .movie(id)?
        .ok_or_else(|| AppError::not_found("movie"))?;
    Ok(Json(MovieDetail {
        downloads: app.db.movie_downloads(id)?,
        blocklist: app.db.blocklist(id)?,
        movie,
    }))
}

#[derive(Deserialize)]
struct DeleteQuery {
    #[serde(default)]
    delete_file: bool,
}

async fn delete_movie(
    State(app): State<App>,
    _o: Owner,
    Path(id): Path<i64>,
    Query(q): Query<DeleteQuery>,
) -> Result<StatusCode, AppError> {
    let m = app
        .db
        .movie(id)?
        .ok_or_else(|| AppError::not_found("movie"))?;
    if let Some(d) = app
        .db
        .active_downloads()?
        .into_iter()
        .find(|d| d.movie_id == id)
    {
        engine::remove_download(&app, d.id, false).await?;
    }
    if q.delete_file
        && let Some(f) = &m.file_path
    {
        let p = std::path::PathBuf::from(f);
        tokio::task::spawn_blocking(move || {
            let _ = std::fs::remove_file(&p);
            // Remove the movie folder too if nothing else is left in it.
            if let Some(dir) = p.parent() {
                let _ = std::fs::remove_dir(dir);
            }
        })
        .await
        .ok();
    }
    app.db.delete_movie(id)?;
    app.db.log(
        "deleted",
        &format!("Removed {} from the library", engine::label(&m)),
    );
    Ok(StatusCode::NO_CONTENT)
}

async fn search_movie(
    State(app): State<App>,
    _o: Owner,
    Path(id): Path<i64>,
) -> ApiResult<Option<Release>> {
    app.db
        .movie(id)?
        .ok_or_else(|| AppError::not_found("movie"))?;
    Ok(Json(engine::auto_search(&app, id).await?))
}

async fn movie_releases(
    State(app): State<App>,
    _o: Owner,
    Path(id): Path<i64>,
) -> ApiResult<Vec<Release>> {
    let m = app
        .db
        .movie(id)?
        .ok_or_else(|| AppError::not_found("movie"))?;
    Ok(Json(engine::releases(&app, &m).await?))
}

async fn grab(
    State(app): State<App>,
    _o: Owner,
    Json(g): Json<Grab>,
) -> Result<StatusCode, AppError> {
    let m = app
        .db
        .movie(g.movie_id)?
        .ok_or_else(|| AppError::not_found("movie"))?;
    engine::grab(&app, &m, &g.release).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn unblock(
    State(app): State<App>,
    _o: Owner,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    app.db.unblock(id)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct DownloadsQuery {
    #[serde(default)]
    all: bool,
}

async fn list_downloads(
    State(app): State<App>,
    _u: CurrentUser,
    Query(q): Query<DownloadsQuery>,
) -> ApiResult<Vec<Download>> {
    Ok(Json(app.db.downloads(!q.all)?))
}

#[derive(Deserialize)]
struct RemoveBody {
    #[serde(default)]
    blocklist: bool,
}

async fn remove_download(
    State(app): State<App>,
    _o: Owner,
    Path(id): Path<i64>,
    Json(b): Json<RemoveBody>,
) -> Result<StatusCode, AppError> {
    engine::remove_download(&app, id, b.blocklist).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn history(State(app): State<App>, _u: CurrentUser) -> ApiResult<Vec<HistoryEvent>> {
    Ok(Json(app.db.history(300)?))
}

async fn scan_library(State(app): State<App>, _o: Owner) -> ApiResult<ScanReport> {
    Ok(Json(engine::scan_library(&app).await?))
}

/// TMDB posters through the server, cached on disk, so the browser never
/// talks to TMDB and works offline for posters already seen.
async fn poster(
    State(app): State<App>,
    _u: CurrentUser,
    Path((size, file)): Path<(String, String)>,
) -> Result<Response, AppError> {
    if !tmdb::valid_image_path(&size, &file) {
        return Err(AppError::not_found("poster"));
    }
    let dir = app.cfg.data_dir.join("cache/posters").join(&size);
    let path = dir.join(&file);
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(_) => {
            let url = format!(
                "{}/{size}/{file}",
                app.cfg.tmdb_images.trim_end_matches('/')
            );
            let b = app.net.get_bytes(&url, |r| r).await?.to_vec();
            tokio::fs::create_dir_all(&dir).await.ok();
            let tmp = dir.join(format!(".{file}.tmp"));
            if tokio::fs::write(&tmp, &b).await.is_ok() {
                let _ = tokio::fs::rename(&tmp, &path).await;
            }
            b
        }
    };
    let ct = if file.ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    };
    Ok((
        [
            (header::CONTENT_TYPE, ct),
            (header::CACHE_CONTROL, "private, max-age=604800, immutable"),
        ],
        bytes,
    )
        .into_response())
}
