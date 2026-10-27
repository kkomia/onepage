use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use regex::Regex;
use rusqlite::{params, OptionalExtension};
use tokio::task::spawn_blocking;
use ureq::ResponseExt;
use url::Url;

use crate::config::Config;
use crate::db::Database;
use crate::services::links;
use crate::util::{now_rfc3339, sha256_bytes_short};

const MAX_BODY: u64 = 512 * 1024;
const MAX_ICON: usize = 256 * 1024;

pub fn build_agent(_config: &Config) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(6)))
        .timeout_connect(Some(Duration::from_secs(4)))
        .max_redirects(3)
        .http_status_as_error(false)
        .user_agent(format!("OnePage/{}", env!("CARGO_PKG_VERSION")))
        .tls_config(tls)
        .build();
    ureq::Agent::new_with_config(config)
}

pub async fn refresh_for_link(
    db: &Database,
    agent: &ureq::Agent,
    config: &Config,
    link_id: i64,
) -> Result<Option<String>> {
    if !config.icon_fetch {
        return Ok(None);
    }
    let Some(link) = links::link_by_id(db, link_id).await? else {
        return Ok(None);
    };
    let Some(link_url) = Url::parse(&link.url).ok() else {
        return Ok(None);
    };
    let agent = agent.clone();
    let source_url = link_url.to_string();

    let fetched = tokio::time::timeout(
        Duration::from_secs(10),
        spawn_blocking(move || fetch_icon(&agent, &link_url)),
    )
    .await
    .ok()
    .and_then(|result| result.ok())
    .flatten();

    let Some(icon) = fetched else {
        return Ok(None);
    };

    let hash = sha256_bytes_short(&icon.bytes);
    let hash_for_db = hash.clone();
    let source_for_db = source_url.clone();
    let content_type = icon.content_type.to_string();
    let bytes = icon.bytes;
    let expected_url = link.url.clone();
    let now = now_rfc3339();

    db.call(move |conn| {
        conn.execute(
            "INSERT INTO icons (hash, content_type, data, source_url, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(hash) DO UPDATE SET fetched_at = excluded.fetched_at",
            params![hash_for_db, content_type, bytes, source_for_db, now],
        )?;
        let icon_id: i64 = conn.query_row(
            "SELECT id FROM icons WHERE hash = ?1",
            params![hash_for_db],
            |row| row.get(0),
        )?;
        conn.execute(
            "UPDATE links SET icon_id = ?1 WHERE id = ?2 AND url = ?3",
            params![icon_id, link_id, expected_url],
        )?;
        Ok(())
    })
    .await?;

    Ok(Some(hash))
}

pub async fn icon_by_hash(
    db: &Database,
    hash: &str,
) -> Result<Option<(String, Vec<u8>, String)>> {
    let hash = hash.to_string();
    db.call(move |conn| {
        let row = conn
            .query_row(
                "SELECT content_type, data, fetched_at FROM icons WHERE hash = ?1",
                params![hash],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(row)
    })
    .await
}

struct FetchedIcon {
    content_type: &'static str,
    bytes: Vec<u8>,
}

fn fetch_icon(agent: &ureq::Agent, page_url: &Url) -> Option<FetchedIcon> {
    let started = Instant::now();
    let looks_like_image = [".png", ".ico", ".jpg", ".jpeg", ".gif", ".webp"]
        .iter()
        .any(|ext| page_url.path().to_ascii_lowercase().ends_with(ext));

    if looks_like_image {
        if let Some(icon) = fetch_image(agent, page_url.as_str()) {
            tracing::debug!(url = %page_url, elapsed = ?started.elapsed(), "icon fetched from link");
            return Some(icon);
        }
    }

    if let Ok(root_icon) = page_url.join("/favicon.ico") {
        if let Some(icon) = fetch_image(agent, root_icon.as_str()) {
            tracing::debug!(url = %root_icon, elapsed = ?started.elapsed(), "icon fetched from favicon.ico");
            return Some(icon);
        }
    }

    let Ok(response) = agent
        .get(page_url.as_str())
        .header("Accept", "text/html,application/xhtml+xml")
        .call()
    else {
        return None;
    };
    if !response.status().is_success() {
        return None;
    }
    let final_url = response
        .get_uri()
        .to_string()
        .parse::<Url>()
        .unwrap_or_else(|_| page_url.clone());
    let mut response = response;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_vec()
        .ok()?;
    let html = String::from_utf8_lossy(&bytes);

    for candidate in icon_candidates(&html) {
        let Ok(icon_url) = final_url.join(&candidate) else {
            continue;
        };
        if !matches!(icon_url.scheme(), "http" | "https") {
            continue;
        }
        if let Some(icon) = fetch_image(agent, icon_url.as_str()) {
            tracing::debug!(url = %icon_url, elapsed = ?started.elapsed(), "icon fetched from link rel");
            return Some(icon);
        }
    }

    if !looks_like_image {
        if let Some(icon) = fetch_image(agent, page_url.as_str()) {
            return Some(icon);
        }
    }

    None
}

fn fetch_image(agent: &ureq::Agent, url: &str) -> Option<FetchedIcon> {
    let mut response = agent
        .get(url)
        .header("Accept", "image/png,image/x-icon,image/vnd.microsoft.icon,image/jpeg,image/webp,image/*;q=0.8")
        .call()
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_vec()
        .ok()?;
    if bytes.len() < 32 || bytes.len() > MAX_ICON {
        return None;
    }
    let content_type = sniff_image(&bytes)?;
    Some(FetchedIcon {
        content_type,
        bytes,
    })
}

fn icon_candidates(html: &str) -> Vec<String> {
    static LINK_TAG: OnceLock<Regex> = OnceLock::new();
    static REL_ATTR: OnceLock<Regex> = OnceLock::new();
    static HREF_ATTR: OnceLock<Regex> = OnceLock::new();
    let link_tag = LINK_TAG.get_or_init(|| Regex::new(r"(?is)<link\b[^>]*>").expect("valid regex"));
    let rel_attr = REL_ATTR.get_or_init(|| {
        Regex::new(r#"(?is)\brel\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).expect("valid regex")
    });
    let href_attr = HREF_ATTR.get_or_init(|| {
        Regex::new(r#"(?is)\bhref\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).expect("valid regex")
    });

    let mut rel_icons = Vec::new();
    let mut apple_icons = Vec::new();

    for tag in link_tag.find_iter(html).take(200) {
        let tag = tag.as_str();
        let Some(rel) = attr_value(rel_attr, tag) else {
            continue;
        };
        let rel_lower = rel.to_ascii_lowercase();
        if !rel_lower.split_whitespace().any(|part| part.contains("icon")) {
            continue;
        }
        let Some(href) = attr_value(href_attr, tag) else {
            continue;
        };
        let href = href.trim();
        if href.is_empty() || href.starts_with("data:") || href.len() > 2048 {
            continue;
        }
        if rel_lower.contains("apple-touch-icon") {
            apple_icons.push(href.to_string());
        } else {
            rel_icons.push(href.to_string());
        }
    }

    rel_icons.extend(apple_icons);
    rel_icons.truncate(5);
    rel_icons
}

fn attr_value(regex: &Regex, tag: &str) -> Option<String> {
    let captures = regex.captures(tag)?;
    (1..=3)
        .find_map(|index| captures.get(index))
        .map(|value| value.as_str().to_string())
}

fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0x00, 0x00, 0x01, 0x00]) {
        return Some("image/x-icon");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

pub async fn spawn_refresh(
    db: Database,
    agent: Arc<ureq::Agent>,
    config: Arc<Config>,
    link_id: i64,
) {
    let result = refresh_for_link(&db, &agent, &config, link_id).await;
    if let Err(error) = result {
        tracing::debug!(link_id, error = %error, "icon refresh failed");
    }
}
