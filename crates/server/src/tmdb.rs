//! The Movie Database (TMDB) client: catalog search and movie details.
//! Each installation uses its own key (v3 API key or v4 read token).

use crate::net::Net;
use anyhow::{Result, bail};
use heistarr_core::api::CatalogMovie;
use serde::Deserialize;

pub const DEFAULT_API: &str = "https://api.themoviedb.org/3";
pub const DEFAULT_IMAGES: &str = "https://image.tmdb.org/t/p";

pub struct Tmdb<'a> {
    pub net: &'a Net,
    pub base: &'a str,
    pub key: &'a str,
    /// e.g. `es-ES`; empty for TMDB's default (English).
    pub language: &'a str,
}

#[derive(Deserialize)]
struct Page {
    results: Vec<Hit>,
}

#[derive(Deserialize)]
struct Hit {
    id: i64,
    title: Option<String>,
    release_date: Option<String>,
    overview: Option<String>,
    poster_path: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Details {
    pub id: i64,
    pub imdb_id: Option<String>,
    pub title: String,
    pub original_title: Option<String>,
    pub release_date: Option<String>,
    #[serde(default)]
    pub overview: Option<String>,
    pub poster_path: Option<String>,
}

impl Details {
    pub fn year(&self) -> Option<i32> {
        year_of(self.release_date.as_deref())
    }
}

fn year_of(date: Option<&str>) -> Option<i32> {
    date.and_then(|d| d.get(..4)).and_then(|y| y.parse().ok())
}

impl Tmdb<'_> {
    fn check(&self) -> Result<()> {
        if self.key.trim().is_empty() {
            bail!("no TMDB key: add one in Settings");
        }
        Ok(())
    }

    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        self.check()?;
        let key = self.key.trim();
        // v4 read tokens are JWTs; v3 keys are 32 hex characters.
        let bearer = key.starts_with("eyJ");
        let mut q: Vec<(&str, String)> = query.to_vec();
        if !self.language.is_empty() {
            q.push(("language", self.language.to_string()));
        }
        if !bearer {
            q.push(("api_key", key.to_string()));
        }
        let url = format!("{}{path}", self.base.trim_end_matches('/'));
        self.net
            .get_json(&url, |r| {
                let r = r.query(&q).header("accept", "application/json");
                if bearer { r.bearer_auth(key) } else { r }
            })
            .await
    }

    pub async fn search(&self, query: &str, year: Option<i32>) -> Result<Vec<CatalogMovie>> {
        let mut q = vec![
            ("query", query.to_string()),
            ("include_adult", "false".into()),
        ];
        if let Some(y) = year {
            q.push(("year", y.to_string()));
        }
        let page: Page = self.get("/search/movie", &q).await?;
        Ok(page
            .results
            .into_iter()
            .map(|h| CatalogMovie {
                tmdb_id: h.id,
                title: h.title.unwrap_or_default(),
                year: year_of(h.release_date.as_deref()),
                overview: h.overview.unwrap_or_default(),
                poster_path: h.poster_path,
                library_id: None,
            })
            .collect())
    }

    pub async fn movie(&self, id: i64) -> Result<Details> {
        self.get(&format!("/movie/{id}"), &[]).await
    }

    /// Checks the key with a cheap call.
    pub async fn test(&self) -> Result<()> {
        let _: serde_json::Value = self.get("/configuration", &[]).await?;
        Ok(())
    }
}

/// Whether `p` is a TMDB image path we are willing to proxy.
pub fn valid_image_path(size: &str, file: &str) -> bool {
    matches!(
        size,
        "w92" | "w154" | "w185" | "w342" | "w500" | "w780" | "original"
    ) && !file.is_empty()
        && file.len() < 100
        && file
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
        && !file.starts_with('.')
}
