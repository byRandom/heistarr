//! SQLite storage. One connection behind a mutex: every query is short, and
//! no lock is held across an `.await`.

use anyhow::{Context, Result};
use heistarr_core::api::{Download, HistoryEvent, Indexer, Movie, MovieStatus, Settings};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    is_owner INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS sessions (
    token TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS kv (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS indexers (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    api_key TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS movies (
    id INTEGER PRIMARY KEY,
    tmdb_id INTEGER NOT NULL UNIQUE,
    imdb_id TEXT,
    title TEXT NOT NULL,
    year INTEGER,
    overview TEXT NOT NULL DEFAULT '',
    poster_path TEXT,
    status TEXT NOT NULL DEFAULT 'wanted',
    file_path TEXT,
    added_at INTEGER NOT NULL,
    last_search_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS downloads (
    id INTEGER PRIMARY KEY,
    movie_id INTEGER NOT NULL REFERENCES movies(id) ON DELETE CASCADE,
    release_title TEXT NOT NULL,
    download_url TEXT NOT NULL,
    client_hash TEXT,
    progress REAL NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'queued',
    active INTEGER NOT NULL DEFAULT 1,
    added_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS history (
    id INTEGER PRIMARY KEY,
    at INTEGER NOT NULL,
    kind TEXT NOT NULL,
    message TEXT NOT NULL
);
";

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub is_owner: bool,
}

fn movie_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Movie> {
    let status: String = r.get(7)?;
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
        added_at: r.get(9)?,
    })
}

const MOVIE_COLS: &str =
    "id, tmdb_id, imdb_id, title, year, overview, poster_path, status, file_path, added_at";

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let c = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(c)
    }

    pub fn memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(c: Connection) -> Result<Self> {
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        c.execute_batch(SCHEMA)?;
        Ok(Self(Arc::new(Mutex::new(c))))
    }

    fn c(&self) -> MutexGuard<'_, Connection> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---- accounts ----

    pub fn user_count(&self) -> Result<i64> {
        Ok(self.c().query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?)
    }

    pub fn create_user(&self, username: &str, hash: &str, owner: bool) -> Result<i64> {
        let c = self.c();
        c.execute(
            "INSERT INTO users (username, password_hash, is_owner) VALUES (?1, ?2, ?3)",
            params![username, hash, owner],
        )?;
        Ok(c.last_insert_rowid())
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
                |r| {
                    Ok(User {
                        id: r.get(0)?,
                        username: r.get(1)?,
                        password_hash: r.get(2)?,
                        is_owner: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn create_session(&self, token: &str, user_id: i64) -> Result<()> {
        self.c().execute(
            "INSERT INTO sessions (token, user_id, created_at) VALUES (?1, ?2, ?3)",
            params![token, user_id, now()],
        )?;
        Ok(())
    }

    pub fn delete_session(&self, token: &str) -> Result<()> {
        self.c().execute("DELETE FROM sessions WHERE token = ?1", [token])?;
        Ok(())
    }

    pub fn session_user(&self, token: &str, max_age: i64) -> Result<Option<User>> {
        Ok(self
            .c()
            .query_row(
                "SELECT u.id, u.username, u.password_hash, u.is_owner FROM sessions s
                 JOIN users u ON u.id = s.user_id WHERE s.token = ?1 AND s.created_at > ?2",
                params![token, now() - max_age],
                |r| {
                    Ok(User {
                        id: r.get(0)?,
                        username: r.get(1)?,
                        password_hash: r.get(2)?,
                        is_owner: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    // ---- settings ----

    pub fn settings(&self) -> Result<Settings> {
        let v: Option<String> = self
            .c()
            .query_row("SELECT value FROM kv WHERE key = 'settings'", [], |r| r.get(0))
            .optional()?;
        Ok(match v {
            Some(v) => serde_json::from_str(&v)?,
            None => Settings {
                qbit_category: "heistarr".into(),
                ..Settings::default()
            },
        })
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        self.c().execute(
            "INSERT INTO kv (key, value) VALUES ('settings', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [serde_json::to_string(s)?],
        )?;
        Ok(())
    }

    // ---- indexers ----

    pub fn indexers(&self) -> Result<Vec<Indexer>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, name, url, api_key, enabled FROM indexers ORDER BY id")?;
        let rows = st.query_map([], |r| {
            Ok(Indexer {
                id: r.get(0)?,
                name: r.get(1)?,
                url: r.get(2)?,
                api_key: r.get(3)?,
                enabled: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn save_indexer(&self, i: &Indexer) -> Result<i64> {
        let c = self.c();
        if i.id == 0 {
            c.execute(
                "INSERT INTO indexers (name, url, api_key, enabled) VALUES (?1, ?2, ?3, ?4)",
                params![i.name, i.url, i.api_key, i.enabled],
            )?;
            Ok(c.last_insert_rowid())
        } else {
            c.execute(
                "UPDATE indexers SET name = ?2, url = ?3, api_key = ?4, enabled = ?5 WHERE id = ?1",
                params![i.id, i.name, i.url, i.api_key, i.enabled],
            )?;
            Ok(i.id)
        }
    }

    pub fn delete_indexer(&self, id: i64) -> Result<()> {
        self.c().execute("DELETE FROM indexers WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---- movies ----

    pub fn movies(&self) -> Result<Vec<Movie>> {
        let c = self.c();
        let mut st = c.prepare(&format!("SELECT {MOVIE_COLS} FROM movies ORDER BY title"))?;
        let rows = st.query_map([], movie_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn movie(&self, id: i64) -> Result<Option<Movie>> {
        Ok(self
            .c()
            .query_row(&format!("SELECT {MOVIE_COLS} FROM movies WHERE id = ?1"), [id], movie_row)
            .optional()?)
    }

    pub fn movie_id_by_tmdb(&self, tmdb_id: i64) -> Result<Option<i64>> {
        Ok(self
            .c()
            .query_row("SELECT id FROM movies WHERE tmdb_id = ?1", [tmdb_id], |r| r.get(0))
            .optional()?)
    }

    pub fn insert_movie(&self, m: &Movie) -> Result<i64> {
        let c = self.c();
        c.execute(
            "INSERT INTO movies (tmdb_id, imdb_id, title, year, overview, poster_path, status, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![m.tmdb_id, m.imdb_id, m.title, m.year, m.overview, m.poster_path, m.status.as_str(), m.added_at],
        )?;
        Ok(c.last_insert_rowid())
    }

    pub fn delete_movie(&self, id: i64) -> Result<()> {
        self.c().execute("DELETE FROM movies WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_movie_status(&self, id: i64, s: MovieStatus, file: Option<&str>) -> Result<()> {
        self.c().execute(
            "UPDATE movies SET status = ?2, file_path = COALESCE(?3, file_path) WHERE id = ?1",
            params![id, s.as_str(), file],
        )?;
        Ok(())
    }

    /// Wanted movies not searched since `before`, oldest search first.
    pub fn movies_due_for_search(&self, before: i64, limit: i64) -> Result<Vec<Movie>> {
        let c = self.c();
        let mut st = c.prepare(&format!(
            "SELECT {MOVIE_COLS} FROM movies WHERE status = 'wanted' AND last_search_at < ?1
             ORDER BY last_search_at LIMIT ?2"
        ))?;
        let rows = st.query_map(params![before, limit], movie_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn mark_searched(&self, id: i64) -> Result<()> {
        self.c().execute("UPDATE movies SET last_search_at = ?2 WHERE id = ?1", params![id, now()])?;
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
        let mut st = c.prepare(
            "SELECT d.id, d.movie_id, m.title, d.release_title, d.client_hash, d.progress, d.state, d.added_at
             FROM downloads d JOIN movies m ON m.id = d.movie_id
             WHERE d.active >= ?1 ORDER BY d.added_at DESC LIMIT 200",
        )?;
        let rows = st.query_map([only_active as i64], |r| {
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
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
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
        let mut st = c.prepare("SELECT id, at, kind, message FROM history ORDER BY id DESC LIMIT ?1")?;
        let rows = st.query_map([limit], |r| {
            Ok(HistoryEvent { id: r.get(0)?, at: r.get(1)?, kind: r.get(2)?, message: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let m = Movie {
            id: 0,
            tmdb_id: 603,
            imdb_id: Some("tt0133093".into()),
            title: "The Matrix".into(),
            year: Some(1999),
            overview: String::new(),
            poster_path: None,
            status: MovieStatus::Wanted,
            file_path: None,
            added_at: 1,
        };
        let id = db.insert_movie(&m).unwrap();
        db.insert_download(id, "r", "u").unwrap();
        assert_eq!(db.downloads(true).unwrap().len(), 1);
        db.delete_movie(id).unwrap();
        assert!(db.downloads(false).unwrap().is_empty());
    }
}
