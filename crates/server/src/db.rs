//! SQLite storage. One connection behind a mutex: every query is short, and
//! no lock is held across an `.await`.

use crate::crypto::Cipher;
use anyhow::{Context, Result};
use heistarr_core::api::{
    BlockedRelease, Download, HistoryEvent, Indexer, Movie, MovieStatus, Settings,
};
use heistarr_core::rules::Quality;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

/// Schema migrations, applied in order. `PRAGMA user_version` holds how many
/// have run. Never edit a shipped migration: add a new one.
const MIGRATIONS: &[&str] = &["
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    is_owner INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL
);
CREATE TABLE kv (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE indexers (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    api_key TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE movies (
    id INTEGER PRIMARY KEY,
    tmdb_id INTEGER NOT NULL UNIQUE,
    imdb_id TEXT,
    title TEXT NOT NULL,
    original_title TEXT,
    year INTEGER,
    overview TEXT NOT NULL DEFAULT '',
    poster_path TEXT,
    status TEXT NOT NULL DEFAULT 'wanted',
    file_path TEXT,
    file_quality TEXT,
    added_at INTEGER NOT NULL,
    last_search_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE downloads (
    id INTEGER PRIMARY KEY,
    movie_id INTEGER NOT NULL REFERENCES movies(id) ON DELETE CASCADE,
    release_title TEXT NOT NULL,
    download_url TEXT NOT NULL,
    client_hash TEXT,
    progress REAL NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'queued',
    active INTEGER NOT NULL DEFAULT 1,
    added_at INTEGER NOT NULL,
    stalled_since INTEGER
);
CREATE INDEX downloads_active ON downloads(active);
CREATE TABLE blocklist (
    id INTEGER PRIMARY KEY,
    movie_id INTEGER NOT NULL REFERENCES movies(id) ON DELETE CASCADE,
    release_title TEXT NOT NULL,
    reason TEXT NOT NULL,
    at INTEGER NOT NULL,
    UNIQUE (movie_id, release_title)
);
CREATE TABLE history (
    id INTEGER PRIMARY KEY,
    at INTEGER NOT NULL,
    kind TEXT NOT NULL,
    message TEXT NOT NULL
);
"];

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
    cipher: Cipher,
}

#[derive(Clone, Debug)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub is_owner: bool,
}

fn user_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: r.get(0)?,
        username: r.get(1)?,
        password_hash: r.get(2)?,
        is_owner: r.get(3)?,
    })
}

/// An active download as the download checker needs it.
#[derive(Clone, Debug)]
pub struct ActiveDownload {
    pub id: i64,
    pub movie_id: i64,
    pub release_title: String,
    pub download_url: String,
    pub client_hash: Option<String>,
    pub state: String,
    pub added_at: i64,
    pub stalled_since: Option<i64>,
}

fn movie_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Movie> {
    let status: String = r.get(7)?;
    let quality: Option<String> = r.get(9)?;
    Ok(Movie {
        id: r.get(0)?,
        tmdb_id: r.get(1)?,
        imdb_id: r.get(2)?,
        title: r.get(3)?,
        year: r.get(4)?,
        overview: r.get(5)?,
        poster_path: r.get(6)?,
        status: MovieStatus::parse(&status).unwrap_or(MovieStatus::Wanted),
        file_path: r.get(8)?,
        file_quality: quality.and_then(|q| serde_json::from_str::<Quality>(&q).ok()),
        added_at: r.get(10)?,
        original_title: r.get(11)?,
    })
}

const MOVIE_COLS: &str = "id, tmdb_id, imdb_id, title, year, overview, poster_path, status, file_path, file_quality, added_at, original_title";

const DOWNLOAD_COLS: &str =
    "d.id, d.movie_id, m.title, d.release_title, d.client_hash, d.progress, d.state, d.added_at";

fn download_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Download> {
    Ok(Download {
        id: r.get(0)?,
        movie_id: r.get(1)?,
        movie_title: r.get(2)?,
        release_title: r.get(3)?,
        client_hash: r.get(4)?,
        progress: r.get::<_, f64>(5)? as f32,
        state: r.get(6)?,
        added_at: r.get(7)?,
    })
}

fn collect<T>(rows: impl Iterator<Item = rusqlite::Result<T>>) -> Result<Vec<T>> {
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

impl Db {
    pub fn open(path: &Path, cipher: Cipher) -> Result<Self> {
        let c = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(c, cipher)
    }

    pub fn memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, Cipher::random())
    }

    fn init(mut c: Connection, cipher: Cipher) -> Result<Self> {
        c.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
        )?;
        migrate(&mut c)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(c)),
            cipher,
        })
    }

    fn c(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---- accounts ----

    pub fn user_count(&self) -> Result<i64> {
        Ok(self
            .c()
            .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?)
    }

    /// Creates the owner only if there is no account yet, atomically.
    pub fn create_first_owner(&self, username: &str, hash: &str) -> Result<Option<i64>> {
        let mut c = self.c();
        let tx = c.transaction()?;
        let n: i64 = tx.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
        if n > 0 {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO users (username, password_hash, is_owner) VALUES (?1, ?2, 1)",
            params![username, hash],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(Some(id))
    }

    pub fn user_by_name(&self, username: &str) -> Result<Option<User>> {
        Ok(self
            .c()
            .query_row(
                "SELECT id, username, password_hash, is_owner FROM users WHERE username = ?1",
                [username],
                user_row,
            )
            .optional()?)
    }

    pub fn create_session(&self, token_hash: &str, user_id: i64) -> Result<()> {
        self.c().execute(
            "INSERT INTO sessions (token_hash, user_id, created_at) VALUES (?1, ?2, ?3)",
            params![token_hash, user_id, now()],
        )?;
        Ok(())
    }

    pub fn delete_session(&self, token_hash: &str) -> Result<()> {
        self.c()
            .execute("DELETE FROM sessions WHERE token_hash = ?1", [token_hash])?;
        Ok(())
    }

    pub fn session_user(&self, token_hash: &str, max_age: i64) -> Result<Option<User>> {
        Ok(self
            .c()
            .query_row(
                "SELECT u.id, u.username, u.password_hash, u.is_owner FROM sessions s
                 JOIN users u ON u.id = s.user_id WHERE s.token_hash = ?1 AND s.created_at > ?2",
                params![token_hash, now() - max_age],
                user_row,
            )
            .optional()?)
    }

    pub fn purge_sessions(&self, max_age: i64) -> Result<usize> {
        Ok(self.c().execute(
            "DELETE FROM sessions WHERE created_at <= ?1",
            [now() - max_age],
        )?)
    }

    // ---- settings (secrets encrypted at rest) ----

    pub fn settings(&self) -> Result<Settings> {
        let v: Option<String> = self
            .c()
            .query_row("SELECT value FROM kv WHERE key = 'settings'", [], |r| {
                r.get(0)
            })
            .optional()?;
        let mut s = match v {
            Some(v) => serde_json::from_str(&v)?,
            None => Settings {
                qbit_category: "heistarr".into(),
                download_client: "qbittorrent".into(),
                theme: "miami".into(),
                ..Settings::default()
            },
        };
        for f in secret_fields(&mut s) {
            *f = self.cipher.decrypt(f)?;
        }
        Ok(s)
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        let mut s = s.clone();
        for f in secret_fields(&mut s) {
            *f = self.cipher.encrypt(f);
        }
        self.c().execute(
            "INSERT INTO kv (key, value) VALUES ('settings', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [serde_json::to_string(&s)?],
        )?;
        Ok(())
    }

    // ---- indexers ----

    pub fn indexers(&self) -> Result<Vec<Indexer>> {
        let c = self.c();
        let mut st =
            c.prepare("SELECT id, name, url, api_key, enabled FROM indexers ORDER BY id")?;
        let rows: Vec<Indexer> = collect(st.query_map([], |r| {
            Ok(Indexer {
                id: r.get(0)?,
                name: r.get(1)?,
                url: r.get(2)?,
                api_key: r.get(3)?,
                enabled: r.get(4)?,
            })
        })?)?;
        rows.into_iter()
            .map(|mut i| {
                i.api_key = self.cipher.decrypt(&i.api_key)?;
                Ok(i)
            })
            .collect()
    }

    pub fn indexer(&self, id: i64) -> Result<Option<Indexer>> {
        Ok(self.indexers()?.into_iter().find(|i| i.id == id))
    }

    pub fn save_indexer(&self, i: &Indexer) -> Result<i64> {
        let key = self.cipher.encrypt(&i.api_key);
        let c = self.c();
        if i.id == 0 {
            c.execute(
                "INSERT INTO indexers (name, url, api_key, enabled) VALUES (?1, ?2, ?3, ?4)",
                params![i.name, i.url, key, i.enabled],
            )?;
            Ok(c.last_insert_rowid())
        } else {
            c.execute(
                "UPDATE indexers SET name = ?2, url = ?3, api_key = ?4, enabled = ?5 WHERE id = ?1",
                params![i.id, i.name, i.url, key, i.enabled],
            )?;
            Ok(i.id)
        }
    }

    pub fn delete_indexer(&self, id: i64) -> Result<()> {
        self.c()
            .execute("DELETE FROM indexers WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---- movies ----

    pub fn movies(&self) -> Result<Vec<Movie>> {
        let c = self.c();
        let mut st = c.prepare(&format!(
            "SELECT {MOVIE_COLS} FROM movies ORDER BY title COLLATE NOCASE"
        ))?;
        collect(st.query_map([], movie_row)?)
    }

    pub fn movie(&self, id: i64) -> Result<Option<Movie>> {
        Ok(self
            .c()
            .query_row(
                &format!("SELECT {MOVIE_COLS} FROM movies WHERE id = ?1"),
                [id],
                movie_row,
            )
            .optional()?)
    }

    pub fn movie_id_by_tmdb(&self, tmdb_id: i64) -> Result<Option<i64>> {
        Ok(self
            .c()
            .query_row("SELECT id FROM movies WHERE tmdb_id = ?1", [tmdb_id], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn library_ids_by_tmdb(&self) -> Result<std::collections::HashMap<i64, i64>> {
        let c = self.c();
        let mut st = c.prepare("SELECT tmdb_id, id FROM movies")?;
        Ok(collect(st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?)?
            .into_iter()
            .collect())
    }

    pub fn insert_movie(&self, m: &Movie) -> Result<i64> {
        let c = self.c();
        c.execute(
            "INSERT INTO movies (tmdb_id, imdb_id, title, year, overview, poster_path, status, file_path, file_quality, added_at, original_title)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                m.tmdb_id,
                m.imdb_id,
                m.title,
                m.year,
                m.overview,
                m.poster_path,
                m.status.as_str(),
                m.file_path,
                m.file_quality.map(|q| serde_json::to_string(&q).unwrap_or_default()),
                m.added_at,
                m.original_title
            ],
        )?;
        Ok(c.last_insert_rowid())
    }

    pub fn delete_movie(&self, id: i64) -> Result<()> {
        self.c().execute("DELETE FROM movies WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_movie_status(&self, id: i64, s: MovieStatus) -> Result<()> {
        self.c().execute(
            "UPDATE movies SET status = ?2 WHERE id = ?1",
            params![id, s.as_str()],
        )?;
        Ok(())
    }

    pub fn set_movie_file(&self, id: i64, file: &str, quality: Option<Quality>) -> Result<()> {
        self.c().execute(
            "UPDATE movies SET status = 'available', file_path = ?2, file_quality = ?3 WHERE id = ?1",
            params![id, file, quality.map(|q| serde_json::to_string(&q).unwrap_or_default())],
        )?;
        Ok(())
    }

    /// Movies to search automatically: wanted ones, and available ones whose
    /// file may still be upgraded. Not searched since `before`, oldest first.
    pub fn movies_due_for_search(&self, before: i64, limit: i64) -> Result<Vec<Movie>> {
        let c = self.c();
        let mut st = c.prepare(&format!(
            "SELECT {MOVIE_COLS} FROM movies m
             WHERE status IN ('wanted', 'available') AND last_search_at < ?1
               AND NOT EXISTS (SELECT 1 FROM downloads d WHERE d.movie_id = m.id AND d.active = 1)
             ORDER BY status = 'available', last_search_at LIMIT ?2"
        ))?;
        collect(st.query_map(params![before, limit], movie_row)?)
    }

    pub fn mark_searched(&self, id: i64) -> Result<()> {
        self.c().execute(
            "UPDATE movies SET last_search_at = ?2 WHERE id = ?1",
            params![id, now()],
        )?;
        Ok(())
    }

    // ---- downloads ----

    pub fn insert_download(&self, movie_id: i64, title: &str, url: &str) -> Result<i64> {
        let c = self.c();
        c.execute(
            "INSERT INTO downloads (movie_id, release_title, download_url, added_at) VALUES (?1, ?2, ?3, ?4)",
            params![movie_id, title, url, now()],
        )?;
        Ok(c.last_insert_rowid())
    }

    pub fn downloads(&self, only_active: bool) -> Result<Vec<Download>> {
        let c = self.c();
        let mut st = c.prepare(&format!(
            "SELECT {DOWNLOAD_COLS} FROM downloads d JOIN movies m ON m.id = d.movie_id
             WHERE d.active >= ?1 ORDER BY d.added_at DESC, d.id DESC LIMIT 200"
        ))?;
        collect(st.query_map([only_active as i64], download_row)?)
    }

    pub fn movie_downloads(&self, movie_id: i64) -> Result<Vec<Download>> {
        let c = self.c();
        let mut st = c.prepare(&format!(
            "SELECT {DOWNLOAD_COLS} FROM downloads d JOIN movies m ON m.id = d.movie_id
             WHERE d.movie_id = ?1 ORDER BY d.added_at DESC, d.id DESC LIMIT 50"
        ))?;
        collect(st.query_map([movie_id], download_row)?)
    }

    pub fn active_downloads(&self) -> Result<Vec<ActiveDownload>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT id, movie_id, release_title, download_url, client_hash, state, added_at, stalled_since
             FROM downloads WHERE active = 1 ORDER BY id",
        )?;
        collect(st.query_map([], |r| {
            Ok(ActiveDownload {
                id: r.get(0)?,
                movie_id: r.get(1)?,
                release_title: r.get(2)?,
                download_url: r.get(3)?,
                client_hash: r.get(4)?,
                state: r.get(5)?,
                added_at: r.get(6)?,
                stalled_since: r.get(7)?,
            })
        })?)
    }

    pub fn update_download(
        &self,
        id: i64,
        hash: Option<&str>,
        progress: f32,
        state: &str,
        active: bool,
    ) -> Result<()> {
        self.c().execute(
            "UPDATE downloads SET client_hash = COALESCE(?2, client_hash), progress = ?3, state = ?4, active = ?5
             WHERE id = ?1",
            params![id, hash, progress as f64, state, active],
        )?;
        Ok(())
    }

    pub fn active_download(&self, id: i64) -> Result<Option<ActiveDownload>> {
        Ok(self.active_downloads()?.into_iter().find(|d| d.id == id))
    }

    pub fn has_active_download(&self, movie_id: i64) -> Result<bool> {
        Ok(self.c().query_row(
            "SELECT EXISTS (SELECT 1 FROM downloads WHERE movie_id = ?1 AND active = 1)",
            [movie_id],
            |r| r.get(0),
        )?)
    }

    pub fn reset_search(&self, movie_id: i64) -> Result<()> {
        self.c().execute(
            "UPDATE movies SET last_search_at = 0 WHERE id = ?1",
            [movie_id],
        )?;
        Ok(())
    }

    pub fn set_stalled_since(&self, id: i64, since: Option<i64>) -> Result<()> {
        self.c().execute(
            "UPDATE downloads SET stalled_since = ?2 WHERE id = ?1",
            params![id, since],
        )?;
        Ok(())
    }

    // ---- blocklist ----

    pub fn block_release(&self, movie_id: i64, title: &str, reason: &str) -> Result<()> {
        self.c().execute(
            "INSERT INTO blocklist (movie_id, release_title, reason, at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(movie_id, release_title) DO UPDATE SET reason = excluded.reason, at = excluded.at",
            params![movie_id, title, reason, now()],
        )?;
        Ok(())
    }

    pub fn blocklist(&self, movie_id: i64) -> Result<Vec<BlockedRelease>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT id, movie_id, release_title, reason, at FROM blocklist WHERE movie_id = ?1 ORDER BY at DESC",
        )?;
        collect(st.query_map([movie_id], |r| {
            Ok(BlockedRelease {
                id: r.get(0)?,
                movie_id: r.get(1)?,
                release_title: r.get(2)?,
                reason: r.get(3)?,
                at: r.get(4)?,
            })
        })?)
    }

    pub fn unblock(&self, id: i64) -> Result<()> {
        self.c()
            .execute("DELETE FROM blocklist WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---- history ----

    pub fn log(&self, kind: &str, message: &str) {
        tracing::info!(kind, "{message}");
        let _ = self.c().execute(
            "INSERT INTO history (at, kind, message) VALUES (?1, ?2, ?3)",
            params![now(), kind, message],
        );
    }

    pub fn history(&self, limit: i64) -> Result<Vec<HistoryEvent>> {
        let c = self.c();
        let mut st =
            c.prepare("SELECT id, at, kind, message FROM history ORDER BY id DESC LIMIT ?1")?;
        collect(st.query_map([limit], |r| {
            Ok(HistoryEvent {
                id: r.get(0)?,
                at: r.get(1)?,
                kind: r.get(2)?,
                message: r.get(3)?,
            })
        })?)
    }

    pub fn prune_history(&self, older_than: i64) -> Result<usize> {
        Ok(self
            .c()
            .execute("DELETE FROM history WHERE at < ?1", [now() - older_than])?)
    }
}

/// The settings fields stored encrypted and masked in the UI.
pub fn secret_fields(s: &mut Settings) -> [&mut String; 4] {
    [
        &mut s.tmdb_key,
        &mut s.qbit_password,
        &mut s.synology_password,
        &mut s.media_server_token,
    ]
}

fn migrate(c: &mut Connection) -> Result<()> {
    let done: usize = c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
    if done > MIGRATIONS.len() {
        anyhow::bail!("the database is from a newer Heistarr (schema {done}); update Heistarr");
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(done) {
        let tx = c.transaction()?;
        tx.execute_batch(sql)
            .with_context(|| format!("migration {}", i + 1))?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
        tracing::info!("database migrated to schema {}", i + 1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie() -> Movie {
        Movie {
            id: 0,
            tmdb_id: 603,
            imdb_id: Some("tt0133093".into()),
            title: "The Matrix".into(),
            original_title: None,
            year: Some(1999),
            overview: String::new(),
            poster_path: None,
            status: MovieStatus::Wanted,
            file_path: None,
            file_quality: None,
            added_at: 1,
        }
    }

    #[test]
    fn reopening_keeps_data_and_newer_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("h.db");
        let cipher = Cipher::random();
        {
            let db = Db::open(&path, cipher.clone()).unwrap();
            db.insert_movie(&movie()).unwrap();
        }
        // Opening again runs no migration twice and keeps the rows.
        let db = Db::open(&path, cipher.clone()).unwrap();
        assert_eq!(db.movies().unwrap().len(), 1);
        drop(db);
        // A database written by a newer Heistarr must not be touched.
        let c = Connection::open(&path).unwrap();
        c.pragma_update(None, "user_version", (MIGRATIONS.len() + 1) as i64)
            .unwrap();
        drop(c);
        let err = Db::open(&path, cipher).err().expect("refused");
        assert!(err.to_string().contains("newer Heistarr"), "{err}");
    }

    #[test]
    fn first_owner_only_once() {
        let db = Db::memory().unwrap();
        assert!(db.create_first_owner("a", "h").unwrap().is_some());
        assert!(db.create_first_owner("b", "h").unwrap().is_none());
        assert_eq!(db.user_count().unwrap(), 1);
    }

    #[test]
    fn movie_roundtrip_and_cascade() {
        let db = Db::memory().unwrap();
        let id = db.insert_movie(&movie()).unwrap();
        db.insert_download(id, "r", "u").unwrap();
        db.block_release(id, "bad", "failed").unwrap();
        assert_eq!(db.downloads(true).unwrap().len(), 1);
        assert_eq!(db.blocklist(id).unwrap().len(), 1);
        db.delete_movie(id).unwrap();
        assert!(db.downloads(false).unwrap().is_empty());
        assert!(db.blocklist(id).unwrap().is_empty());
    }

    #[test]
    fn secrets_are_encrypted_at_rest() {
        let db = Db::memory().unwrap();
        let mut s = db.settings().unwrap();
        s.tmdb_key = "tmdb-secret-123".into();
        db.save_settings(&s).unwrap();
        let raw: String = db
            .c()
            .query_row("SELECT value FROM kv", [], |r| r.get(0))
            .unwrap();
        assert!(!raw.contains("tmdb-secret-123"));
        assert_eq!(db.settings().unwrap().tmdb_key, "tmdb-secret-123");

        db.save_indexer(&Indexer {
            id: 0,
            name: "j".into(),
            url: "u".into(),
            api_key: "idx-key".into(),
            enabled: true,
        })
        .unwrap();
        let raw: String = db
            .c()
            .query_row("SELECT api_key FROM indexers", [], |r| r.get(0))
            .unwrap();
        assert!(!raw.contains("idx-key"));
        assert_eq!(db.indexers().unwrap()[0].api_key, "idx-key");
    }

    #[test]
    fn due_for_search_skips_active_downloads() {
        let db = Db::memory().unwrap();
        let id = db.insert_movie(&movie()).unwrap();
        assert_eq!(db.movies_due_for_search(now() + 1, 10).unwrap().len(), 1);
        db.insert_download(id, "r", "u").unwrap();
        assert!(db.movies_due_for_search(now() + 1, 10).unwrap().is_empty());
    }
}
