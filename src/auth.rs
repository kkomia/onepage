use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration as StdDuration, Instant};

use anyhow::{Context, Result};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::http::HeaderMap;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::Utc;
use cookie::time::Duration as CookieDuration;
use rusqlite::{params, OptionalExtension};
use tokio::task::spawn_blocking;

use crate::config::Config;
use crate::db::Database;
use crate::models::{SessionUser, User};
use crate::util::{now_unix, random_hex, sha256_hex};

pub const SESSION_COOKIE: &str = "onepage_session";
const THROTTLE_WINDOW: StdDuration = StdDuration::from_secs(10 * 60);
const ACCOUNT_LIMIT: usize = 8;
// One address gets a larger budget than one account, so that trying many
// usernames from the same client is still bounded.
const ADDRESS_LIMIT: usize = 20;

pub async fn hash_password(password: &str) -> Result<String> {
    let password = password.to_string();
    spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|error| anyhow::anyhow!("密码哈希失败: {error}"))
    })
    .await
    .context("密码哈希任务异常")?
}

pub async fn verify_password(hash: &str, password: &str) -> bool {
    let hash = hash.to_string();
    let password = password.to_string();
    spawn_blocking(move || {
        let Ok(parsed) = PasswordHash::new(&hash) else {
            return false;
        };
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    })
    .await
    .unwrap_or(false)
}

pub async fn create_session(
    db: &Database,
    user_id: i64,
    session_days: i64,
    ip: String,
    user_agent: String,
) -> Result<(String, SessionUser)> {
    let token = random_hex(32);
    let token_hash = sha256_hex(&token);
    let csrf = random_hex(24);
    let now = now_unix();
    let expires = now + session_days * 86_400;

    let session_hash = token_hash.clone();
    let session_csrf = csrf.clone();
    db.call(move |conn| {
        conn.execute("DELETE FROM sessions WHERE expires_at <= ?1", params![now])?;
        conn.execute(
            "INSERT INTO sessions (token_hash, user_id, csrf, created_at, expires_at, last_seen, ip, user_agent)
             VALUES (?1, ?2, ?3, ?4, ?5, ?4, ?6, ?7)",
            params![session_hash, user_id, session_csrf, now, expires, ip, user_agent],
        )?;
        Ok(())
    })
    .await?;

    let user = user_by_id(db, user_id)
        .await?
        .context("session user missing")?;
    Ok((
        token,
        SessionUser {
            user,
            csrf,
            token_hash,
        },
    ))
}

pub async fn session_from_jar(db: &Database, jar: &CookieJar) -> Option<SessionUser> {
    let token = jar.get(SESSION_COOKIE)?.value().to_string();
    let token_hash = sha256_hex(&token);
    let now = now_unix();

    db.call(move |conn| {
        let result = conn
            .query_row(
                "SELECT u.id, u.username, u.display_name, u.is_super_admin, u.is_active,
                        u.must_change_password, u.last_login_at, s.csrf, s.token_hash
                 FROM sessions s
                 JOIN users u ON u.id = s.user_id
                 WHERE s.token_hash = ?1 AND s.expires_at > ?2 AND u.is_active = 1",
                params![token_hash, now],
                |row| {
                    Ok(SessionUser {
                        user: User {
                            id: row.get(0)?,
                            username: row.get(1)?,
                            display_name: row.get(2)?,
                            is_super_admin: row.get(3)?,
                            is_active: row.get(4)?,
                            must_change_password: row.get(5)?,
                            last_login_at: row.get(6)?,
                        },
                        csrf: row.get(7)?,
                        token_hash: row.get(8)?,
                    })
                },
            )
            .optional()?;
        Ok(result)
    })
    .await
    .ok()
    .flatten()
}

pub async fn destroy_session(db: &Database, token_hash: &str) -> Result<()> {
    let token_hash = token_hash.to_string();
    db.call(move |conn| {
        conn.execute(
            "DELETE FROM sessions WHERE token_hash = ?1",
            params![token_hash],
        )?;
        Ok(())
    })
    .await
}

pub async fn user_by_id(db: &Database, user_id: i64) -> Result<Option<User>> {
    db.call(move |conn| {
        let user = conn
            .query_row(
                "SELECT id, username, display_name, is_super_admin, is_active,
                        must_change_password, last_login_at
                 FROM users WHERE id = ?1",
                params![user_id],
                map_user,
            )
            .optional()?;
        Ok(user)
    })
    .await
}

pub async fn user_with_password(db: &Database, username: &str) -> Result<Option<(User, String)>> {
    let username = username.trim().to_string();
    db.call(move |conn| {
        let user = conn
            .query_row(
                "SELECT id, username, display_name, is_super_admin, is_active,
                        must_change_password, last_login_at, password_hash
                 FROM users WHERE username = ?1",
                params![username],
                |row| {
                    Ok((
                        User {
                            id: row.get(0)?,
                            username: row.get(1)?,
                            display_name: row.get(2)?,
                            is_super_admin: row.get(3)?,
                            is_active: row.get(4)?,
                            must_change_password: row.get(5)?,
                            last_login_at: row.get(6)?,
                        },
                        row.get(7)?,
                    ))
                },
            )
            .optional()?;
        Ok(user)
    })
    .await
}

pub async fn mark_login(db: &Database, user_id: i64) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    db.call(move |conn| {
        conn.execute(
            "UPDATE users SET last_login_at = ?1 WHERE id = ?2",
            params![now, user_id],
        )?;
        Ok(())
    })
    .await
}

pub async fn update_password(
    db: &Database,
    user_id: i64,
    password_hash: String,
    keep_token_hash: Option<String>,
) -> Result<()> {
    db.call(move |conn| {
        conn.execute(
            "UPDATE users SET password_hash = ?1, must_change_password = 0 WHERE id = ?2",
            params![password_hash, user_id],
        )?;
        if let Some(keep) = keep_token_hash {
            conn.execute(
                "DELETE FROM sessions WHERE user_id = ?1 AND token_hash <> ?2",
                params![user_id, keep],
            )?;
        } else {
            conn.execute(
                "DELETE FROM sessions WHERE user_id = ?1",
                params![user_id],
            )?;
        }
        Ok(())
    })
    .await
}

pub async fn reset_password(db: &Database, user_id: i64, password_hash: String) -> Result<()> {
    db.call(move |conn| {
        conn.execute(
            "UPDATE users SET password_hash = ?1, must_change_password = 1 WHERE id = ?2",
            params![password_hash, user_id],
        )?;
        conn.execute(
            "DELETE FROM sessions WHERE user_id = ?1",
            params![user_id],
        )?;
        Ok(())
    })
    .await
}

pub fn verify_csrf(session: &SessionUser, token: &str) -> bool {
    let expected = session.csrf.as_bytes();
    let given = token.as_bytes();
    if expected.len() != given.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in expected.iter().zip(given) {
        diff |= a ^ b;
    }
    diff == 0
}

pub fn verify_api_csrf(session: &SessionUser, headers: &HeaderMap) -> bool {
    headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .map(|token| verify_csrf(session, token))
        .unwrap_or(false)
}

pub fn session_cookie(token: &str, config: &Config) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, token.to_string()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(config.secure_cookies)
        .max_age(CookieDuration::days(config.session_days))
        .build()
}

pub fn clear_session_cookie() -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(CookieDuration::ZERO)
        .build()
}

fn map_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        username: row.get(1)?,
        display_name: row.get(2)?,
        is_super_admin: row.get(3)?,
        is_active: row.get(4)?,
        must_change_password: row.get(5)?,
        last_login_at: row.get(6)?,
    })
}

#[derive(Default)]
pub struct LoginThrottle {
    attempts: Mutex<HashMap<String, Vec<Instant>>>,
}

impl LoginThrottle {
    pub fn blocked_for(&self, account_key: &str, address_key: &str) -> Option<u64> {
        let attempts = self.attempts.lock().unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        [
            (account_key, ACCOUNT_LIMIT),
            (address_key, ADDRESS_LIMIT),
        ]
        .into_iter()
        .filter_map(|(key, limit)| {
            // Reading must not create entries, or unauthenticated traffic would
            // grow the map on its own.
            let list = attempts.get(key)?;
            let recent = list
                .iter()
                .filter(|instant| now.duration_since(**instant) < THROTTLE_WINDOW)
                .copied()
                .collect::<Vec<_>>();
            if recent.len() < limit {
                return None;
            }
            let oldest = recent.first().copied().unwrap_or(now);
            let left = THROTTLE_WINDOW.saturating_sub(now.duration_since(oldest));
            Some(left.as_secs().max(1))
        })
        .min()
    }

    pub fn record_failure(&self, account_key: &str, address_key: &str) {
        let mut attempts = self.attempts.lock().unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        for key in [account_key, address_key] {
            let list = attempts.entry(key.to_string()).or_default();
            list.retain(|instant| now.duration_since(*instant) < THROTTLE_WINDOW);
            list.push(now);
        }
        if attempts.len() > 5_000 {
            attempts.retain(|_, list| {
                list.retain(|instant| now.duration_since(*instant) < THROTTLE_WINDOW);
                !list.is_empty()
            });
        }
    }

    pub fn clear(&self, account_key: &str, address_key: &str) {
        let mut attempts = self.attempts.lock().unwrap_or_else(|error| error.into_inner());
        attempts.remove(account_key);
        attempts.remove(address_key);
    }
}
