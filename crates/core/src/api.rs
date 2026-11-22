//! JSON shapes of the HTTP API. The UI and the server both use these, so a
//! field rename breaks the build instead of the app.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MovieStatus {
    /// In the library list, nothing grabbed yet.
    Wanted,
    /// A release was sent to the downloader.
    Downloading,
    /// The file is in the library folder.
    Available,
}

impl MovieStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wanted => "wanted",
            Self::Downloading => "downloading",
            Self::Available => "available",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "wanted" => Self::Wanted,
            "downloading" => Self::Downloading,
            "available" => Self::Available,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Movie {
    pub id: i64,
    pub tmdb_id: i64,
    pub imdb_id: Option<String>,
    pub title: String,
    /// Title in the original language, when it differs (releases often use
    /// it instead of the translated one).
    #[serde(default)]
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub overview: String,
    pub poster_path: Option<String>,
    pub status: MovieStatus,
    /// Path of the imported file, once available.
    pub file_path: Option<String>,
    /// Quality of the imported file (`None` when unknown or no file).
    pub file_quality: Option<crate::rules::Quality>,
    pub added_at: i64,
}

/// A catalog search hit (from the metadata provider), not yet in the library.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogMovie {
    pub tmdb_id: i64,
    pub title: String,
    pub year: Option<i32>,
    pub overview: String,
    pub poster_path: Option<String>,
    /// Already in the library under this id.
    pub library_id: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AddMovie {
    pub tmdb_id: i64,
    /// Search and grab the best release right away.
    pub search_now: bool,
}

/// A release found on an indexer, with its score under the pick rules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub title: String,
    pub indexer: String,
    pub download_url: String,
    pub size: u64,
    pub seeders: Option<u32>,
    /// `None` when the pick rules reject it; the reason is in `rejections`.
    pub score: Option<i32>,
    pub rejections: Vec<String>,
    pub quality: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MovieDetail {
    pub movie: Movie,
    pub downloads: Vec<Download>,
    pub blocklist: Vec<BlockedRelease>,
}

/// A release that failed or was removed by hand; never grabbed again for
/// that movie.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlockedRelease {
    pub id: i64,
    pub movie_id: i64,
    pub release_title: String,
    pub reason: String,
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Grab {
    pub movie_id: i64,
    pub release: Release,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Download {
    pub id: i64,
    pub movie_id: i64,
    pub movie_title: String,
    pub release_title: String,
    pub client_hash: Option<String>,
    pub progress: f32,
    pub state: String,
    pub added_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryEvent {
    pub id: i64,
    pub at: i64,
    pub kind: String,
    pub message: String,
}

/// What a secret looks like when sent to the UI: only the last 4 characters.
/// Saving a value that still starts with the mask keeps the stored secret.
pub const SECRET_MASK: &str = "••••";

pub fn mask_secret(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let tail: String = s
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{SECRET_MASK}{tail}")
}

pub fn is_masked(s: &str) -> bool {
    s.starts_with(SECRET_MASK)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Indexer {
    pub id: i64,
    pub name: String,
    /// Torznab endpoint, e.g. `http://jackett:9117/api/v2.0/indexers/all/results/torznab`.
    pub url: String,
    pub api_key: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// TMDB API read token (v4 "API Read Access Token") or v3 key.
    pub tmdb_key: String,
    /// Root folder that holds one folder per movie.
    pub movies_root: String,
    /// Download client: `qbittorrent` or `synology` (Download Station).
    #[serde(default = "default_client")]
    pub download_client: String,
    pub qbit_url: String,
    pub qbit_username: String,
    pub qbit_password: String,
    pub qbit_category: String,
    /// Synology DSM address, e.g. `http://nas:5000`.
    #[serde(default)]
    pub synology_url: String,
    #[serde(default)]
    pub synology_username: String,
    #[serde(default)]
    pub synology_password: String,
    /// Shared folder path for new tasks, e.g. `data/torrents`. Empty: the
    /// Download Station default.
    #[serde(default)]
    pub synology_destination: String,
    /// Remove the torrent (and its data) from qBittorrent after import, or
    /// keep seeding it.
    #[serde(default)]
    pub remove_after_import: bool,
    /// When qBittorrent sees another filesystem (Docker), its save path
    /// prefix and what that prefix is here. Empty: same paths.
    #[serde(default)]
    pub remote_path: String,
    #[serde(default)]
    pub local_path: String,
    /// Language for TMDB titles and overviews, e.g. `es-ES`. Empty: English.
    #[serde(default)]
    pub metadata_language: String,
    /// Theme id, e.g. `miami`.
    #[serde(default)]
    pub theme: String,
    /// `jellyfin`, `plex` or empty.
    pub media_server_kind: String,
    pub media_server_url: String,
    pub media_server_token: String,
    pub rules: crate::rules::PickRules,
}

fn default_client() -> String {
    "qbittorrent".into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Me {
    pub username: String,
    pub is_owner: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetupState {
    /// No account exists yet: the first visitor creates the owner.
    pub needs_owner: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Login {
    pub username: String,
    pub password: String,
}

/// Result of a "Test" button.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanReport {
    pub matched: Vec<String>,
    pub unmatched: Vec<String>,
    pub already: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}
