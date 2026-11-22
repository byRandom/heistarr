//! Torznab indexers (Jackett, Prowlarr and compatible).

use crate::net::Net;
use anyhow::{Context, Result, bail};
use heistarr_core::api::Indexer;

/// One `<item>` of a Torznab answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub title: String,
    /// `.torrent` URL or `magnet:` link.
    pub download_url: String,
    pub size: u64,
    pub seeders: Option<u32>,
}

const TORZNAB_NS: &str = "http://torznab.com/schemas/2015/feed";

pub fn parse_feed(xml: &str) -> Result<Vec<Item>> {
    let doc = roxmltree::Document::parse(xml).context("the indexer did not answer with XML")?;
    let root = doc.root_element();
    if root.has_tag_name("error") {
        bail!(
            "indexer error {}: {}",
            root.attribute("code").unwrap_or("?"),
            root.attribute("description").unwrap_or("")
        );
    }
    let mut out = Vec::new();
    for item in doc.descendants().filter(|n| n.has_tag_name("item")) {
        let child_text = |name: &str| {
            item.children()
                .find(|c| {
                    c.is_element()
                        && c.tag_name().name() == name
                        && c.tag_name().namespace().is_none()
                })
                .and_then(|c| c.text())
                .map(str::trim)
                .filter(|s| !s.is_empty())
        };
        let attr = |name: &str| {
            item.children()
                .find(|c| {
                    c.is_element()
                        && c.tag_name().name() == "attr"
                        && c.tag_name().namespace() == Some(TORZNAB_NS)
                        && c.attribute("name") == Some(name)
                })
                .and_then(|c| c.attribute("value"))
        };
        let enclosure = item.children().find(|c| c.has_tag_name("enclosure"));

        let Some(title) = child_text("title") else {
            continue;
        };
        let url = attr("magneturl")
            .filter(|_| {
                enclosure.and_then(|e| e.attribute("url")).is_none() && child_text("link").is_none()
            })
            .or_else(|| enclosure.and_then(|e| e.attribute("url")))
            .or_else(|| child_text("link"));
        let Some(url) = url else { continue };
        let size = child_text("size")
            .or_else(|| attr("size"))
            .or_else(|| enclosure.and_then(|e| e.attribute("length")))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let seeders = attr("seeders").and_then(|s| s.parse().ok());
        out.push(Item {
            title: title.to_string(),
            download_url: url.to_string(),
            size,
            seeders,
        });
    }
    Ok(out)
}

fn base(ix: &Indexer) -> String {
    let u = ix.url.trim().trim_end_matches('/');
    if u.ends_with("/api") {
        u.to_string()
    } else {
        format!("{u}/api")
    }
}

async fn query(net: &Net, ix: &Indexer, params: &[(&str, String)]) -> Result<String> {
    let url = base(ix);
    let key = ix.api_key.trim().to_string();
    let r = net
        .send(reqwest::Method::GET, &url, |r| {
            let r = r.query(params);
            if key.is_empty() {
                r
            } else {
                r.query(&[("apikey", &key)])
            }
        })
        .await?;
    let r = crate::net::ok(r, &url).await?;
    Ok(r.text().await?)
}

/// Searches one indexer for a movie: by IMDb id first, then by title and
/// year when the id search finds nothing.
pub async fn search_movie(
    net: &Net,
    ix: &Indexer,
    imdb_id: Option<&str>,
    title: &str,
    year: Option<i32>,
) -> Result<Vec<Item>> {
    let cat = ("cat", "2000".to_string());
    if let Some(id) = imdb_id.filter(|i| i.starts_with("tt")) {
        let xml = query(
            net,
            ix,
            &[
                ("t", "movie".into()),
                ("imdbid", id.trim_start_matches("tt").into()),
                cat.clone(),
            ],
        )
        .await?;
        let items = parse_feed(&xml)?;
        if !items.is_empty() {
            return Ok(items);
        }
    }
    let q = match year {
        Some(y) => format!("{title} {y}"),
        None => title.to_string(),
    };
    let xml = query(net, ix, &[("t", "movie".into()), ("q", q), cat]).await?;
    parse_feed(&xml)
}

/// The "Test" button: asks for the capabilities document.
pub async fn test(net: &Net, ix: &Indexer) -> Result<()> {
    let xml = query(net, ix, &[("t", "caps".into())]).await?;
    let doc = roxmltree::Document::parse(&xml).context("the indexer did not answer with XML")?;
    let root = doc.root_element();
    if root.has_tag_name("error") {
        bail!(
            "{}",
            root.attribute("description").unwrap_or("indexer error")
        );
    }
    if !root.has_tag_name("caps") {
        bail!("not a Torznab endpoint (no <caps>)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:torznab="http://torznab.com/schemas/2015/feed">
<channel>
<item>
  <title>The.Matrix.1999.1080p.BluRay.x264-GROUP</title>
  <link>http://jackett/dl/1.torrent</link>
  <size>8589934592</size>
  <enclosure url="http://jackett/dl/1.torrent" length="8589934592" type="application/x-bittorrent"/>
  <torznab:attr name="seeders" value="120"/>
</item>
<item>
  <title>The.Matrix.1999.720p.WEB</title>
  <torznab:attr name="magneturl" value="magnet:?xt=urn:btih:abc"/>
  <torznab:attr name="size" value="2000"/>
</item>
<item><link>http://x/no-title</link></item>
</channel></rss>"#;

    #[test]
    fn parses_items() {
        let items = parse_feed(FEED).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].size, 8589934592);
        assert_eq!(items[0].seeders, Some(120));
        assert_eq!(items[1].download_url, "magnet:?xt=urn:btih:abc");
        assert_eq!(items[1].size, 2000);
        assert_eq!(items[1].seeders, None);
    }

    #[test]
    fn error_document() {
        let e = parse_feed(r#"<error code="100" description="Invalid API Key"/>"#).unwrap_err();
        assert!(e.to_string().contains("Invalid API Key"));
    }

    #[test]
    fn base_url() {
        let ix = |u: &str| Indexer {
            url: u.into(),
            ..Default::default()
        };
        assert_eq!(
            base(&ix("http://j:9117/api/v2.0/indexers/all/results/torznab/")),
            "http://j:9117/api/v2.0/indexers/all/results/torznab/api"
        );
        assert_eq!(
            base(&ix("http://prowlarr:9696/1/")),
            "http://prowlarr:9696/1/api"
        );
        assert_eq!(
            base(&ix("http://prowlarr:9696/1/api")),
            "http://prowlarr:9696/1/api"
        );
    }
}
