//! Background tasks: follow downloads, search for wanted movies and
//! upgrades, and clean up old history and sessions.

use crate::db::now;
use crate::engine;
use crate::state::App;
use std::time::Duration;

/// How often active downloads are checked.
pub const CHECK_EVERY: Duration = Duration::from_secs(30);
/// How often due movies are searched, and how long between two searches of
/// the same movie.
pub const SEARCH_EVERY: i64 = 15 * 60;
pub const RESEARCH_AFTER: i64 = 6 * 3600;
/// Movies searched per round, to be gentle with indexers.
pub const SEARCH_BATCH: i64 = 10;
pub const HISTORY_DAYS: i64 = 90;

pub fn spawn(app: App) {
    tokio::spawn(async move {
        let mut last_search = 0i64;
        let mut last_cleanup = 0i64;
        loop {
            if let Err(e) = engine::check_downloads(&app).await {
                tracing::warn!("checking downloads: {e:#}");
            }
            let t = now();
            if t - last_search >= SEARCH_EVERY {
                last_search = t;
                search_due(&app).await;
            }
            if t - last_cleanup >= 24 * 3600 {
                last_cleanup = t;
                cleanup(&app);
            }
            tokio::select! {
                _ = tokio::time::sleep(CHECK_EVERY) => {}
                _ = app.wake.notified() => {
                    // A grab or a failure: let the client register it, then
                    // look again; failures also search right away.
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    search_due(&app).await;
                }
            }
        }
    });
}

async fn search_due(app: &App) {
    let s = match app.db.settings() {
        Ok(s) => s,
        Err(e) => return tracing::warn!("settings: {e:#}"),
    };
    let has_indexer = app
        .db
        .indexers()
        .map(|v| v.iter().any(|i| i.enabled))
        .unwrap_or(false);
    if !has_indexer || s.tmdb_key.is_empty() {
        return;
    }
    let due = match app
        .db
        .movies_due_for_search(now() - RESEARCH_AFTER, SEARCH_BATCH)
    {
        Ok(d) => d,
        Err(e) => return tracing::warn!("due movies: {e:#}"),
    };
    for m in due {
        if m.status == heistarr_core::api::MovieStatus::Available && !s.rules.upgrades {
            let _ = app.db.mark_searched(m.id);
            continue;
        }
        if let Err(e) = engine::auto_search(app, m.id).await {
            app.db.log(
                "search",
                &format!("Search for {} failed: {e:#}", engine::label(&m)),
            );
        }
    }
}

fn cleanup(app: &App) {
    if let Ok(n) = app.db.prune_history(HISTORY_DAYS * 24 * 3600)
        && n > 0
    {
        tracing::info!("pruned {n} history events");
    }
    let _ = app.db.purge_sessions(crate::auth::SESSION_AGE);
}
