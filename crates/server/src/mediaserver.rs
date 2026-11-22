//! Telling Jellyfin or Plex to rescan after an import.

use crate::net::{Net, ok};
use anyhow::{Result, bail};
use heistarr_core::api::Settings;
use reqwest::Method;

pub async fn refresh(net: &Net, s: &Settings) -> Result<()> {
    let base = s.media_server_url.trim().trim_end_matches('/');
    let token = s.media_server_token.trim();
    match s.media_server_kind.as_str() {
        "" => Ok(()),
        "jellyfin" | "emby" => {
            let url = format!("{base}/Library/Refresh");
            let r = net
                .send(Method::POST, &url, |r| r.header("X-Emby-Token", token))
                .await?;
            ok(r, &url).await?;
            Ok(())
        }
        "plex" => {
            let url = format!("{base}/library/sections/all/refresh");
            let r = net
                .send(Method::GET, &url, |r| r.header("X-Plex-Token", token))
                .await?;
            ok(r, &url).await?;
            Ok(())
        }
        other => bail!("unknown media server kind \"{other}\""),
    }
}

/// The "Test" button: an authenticated read.
pub async fn test(net: &Net, s: &Settings) -> Result<String> {
    let base = s.media_server_url.trim().trim_end_matches('/');
    let token = s.media_server_token.trim();
    match s.media_server_kind.as_str() {
        "jellyfin" | "emby" => {
            let url = format!("{base}/System/Info");
            let v: serde_json::Value = net
                .get_json(&url, |r| r.header("X-Emby-Token", token))
                .await?;
            Ok(format!(
                "Connected to {} {}",
                v["ServerName"].as_str().unwrap_or("server"),
                v["Version"].as_str().unwrap_or("")
            ))
        }
        "plex" => {
            let url = format!("{base}/identity");
            net.get_bytes(&url, |r| {
                r.header("X-Plex-Token", token)
                    .header("Accept", "application/json")
            })
            .await?;
            let url = format!("{base}/library/sections");
            net.get_bytes(&url, |r| {
                r.header("X-Plex-Token", token)
                    .header("Accept", "application/json")
            })
            .await?;
            Ok("Connected to Plex".into())
        }
        "" => bail!("choose Jellyfin or Plex first"),
        other => bail!("unknown media server kind \"{other}\""),
    }
}
