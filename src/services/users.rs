use anyhow::{bail, Result};
use rusqlite::{params, OptionalExtension};

use crate::db::Database;
use crate::models::User;
use crate::util::now_rfc3339;

#[derive(Debug, Clone)]
pub struct UserRow {
    pub user: User,
    pub created_at: String,
    pub spaces: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct UserChoice {
    pub id: i64,
    pub username: String,
    pub display_name: String,
}

pub async fn list_users(db: &Database) -> Result<Vec<UserRow>> {
    db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, username, display_name, is_super_admin, is_active,
                    must_change_password, last_login_at, created_at
             FROM users
             ORDER BY is_active DESC, is_super_admin DESC, display_name COLLATE NOCASE",
        )?;
        let users = stmt
            .query_map([], |row| {
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
                    row.get::<_, String>(7)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut result = Vec::with_capacity(users.len());
        for (user, created_at) in users {
            let mut spaces_stmt = conn.prepare(
                "SELECT s.name FROM memberships m
                 JOIN spaces s ON s.id = m.space_id
                 WHERE m.user_id = ?1
                 ORDER BY s.name COLLATE NOCASE",
            )?;
            let spaces = spaces_stmt
                .query_map(params![user.id], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            result.push(UserRow {
                user,
                created_at,
                spaces,
            });
        }
        Ok(result)
    })
    .await
}

pub async fn find_by_username(db: &Database, username: &str) -> Result<Option<User>> {
    let username = username.trim().to_string();
    db.call(move |conn| {
        let user = conn
            .query_row(
                "SELECT id, username, display_name, is_super_admin, is_active,
                        must_change_password, last_login_at
                 FROM users WHERE username = ?1",
                params![username],
                |row| {
                    Ok(User {
                        id: row.get(0)?,
                        username: row.get(1)?,
                        display_name: row.get(2)?,
                        is_super_admin: row.get(3)?,
                        is_active: row.get(4)?,
                        must_change_password: row.get(5)?,
                        last_login_at: row.get(6)?,
                    })
                },
            )
            .optional()?;
        Ok(user)
    })
    .await
}

pub async fn user_choices(db: &Database) -> Result<Vec<UserChoice>> {
    db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, username, display_name FROM users
             WHERE is_active = 1
             ORDER BY display_name COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(UserChoice {
                    id: row.get(0)?,
                    username: row.get(1)?,
                    display_name: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
}

pub async fn create_user(
    db: &Database,
    username: &str,
    display_name: &str,
    password_hash: &str,
    must_change_password: bool,
) -> Result<User> {
    let username = username.to_string();
    let display_name = display_name.to_string();
    let password_hash = password_hash.to_string();
    let created_at = now_rfc3339();
    db.call(move |conn| {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT id FROM users WHERE username = ?1",
                params![username],
                |row| row.get(0),
            )
            .ok();
        if exists.is_some() {
            bail!("这个账号已经存在。");
        }
        conn.execute(
            "INSERT INTO users
                (username, display_name, password_hash, is_super_admin, is_active,
                 must_change_password, created_at)
             VALUES (?1, ?2, ?3, 0, 1, ?4, ?5)",
            params![
                username,
                display_name,
                password_hash,
                must_change_password,
                created_at
            ],
        )?;
        Ok(User {
            id: conn.last_insert_rowid(),
            username,
            display_name,
            is_super_admin: false,
            is_active: true,
            must_change_password,
            last_login_at: None,
        })
    })
    .await
}

pub async fn set_active(db: &Database, user_id: i64, active: bool) -> Result<()> {
    db.call(move |conn| {
        conn.execute(
            "UPDATE users SET is_active = ?1 WHERE id = ?2",
            params![active, user_id],
        )?;
        if !active {
            conn.execute("DELETE FROM sessions WHERE user_id = ?1", params![user_id])?;
        }
        Ok(())
    })
    .await
}

pub async fn active_super_admin_count(db: &Database) -> Result<i64> {
    db.call(|conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM users WHERE is_super_admin = 1 AND is_active = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    })
    .await
}
