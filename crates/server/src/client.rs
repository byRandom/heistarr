//! One interface over the supported download clients (qBittorrent and
//! Synology Download Station), so the engine does not care which is used.

use crate::importer::map_path;
use crate::qbit::{self, Qbit};
use crate::state::AppState;
use crate::synology::Ds;
use anyhow::Result;
use heistarr_core::api::Settings;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemState {
    Downloading,
    /// No progress possible right now (no peers, fetching metadata).
    Stalled,
    Completed,
    Failed,
}

/// A transfer as the client reports it.
#[derive(Clone, Debug)]
pub struct Item {
    /// The client's id: torrent hash (qBittorrent) or task id (Synology).
    pub id: String,
    pub name: String,
    pub progress: f32,
    pub state: ItemState,
    /// The client's own words for the state, shown in the UI.
    pub raw_state: String,
    /// The downloaded files, as a path on this machine (mapped).
    pub content_path: PathBuf,
    /// The URL it was added from, when the client reports it.
    pub source_url: String,
    pub error: Option<String>,
    /// qBittorrent tags, comma separated.
    pub tags: String,
}

pub enum Client<'a> {
    Qbit { api: Qbit<'a>, category: &'a str },
    Synology(Ds<'a>),
}

pub struct Mapping<'a> {
    remote: &'a str,
    local: &'a str,
}

impl<'a> Client<'a> {
    pub fn new(app: &'a AppState, s: &'a Settings) -> (Self, Mapping<'a>) {
        let mapping = Mapping {
            remote: &s.remote_path,
            local: &s.local_path,
        };
        let c = if s.download_client == "synology" {
            Self::Synology(Ds {
                net: &app.net,
                session: &app.synology,
                url: &s.synology_url,
                username: &s.synology_username,
                password: &s.synology_password,
                destination: &s.synology_destination,
            })
        } else {
            Self::Qbit {
                api: Qbit {
                    net: &app.net,
                    session: &app.qbit,
                    url: &s.qbit_url,
                    username: &s.qbit_username,
                    password: &s.qbit_password,
                },
                category: &s.qbit_category,
            }
        };
        (c, mapping)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Qbit { .. } => "qBittorrent",
            Self::Synology(_) => "Download Station",
        }
    }

    pub async fn test(&self) -> Result<String> {
        match self {
            Self::Qbit { api, .. } => Ok(format!(
                "Connected to qBittorrent {}",
                api.test().await?.trim()
            )),
            Self::Synology(ds) => ds.test().await,
        }
    }

    /// Sends a release. Returns the client's id for it when known right away.
    pub async fn add(&self, url: &str, download_id: i64) -> Result<Option<String>> {
        match self {
            Self::Qbit { api, category } => {
                api.add(url, category, &qbit::tag_for(download_id)).await?;
                Ok(magnet_hash(url))
            }
            Self::Synology(ds) => ds.add(url).await,
        }
    }

    pub async fn list(&self, m: &Mapping<'_>) -> Result<Vec<Item>> {
        match self {
            Self::Qbit { api, category } => Ok(api
                .torrents(category)
                .await?
                .into_iter()
                .map(|t| {
                    let state = if t.is_error() {
                        ItemState::Failed
                    } else if t.is_complete() {
                        ItemState::Completed
                    } else if t.is_stalled() {
                        ItemState::Stalled
                    } else {
                        ItemState::Downloading
                    };
                    let path = if t.content_path.is_empty() {
                        format!("{}/{}", t.save_path, t.name)
                    } else {
                        t.content_path.clone()
                    };
                    Item {
                        id: t.hash.to_ascii_lowercase(),
                        content_path: map_path(&path, m.remote, m.local),
                        name: t.name,
                        progress: t.progress,
                        error: (state == ItemState::Failed).then(|| t.state.clone()),
                        raw_state: t.state,
                        state,
                        source_url: String::new(),
                        tags: t.tags,
                    }
                })
                .collect()),
            Self::Synology(ds) => Ok(ds
                .tasks()
                .await?
                .into_iter()
                .map(|t| {
                    let state = if t.is_error() {
                        ItemState::Failed
                    } else if t.is_complete() {
                        ItemState::Completed
                    } else if t.status == "downloading" && t.downloaded == 0 {
                        ItemState::Stalled
                    } else {
                        ItemState::Downloading
                    };
                    Item {
                        id: t.id.clone(),
                        name: t.title.clone(),
                        progress: t.progress(),
                        state,
                        raw_state: t.status.clone(),
                        content_path: map_path(&t.content_path(), m.remote, m.local),
                        source_url: t.uri.clone(),
                        error: t
                            .error
                            .clone()
                            .or_else(|| t.is_error().then(|| "error".to_string())),
                        tags: String::new(),
                    }
                })
                .collect()),
        }
    }

    /// Removes a transfer. qBittorrent also deletes its files when asked;
    /// Download Station keeps finished files in the download folder.
    pub async fn remove(&self, id: &str, delete_files: bool) -> Result<()> {
        match self {
            Self::Qbit { api, .. } => api.delete(id, delete_files).await,
            Self::Synology(ds) => ds.delete(id).await,
        }
    }
}

/// Finds the transfer for one of our downloads: by the stored id, then by
/// our qBittorrent tag, then by the URL it was added from, then by name.
pub fn find<'i>(
    items: &'i [Item],
    stored_id: Option<&str>,
    download_id: i64,
    url: &str,
    release_title: &str,
) -> Option<&'i Item> {
    if let Some(id) = stored_id
        && let Some(i) = items.iter().find(|i| i.id.eq_ignore_ascii_case(id))
    {
        return Some(i);
    }
    let tag = qbit::tag_for(download_id);
    items
        .iter()
        .find(|i| i.tags.split(',').any(|t| t.trim() == tag))
        .or_else(|| {
            items
                .iter()
                .find(|i| !url.is_empty() && i.source_url == url)
        })
        .or_else(|| items.iter().find(|i| i.name == release_title))
}

/// The info hash of a magnet link, lowercased hex.
pub fn magnet_hash(url: &str) -> Option<String> {
    let rest = url.strip_prefix("magnet:?")?;
    rest.split('&').find_map(|kv| {
        let v = kv.strip_prefix("xt=urn:btih:")?;
        (v.len() == 40 && v.chars().all(|c| c.is_ascii_hexdigit())).then(|| v.to_ascii_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnet() {
        assert_eq!(
            magnet_hash("magnet:?xt=urn:btih:ABCDEF0123456789ABCDEF0123456789ABCDEF01&dn=x")
                .as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef01")
        );
        assert_eq!(magnet_hash("http://x/1.torrent"), None);
    }
}
