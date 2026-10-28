use std::net::{IpAddr, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use regex::Regex;
use rusqlite::{params, OptionalExtension};
use tokio::sync::Semaphore;
use tokio::task::spawn_blocking;
use url::Url;

use crate::config::Config;
use crate::db::Database;
use crate::services::links;
use crate::util::{now_rfc3339, sha256_bytes_short};

const MAX_BODY: u64 = 512 * 1024;
const MAX_ICON: usize = 256 * 1024;
const FETCH_BUDGET: Duration = Duration::from_secs(4);
const MAX_HOPS: usize = 3;
const MAX_CONCURRENT_FETCHES: usize = 4;

static ICON_GATE: Semaphore = Semaphore::const_new(MAX_CONCURRENT_FETCHES);

pub fn build_agent(_config: &Config) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(6)))
        .timeout_connect(Some(Duration::from_secs(4)))
        // Redirects are followed by `guarded_get` so that every hop is checked
        // for internal addresses before the request leaves the process.
        .max_redirects(0)
        .http_status_as_error(false)
        .user_agent(format!("OnePage/{}", env!("CARGO_PKG_VERSION")))
        .tls_config(tls)
        .build();
    ureq::Agent::new_with_config(config)
}

/// Whether the server may fetch this URL. Loopback, link-local (including cloud
/// metadata endpoints), multicast and unspecified addresses are always refused;
/// site-local ranges are refused unless `allow_private` is set, because an
/// intranet navigator legitimately points at internal hosts.
pub fn address_allowed(ip: IpAddr, allow_private: bool) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() || v4.is_link_local() || v4.is_unspecified() {
                return false;
            }
            if v4.is_multicast() || v4.is_broadcast() || v4.octets()[0] >= 240 {
                return false;
            }
            if !allow_private && (v4.is_private() || is_cgnat(v4)) {
                return false;
            }
            true
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() {
                return false;
            }
            let first = v6.segments()[0];
            // fe80::/10 link-local and ff00::/8 multicast.
            if first & 0xffc0 == 0xfe80 || first & 0xff00 == 0xff00 {
                return false;
            }
            // fc00::/7 unique-local and 2002::/16 Teredo (embeds IPv4 targets).
            if !allow_private && (first & 0xfe00 == 0xfc00 || first == 0x2001 && v6.segments()[1] == 0xdb8)
            {
                return false;
            }
            true
        }
    }
}

fn is_cgnat(ip: std::net::Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (64..=127).contains(&octets[1])
}

fn host_addresses(url: &Url) -> Option<Vec<IpAddr>> {
    let host = url.host_str()?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(vec![ip]);
    }
    let port = url.port_or_known_default()?;
    let resolved = (host, port).to_socket_addrs().ok()?;
    let addresses = resolved.map(|socket| socket.ip()).collect::<Vec<_>>();
    // Fail closed: an unresolvable host must not be fetched.
    if addresses.is_empty() {
        return None;
    }
    Some(addresses)
}

pub fn url_allowed(url: &Url, allow_private: bool) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    if url.host_str().is_none() {
        return false;
    }
    match host_addresses(url) {
        // Metadata services such as 169.254.169.254 are reachable through DNS
        // too, so every resolved record has to pass the check.
        Some(addresses) => addresses
            .into_iter()
            .all(|ip| address_allowed(ip, allow_private)),
        None => false,
    }
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

    // The permit is held for the whole fetch, so a slow internet cannot occupy
    // more than MAX_CONCURRENT_FETCHES blocking threads at a time.
    let permit = match ICON_GATE.try_acquire() {
        Ok(permit) => permit,
        Err(_) => {
            tracing::debug!(link_id, "icon fetch skipped, concurrency limit reached");
            return Ok(None);
        }
    };
    let allow_private = config.icon_allow_private;

    let fetched = tokio::time::timeout(
        FETCH_BUDGET + Duration::from_secs(6),
        spawn_blocking(move || {
            let _permit = permit;
            fetch_icon(&agent, &link_url, allow_private)
        }),
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

/// Deletes icon blobs that no longer reference any link. Rows only ever grow
/// otherwise, and the database directory doubles as the backup unit.
pub async fn prune_orphan_icons(db: &Database) -> Result<u64> {
    db.call(move |conn| {
        let changed = conn.execute(
            "DELETE FROM icons
             WHERE id NOT IN (SELECT icon_id FROM links WHERE icon_id IS NOT NULL)",
            [],
        )? as u64;
        Ok(changed)
    })
    .await
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

struct Budget {
    deadline: Instant,
}

impl Budget {
    fn new() -> Self {
        Self {
            deadline: Instant::now() + FETCH_BUDGET,
        }
    }

    fn expired(&self) -> bool {
        Instant::now() >= self.deadline
    }
}

/// Fetches a URL, following redirects by hand so that each hop is re-checked
/// against the address policy before the request is sent.
fn guarded_get(
    agent: &ureq::Agent,
    start: &Url,
    accept: &str,
    budget: &Budget,
    allow_private: bool,
) -> Option<(Url, Vec<u8>)> {
    let mut current = start.clone();
    for _ in 0..=MAX_HOPS {
        if budget.expired() {
            tracing::debug!(url = %current, "icon fetch stopped: time budget exhausted");
            return None;
        }
        if !url_allowed(&current, allow_private) {
            tracing::debug!(url = %current, "icon fetch refused by address policy");
            return None;
        }
        let mut response = agent
            .get(current.as_str())
            .header("Accept", accept)
            .call()
            .ok()?;
        let status = response.status();
        if status.is_success() {
            let bytes = response
                .body_mut()
                .with_config()
                .limit(MAX_BODY)
                .read_to_vec()
                .ok()?;
            return Some((current, bytes));
        }
        if !status.is_redirection() {
            return None;
        }
        let location = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)?;
        current = current.join(&location).ok()?;
    }
    None
}

fn fetch_icon(agent: &ureq::Agent, page_url: &Url, allow_private: bool) -> Option<FetchedIcon> {
    let started = Instant::now();
    let budget = Budget::new();
    let looks_like_image = [".png", ".ico", ".jpg", ".jpeg", ".gif", ".webp"]
        .iter()
        .any(|ext| page_url.path().to_ascii_lowercase().ends_with(ext));

    if looks_like_image {
        if let Some(icon) = fetch_image(agent, page_url, &budget, allow_private) {
            tracing::debug!(url = %page_url, elapsed = ?started.elapsed(), "icon fetched from link");
            return Some(icon);
        }
    }

    if let Ok(root_icon) = page_url.join("/favicon.ico") {
        if let Some(icon) = fetch_image(agent, &root_icon, &budget, allow_private) {
            tracing::debug!(url = %root_icon, elapsed = ?started.elapsed(), "icon fetched from favicon.ico");
            return Some(icon);
        }
    }

    let page = guarded_get(
        agent,
        page_url,
        "text/html,application/xhtml+xml",
        &budget,
        allow_private,
    );

    if let Some((final_url, bytes)) = page {
        // The URL may have answered with an image directly instead of HTML, so
        // reuse those bytes rather than requesting the same address twice.
        if !looks_like_image && bytes.len() >= 32 && bytes.len() <= MAX_ICON {
            if let Some(content_type) = sniff_image(&bytes) {
                return Some(FetchedIcon {
                    content_type,
                    bytes,
                });
            }
        }
        let html = String::from_utf8_lossy(&bytes);
        for candidate in icon_candidates(&html) {
            let Ok(icon_url) = final_url.join(&candidate) else {
                continue;
            };
            if !matches!(icon_url.scheme(), "http" | "https") {
                continue;
            }
            if let Some(icon) = fetch_image(agent, &icon_url, &budget, allow_private) {
                tracing::debug!(url = %icon_url, elapsed = ?started.elapsed(), "icon fetched from link rel");
                return Some(icon);
            }
        }
    } else if !looks_like_image {
        // Nothing readable came back as HTML; the address may still be an image
        // without a recognised extension.
        if let Some(icon) = fetch_image(agent, page_url, &budget, allow_private) {
            return Some(icon);
        }
    }

    None
}

fn fetch_image(
    agent: &ureq::Agent,
    url: &Url,
    budget: &Budget,
    allow_private: bool,
) -> Option<FetchedIcon> {
    let (_final_url, bytes) = guarded_get(
        agent,
        url,
        "image/png,image/x-icon,image/vnd.microsoft.icon,image/jpeg,image/webp,image/*;q=0.8",
        budget,
        allow_private,
    )?;
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
    match result {
        Ok(Some(hash)) => {
            tracing::debug!(link_id, hash, "icon fetched in background");
        }
        Ok(None) => {}
        Err(error) => {
            tracing::debug!(link_id, error = %error, "icon refresh failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(raw: &str) -> IpAddr {
        raw.parse::<IpAddr>().expect("test address")
    }

    #[test]
    fn loopback_link_local_and_metadata_are_always_refused() {
        for raw in [
            "127.0.0.1",
            "127.5.6.7",
            "0.0.0.0",
            "169.254.169.254",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "fe80::1234",
            "ff02::1",
        ] {
            let address = ip(raw);
            assert!(
                !address_allowed(address, true) && !address_allowed(address, false),
                "{raw} must never be fetched"
            );
        }
    }

    #[test]
    fn site_local_follows_the_switch_only() {
        for raw in ["10.1.2.3", "172.16.5.6", "192.168.10.10", "100.64.0.1", "fd00::1"] {
            let address = ip(raw);
            assert!(address_allowed(address, true), "{raw} allowed when enabled");
            assert!(!address_allowed(address, false), "{raw} refused when disabled");
        }
    }

    #[test]
    fn public_addresses_pass() {
        for raw in ["8.8.8.8", "93.184.216.34", "2606:2800:220:1:248:1893:25c8:1946"] {
            assert!(address_allowed(ip(raw), false), "{raw} is public");
            assert!(address_allowed(ip(raw), true), "{raw} is public");
        }
    }

    #[test]
    fn url_policy_rejects_non_http_and_credentials() {
        // These all short-circuit before any DNS lookup happens.
        assert!(!url_allowed(&Url::parse("file:///etc/passwd").unwrap(), true));
        assert!(!url_allowed(&Url::parse("ftp://10.0.0.1/logo.png").unwrap(), true));
        assert!(!url_allowed(
            &Url::parse("http://user:pass@8.8.8.8/").unwrap(),
            true
        ));
        assert!(!url_allowed(
            &Url::parse("https://admin@169.254.169.254/").unwrap(),
            true
        ));
    }

    #[test]
    fn url_policy_refuses_internal_literal_hosts() {
        assert!(!url_allowed(
            &Url::parse("http://169.254.169.254/latest/user-data").unwrap(),
            true
        ));
        assert!(!url_allowed(&Url::parse("http://[::1]:8686/").unwrap(), true));
        assert!(url_allowed(&Url::parse("http://10.0.0.7/logo.png").unwrap(), true));
        assert!(!url_allowed(&Url::parse("http://10.0.0.7/logo.png").unwrap(), false));
    }
}
