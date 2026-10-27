use chrono::{DateTime, Utc};
use pinyin::ToPinyin;
use rand::{rngs::OsRng, RngCore};
use sha2::{Digest, Sha256};
use url::Url;

pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

pub fn now_unix() -> i64 {
    Utc::now().timestamp()
}

pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

pub fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn sha256_bytes_short(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hex::encode(hasher.finalize());
    digest[..16].to_string()
}

pub fn validate_username(raw: &str) -> Result<String, String> {
    let username = raw.trim().to_ascii_lowercase();
    let mut chars = username.chars();
    let valid_first = chars
        .next()
        .map(|c| c.is_ascii_alphanumeric())
        .unwrap_or(false);
    let valid_rest = username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if username.len() < 3 || username.len() > 32 || !valid_first || !valid_rest {
        return Err("账号需为 3–32 位小写字母、数字或 . _ -，并以字母或数字开头".into());
    }
    Ok(username)
}

pub fn validate_display_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > 40 {
        return Err("姓名需为 1–40 个字符".into());
    }
    Ok(name.to_string())
}

pub fn validate_slug(raw: &str) -> Result<String, String> {
    let slug = raw.trim().to_ascii_lowercase();
    let bytes = slug.as_bytes();
    let valid = (2..=40).contains(&bytes.len())
        && bytes
            .first()
            .map(|b| b.is_ascii_alphanumeric())
            .unwrap_or(false)
        && bytes
            .last()
            .map(|b| b.is_ascii_alphanumeric())
            .unwrap_or(false)
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-');
    if !valid {
        return Err("空间标识需为 2–40 位小写字母、数字或短横线".into());
    }
    Ok(slug)
}

pub fn validate_space_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > 40 {
        return Err("空间名称需为 1–40 个字符".into());
    }
    Ok(name.to_string())
}

pub fn validate_site_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > 30 {
        return Err("站点名称需为 1–30 个字符".into());
    }
    Ok(name.to_string())
}

pub fn validate_category_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > 24 {
        return Err("分类名称需为 1–24 个字符".into());
    }
    Ok(name.to_string())
}

pub fn validate_link_title(raw: &str) -> Result<String, String> {
    let title = raw.trim();
    if title.is_empty() || title.chars().count() > 80 {
        return Err("名称需为 1–80 个字符".into());
    }
    Ok(title.to_string())
}

pub fn validate_description(raw: &str) -> Result<String, String> {
    let text = raw.trim();
    if text.chars().count() > 280 {
        return Err("描述不能超过 280 个字符".into());
    }
    Ok(text.to_string())
}

pub fn validate_password(raw: &str) -> Result<(), String> {
    let count = raw.chars().count();
    if !(8..=128).contains(&count) {
        return Err("密码需为 8–128 个字符".into());
    }
    Ok(())
}

pub fn normalize_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("请填写网址".into());
    }
    if raw.chars().count() > 2048 {
        return Err("网址过长".into());
    }
    let candidate = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let url = Url::parse(&candidate).map_err(|_| "网址格式不正确".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("仅支持 http 或 https 网址".into());
    }
    if url.host_str().is_none() {
        return Err("网址缺少域名".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("网址不能包含账号密码".into());
    }
    Ok(url.to_string())
}

pub fn host_label(raw: &str) -> String {
    Url::parse(raw)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .map(|host| host.strip_prefix("www.").unwrap_or(&host).to_string())
        .unwrap_or_else(|| raw.to_string())
}

pub fn pinyin_index(input: &str) -> (String, String) {
    let mut full = String::with_capacity(input.len() * 2);
    let mut initials = String::with_capacity(input.len());
    // ASCII words contribute their leading letter to the acronym too, so that
    // "gl" finds "GitLab" the same way "sjzl" finds "数据治理".
    let mut at_word_start = true;

    for (ch, pinyin) in input.chars().zip(input.to_pinyin()) {
        match pinyin {
            Some(p) => {
                full.push_str(p.plain());
                initials.push_str(p.first_letter());
                at_word_start = true;
            }
            None => {
                if ch.is_ascii_alphanumeric() {
                    if at_word_start || ch.is_ascii_uppercase() {
                        initials.push(ch.to_ascii_lowercase());
                    }
                    full.push(ch.to_ascii_lowercase());
                    at_word_start = false;
                } else {
                    at_word_start = true;
                }
            }
        }
    }

    (full, initials)
}

pub fn format_date(rfc3339: &str) -> String {
    DateTime::parse_from_rfc3339(rfc3339)
        .map(|date| date.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| rfc3339.chars().take(10).collect())
}

pub fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(value)
            if value.starts_with('/')
                && !value.starts_with("//")
                && !value.starts_with("/login")
                && !value.starts_with("/setup")
                && !value.starts_with("/logout")
                && !value.contains('\\')
                && !value.chars().any(char::is_control) =>
        {
            value.to_string()
        }
        _ => "/".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_normalizes_and_rejects_bad_schemes() {
        assert_eq!(
            normalize_url("git.example.com/team/repo").unwrap(),
            "https://git.example.com/team/repo"
        );
        assert!(normalize_url("javascript:alert(1)").is_err());
        assert!(normalize_url("ftp://example.com").is_err());
    }

    #[test]
    fn slug_rules() {
        assert!(validate_slug("data-gov").is_ok());
        assert!(validate_slug("-bad").is_err());
        assert!(validate_slug("中文").is_err());
    }

    #[test]
    fn pinyin_handles_mixed_text() {
        let (full, initials) = pinyin_index("数据治理 GitLab");
        assert!(full.contains("shuju"));
        assert!(initials.contains("sjzl"));
        assert!(initials.contains("gl"), "english acronyms index too: {initials}");
    }

    #[test]
    fn pinyin_keeps_chinese_only_acronym() {
        let (full, initials) = pinyin_index("数据治理");
        assert_eq!(full, "shujuzhili");
        assert_eq!(initials, "sjzl");
    }

    #[test]
    fn next_is_local_only() {
        assert_eq!(safe_next(Some("/s/team")), "/s/team");
        assert_eq!(safe_next(Some("//evil.test")), "/");
        assert_eq!(safe_next(Some("https://evil.test")), "/");
    }
}
