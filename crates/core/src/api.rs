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
    pub year: Option<i32>,
    pub overview: String,
    pub poster_path: Option<String>,
    pub status: MovieStatus,
    /// Path of the imported file, once available.
    pub file_path: Option<String>,
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
    pub qbit_url: String,
    pub qbit_username: String,
    pub qbit_password: String,
    pub qbit_category: String,
    /// `jellyfin`, `plex` or empty.
    pub media_server_kind: String,
    pub media_server_url: String,
    pub media_server_token: String,
    pub rules: crate::rules::PickRules,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}
