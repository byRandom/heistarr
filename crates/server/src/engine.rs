//! What Heistarr does: add movies, search indexers, grab the best release,
//! follow the download, import it into the library and tell the media
//! server.

use crate::client::{self, Client, ItemState};
use crate::db::{ActiveDownload, now};
use crate::importer;
use crate::state::{App, AppError};
use crate::tmdb::Tmdb;
use crate::torznab;
use anyhow::{Context, Result, bail};
use heistarr_core::api::{CatalogMovie, Movie, MovieStatus, Release, ScanReport, Settings};
use heistarr_core::release::{normalize_title, parse};
use heistarr_core::rules::{Quality, Target, is_upgrade, judge};
use std::path::Path;

/// A download stalled this long is given up on and the next release tried.
pub const STALL_LIMIT: i64 = 6 * 3600;
/// A download the client does not know after this long has failed to add.
pub const MISSING_LIMIT: i64 = 15 * 60;

pub fn tmdb<'a>(app: &'a App, s: &'a Settings) -> Tmdb<'a> {
    Tmdb {
        net: &app.net,
        base: &app.cfg.tmdb_api,
        key: &s.tmdb_key,
        language: &s.metadata_language,
    }
}

pub async fn catalog_search(app: &App, q: &str, year: Option<i32>) -> Result<Vec<CatalogMovie>> {
    let s = app.db.settings()?;
    let mut hits = tmdb(app, &s).search(q, year).await?;
    let lib = app.db.library_ids_by_tmdb()?;
    for h in &mut hits {
        h.library_id = lib.get(&h.tmdb_id).copied();
    }
    Ok(hits)
}

pub async fn add_movie(app: &App, tmdb_id: i64, search_now: bool) -> Result<Movie, AppError> {
    if let Some(id) = app.db.movie_id_by_tmdb(tmdb_id)? {
        return Err(AppError::new(
            axum::http::StatusCode::CONFLICT,
            format!("already in the library (#{id})"),
        ));
    }
    let s = app.db.settings()?;
    let d = tmdb(app, &s).movie(tmdb_id).await?;
    let mut m = Movie {
        id: 0,
        tmdb_id: d.id,
        imdb_id: d.imdb_id.clone().filter(|i| !i.is_empty()),
        title: d.title.clone(),
        original_title: d.original_title.clone().filter(|o| *o != d.title),
        year: d.year(),
        overview: d.overview.clone().unwrap_or_default(),
        poster_path: d.poster_path.clone(),
        status: MovieStatus::Wanted,
        file_path: None,
        file_quality: None,
        added_at: now(),
    };
    m.id = app.db.insert_movie(&m)?;
    app.db.log("added", &format!("Added {}", label(&m)));
    if search_now {
        let app = app.clone();
        let id = m.id;
        tokio::spawn(async move {
            if let Err(e) = auto_search(&app, id).await {
                app.db.log("search", &format!("Search failed: {e:#}"));
            }
        });
    }
    Ok(m)
}

pub fn label(m: &Movie) -> String {
    match m.year {
        Some(y) => format!("{} ({y})", m.title),
        None => m.title.clone(),
    }
}

/// Searches every enabled indexer and scores every release; accepted ones
/// first, best first.
pub async fn releases(app: &App, m: &Movie) -> Result<Vec<Release>> {
    let s = app.db.settings()?;
    let indexers: Vec<_> = app
        .db
        .indexers()?
        .into_iter()
        .filter(|i| i.enabled)
        .collect();
    if indexers.is_empty() {
        bail!("no indexer enabled: add one in Settings");
    }
    let searches = indexers.iter().map(|ix| {
        let net = &app.net;
        async move {
            (
                ix,
                torznab::search_movie(net, ix, m.imdb_id.as_deref(), &m.title, m.year).await,
            )
        }
    });
    let results = futures_util::future::join_all(searches).await;

    let blocked: Vec<String> = app
        .db
        .blocklist(m.id)?
        .into_iter()
        .map(|b| b.release_title)
        .collect();
    let target = Target {
        title: &m.title,
        year: m.year,
    };
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (ix, r) in results {
        let items = match r {
            Ok(items) => items,
            Err(e) => {
                tracing::warn!("indexer {}: {e:#}", ix.name);
                errors.push(format!("{}: {e:#}", ix.name));
                continue;
            }
        };
        for it in items {
            let p = parse(&it.title);
            let mut v = judge(&s.rules, &target, &p, it.size, it.seeders);
            // Releases often carry the original title instead of the
            // translated one.
            if let Some(orig) = &m.original_title
                && v.rejections.iter().any(|r| r.starts_with("title"))
            {
                let alt = judge(
                    &s.rules,
                    &Target {
                        title: orig,
                        year: m.year,
                    },
                    &p,
                    it.size,
                    it.seeders,
                );
                if alt.rejections.len() < v.rejections.len() {
                    v = alt;
                }
            }
            if blocked.contains(&it.title) {
                v.score = None;
                v.rejections.push("blocklisted".into());
            }
            if let (MovieStatus::Available, Some(cur)) = (m.status, m.file_quality)
                && v.score.is_some()
                && !is_upgrade(&s.rules, cur, Quality::of(&p))
            {
                v.score = None;
                v.rejections
                    .push(format!("not an upgrade over {}", cur.label()));
            }
            out.push(Release {
                title: it.title,
                indexer: ix.name.clone(),
                download_url: it.download_url,
                size: it.size,
                seeders: it.seeders,
                score: v.score,
                rejections: v.rejections,
                quality: p.quality_label(),
            });
        }
    }
    if out.is_empty() && !errors.is_empty() {
        bail!("every indexer failed: {}", errors.join("; "));
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then(b.seeders.cmp(&a.seeders)));
    out.dedup_by(|a, b| a.title == b.title && a.size == b.size);
    Ok(out)
}

/// Searches and grabs the best accepted release, if any.
pub async fn auto_search(app: &App, movie_id: i64) -> Result<Option<Release>> {
    let m = app.db.movie(movie_id)?.context("movie not found")?;
    app.db.mark_searched(m.id)?;
    let list = releases(app, &m).await?;
    let Some(best) = list.into_iter().find(|r| r.score.is_some()) else {
        if m.status == MovieStatus::Wanted {
            app.db.log(
                "search",
                &format!("No acceptable release for {}", label(&m)),
            );
        }
        return Ok(None);
    };
    grab(app, &m, &best).await?;
    Ok(Some(best))
}

/// Sends a release to the download client.
pub async fn grab(app: &App, m: &Movie, r: &Release) -> Result<i64, AppError> {
    let _g = app.grab_lock.lock().await;
    if app.db.has_active_download(m.id)? {
        return Err(AppError::new(
            axum::http::StatusCode::CONFLICT,
            format!(
                "{} already has a download in progress: remove it first",
                label(m)
            ),
        ));
    }
    let s = app.db.settings()?;
    let (c, _) = Client::new(app, &s);
    let id = app.db.insert_download(m.id, &r.title, &r.download_url)?;
    match c.add(&r.download_url, id).await {
        Ok(client_id) => {
            app.db
                .update_download(id, client_id.as_deref(), 0.0, "sent", true)?;
            if m.status == MovieStatus::Wanted {
                app.db.set_movie_status(m.id, MovieStatus::Downloading)?;
            }
            app.db.log(
                "grabbed",
                &format!("Sent {} to {} for {}", r.title, c.name(), label(m)),
            );
            app.wake.notify_one();
            Ok(id)
        }
        Err(e) => {
            app.db
                .update_download(id, None, 0.0, "failed to send", false)?;
            app.db.log(
                "error",
                &format!("Could not send {} to {}: {e:#}", r.title, c.name()),
            );
            Err(e.into())
        }
    }
}

/// Follows active downloads: progress, completion (import), failures.
pub async fn check_downloads(app: &App) -> Result<()> {
    let active = app.db.active_downloads()?;
    if active.is_empty() {
        return Ok(());
    }
    let s = app.db.settings()?;
    let (c, mapping) = Client::new(app, &s);
    let items = c.list(&mapping).await?;
    for d in active {
        let Some(it) = client::find(
            &items,
            d.client_hash.as_deref(),
            d.id,
            &d.download_url,
            &d.release_title,
        ) else {
            if now() - d.added_at > MISSING_LIMIT {
                fail(app, &c, &d, None, &format!("not found in {}", c.name())).await?;
            }
            continue;
        };
        match it.state {
            ItemState::Completed => {
                app.db
                    .update_download(d.id, Some(&it.id), 1.0, "importing", true)?;
                match import(app, &s, &d, &it.content_path).await {
                    Ok(()) => {
                        app.db
                            .update_download(d.id, Some(&it.id), 1.0, "imported", false)?;
                        if s.remove_after_import
                            && let Err(e) = c.remove(&it.id, true).await
                        {
                            tracing::warn!("could not remove {} from {}: {e:#}", it.name, c.name());
                        }
                    }
                    Err(e) => {
                        // Keep it active: a fix (path mapping, permissions)
                        // lets the next check import it.
                        app.db
                            .update_download(d.id, Some(&it.id), 1.0, "import failed", true)?;
                        if d.state != "import failed" {
                            app.db.log(
                                "error",
                                &format!("Import of {} failed: {e:#}", d.release_title),
                            );
                        }
                    }
                }
            }
            ItemState::Failed => {
                let why = it.error.clone().unwrap_or_else(|| it.raw_state.clone());
                fail(
                    app,
                    &c,
                    &d,
                    Some(&it.id),
                    &format!("{} reported {why}", c.name()),
                )
                .await?;
            }
            ItemState::Stalled => {
                let since = d.stalled_since.unwrap_or_else(now);
                if d.stalled_since.is_none() {
                    app.db.set_stalled_since(d.id, Some(since))?;
                }
                app.db
                    .update_download(d.id, Some(&it.id), it.progress, &it.raw_state, true)?;
                if now() - since > STALL_LIMIT {
                    fail(app, &c, &d, Some(&it.id), "stalled for over 6 hours").await?;
                }
            }
            ItemState::Downloading => {
                if d.stalled_since.is_some() {
                    app.db.set_stalled_since(d.id, None)?;
                }
                app.db
                    .update_download(d.id, Some(&it.id), it.progress, &it.raw_state, true)?;
            }
        }
    }
    Ok(())
}

/// Gives up on a download: blocklists the release, removes it from the
/// client and lets the next search pick another one.
pub async fn fail(
    app: &App,
    c: &Client<'_>,
    d: &ActiveDownload,
    client_id: Option<&str>,
    reason: &str,
) -> Result<()> {
    app.db.block_release(d.movie_id, &d.release_title, reason)?;
    app.db
        .update_download(d.id, client_id, 0.0, "failed", false)?;
    if let Some(id) = client_id.or(d.client_hash.as_deref())
        && let Err(e) = c.remove(id, true).await
    {
        tracing::warn!("could not remove failed download: {e:#}");
    }
    if let Some(m) = app.db.movie(d.movie_id)? {
        if m.status == MovieStatus::Downloading {
            app.db.set_movie_status(m.id, MovieStatus::Wanted)?;
        }
        app.db.reset_search(m.id)?;
        app.db.log(
            "failed",
            &format!(
                "{} failed ({reason}); blocklisted, will try another release",
                d.release_title
            ),
        );
    }
    app.wake.notify_one();
    Ok(())
}

/// Removes an active download by hand, optionally blocklisting it.
pub async fn remove_download(app: &App, id: i64, blocklist: bool) -> Result<(), AppError> {
    let d = app
        .db
        .active_download(id)?
        .ok_or_else(|| AppError::not_found("active download"))?;
    let s = app.db.settings()?;
    let (c, _) = Client::new(app, &s);
    if let Some(cid) = &d.client_hash
        && let Err(e) = c.remove(cid, true).await
    {
        tracing::warn!("could not remove from {}: {e:#}", c.name());
    }
    if blocklist {
        app.db
            .block_release(d.movie_id, &d.release_title, "removed by hand")?;
    }
    app.db.update_download(d.id, None, 0.0, "removed", false)?;
    if let Some(m) = app.db.movie(d.movie_id)?
        && m.status == MovieStatus::Downloading
    {
        app.db.set_movie_status(m.id, MovieStatus::Wanted)?;
    }
    app.db
        .log("removed", &format!("Removed download {}", d.release_title));
    Ok(())
}

async fn import(app: &App, s: &Settings, d: &ActiveDownload, content: &Path) -> Result<()> {
    let m = app.db.movie(d.movie_id)?.context("movie was deleted")?;
    if s.movies_root.trim().is_empty() {
        bail!("no movies folder set in Settings");
    }
    let root = Path::new(s.movies_root.trim()).to_path_buf();
    let content = content.to_path_buf();
    let (title, year, old) = (m.title.clone(), m.year, m.file_path.clone());
    let (dest, how) = tokio::task::spawn_blocking(move || -> Result<_> {
        if !content.exists() {
            bail!(
                "{} does not exist here: check the path mapping in Settings",
                content.display()
            );
        }
        let video = importer::find_video(&content)?;
        let ext = video
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mkv")
            .to_string();
        let dest = importer::movie_dest(&root, &title, year, &ext);
        let how = importer::place(&video, &dest)?;
        importer::remove_replaced(old.as_deref(), &dest);
        Ok((dest, how))
    })
    .await??;
    let quality = Quality::of(&parse(&d.release_title));
    app.db
        .set_movie_file(m.id, &dest.to_string_lossy(), Some(quality))?;
    let how = match how {
        importer::Placed::Hardlinked => "hardlinked",
        importer::Placed::Copied => "copied",
    };
    app.db.log(
        "imported",
        &format!("Imported {} ({how}) to {}", label(&m), dest.display()),
    );
    if let Err(e) = crate::mediaserver::refresh(&app.net, s).await {
        app.db
            .log("error", &format!("Media server refresh failed: {e:#}"));
    }
    Ok(())
}

/// Reads the movies folder and adds what is there as available, matching
/// each folder against TMDB.
pub async fn scan_library(app: &App) -> Result<ScanReport> {
    let s = app.db.settings()?;
    let root = s.movies_root.trim().to_string();
    if root.is_empty() {
        bail!("no movies folder set in Settings");
    }
    let found = tokio::task::spawn_blocking(move || importer::scan(Path::new(&root))).await??;
    let known: std::collections::HashSet<String> = app
        .db
        .movies()?
        .into_iter()
        .filter_map(|m| m.file_path)
        .collect();
    let t = tmdb(app, &s);
    let mut report = ScanReport {
        matched: vec![],
        unmatched: vec![],
        already: 0,
    };
    for f in found {
        if known.contains(&*f.file.to_string_lossy()) {
            report.already += 1;
            continue;
        }
        let hits = t.search(&f.title, f.year).await.unwrap_or_default();
        let want = normalize_title(&f.title);
        let hit = hits
            .iter()
            .find(|h| normalize_title(&h.title) == want && (f.year.is_none() || h.year == f.year))
            .or_else(|| {
                hits.first()
                    .filter(|h| f.year.is_some() && h.year == f.year)
            });
        let Some(hit) = hit else {
            report.unmatched.push(f.label);
            continue;
        };
        if app.db.movie_id_by_tmdb(hit.tmdb_id)?.is_some() {
            report.already += 1;
            continue;
        }
        let d = t.movie(hit.tmdb_id).await?;
        let name = f
            .file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let p = parse(&name);
        let quality =
            (p.resolution != heistarr_core::release::Resolution::Unknown).then(|| Quality::of(&p));
        let m = Movie {
            id: 0,
            tmdb_id: d.id,
            imdb_id: d.imdb_id.clone().filter(|i| !i.is_empty()),
            title: d.title.clone(),
            original_title: d.original_title.clone().filter(|o| *o != d.title),
            year: d.year(),
            overview: d.overview.clone().unwrap_or_default(),
            poster_path: d.poster_path.clone(),
            status: MovieStatus::Available,
            file_path: Some(f.file.to_string_lossy().to_string()),
            file_quality: quality,
            added_at: now(),
        };
        app.db.insert_movie(&m)?;
        report.matched.push(format!("{} → {}", f.label, label(&m)));
    }
    app.db.log(
        "scan",
        &format!(
            "Library scan: {} added, {} not recognised, {} already known",
            report.matched.len(),
            report.unmatched.len(),
            report.already
        ),
    );
    Ok(report)
}
