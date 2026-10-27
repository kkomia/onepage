use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use crate::db::Database;

pub async fn get_setting(db: &Database, key: &str) -> Result<Option<String>> {
    let key = key.to_string();
    db.call(move |conn| {
        let value = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value)
    })
    .await
}

pub async fn set_setting(db: &Database, key: &str, value: &str) -> Result<()> {
    let key = key.to_string();
    let value = value.to_string();
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    })
    .await
}

pub async fn site_name(db: &Database) -> Result<String> {
    Ok(get_setting(db, "site_name")
        .await?
        .unwrap_or_else(|| "OnePage".to_string()))
}

pub async fn setup_complete(db: &Database) -> Result<bool> {
    db.call(|conn| {
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
        Ok(count > 0)
    })
    .await
}
