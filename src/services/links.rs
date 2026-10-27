use anyhow::{bail, Result};
use rusqlite::{params, OptionalExtension};

use crate::db::Database;
use crate::models::{Category, LinkRecord};
use crate::util::{now_rfc3339, pinyin_index};

pub async fn categories_for(
    db: &Database,
    space_id: i64,
    owner_id: Option<i64>,
) -> Result<Vec<Category>> {
    db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, space_id, owner_id, name
             FROM categories
             WHERE space_id = ?1 AND owner_id IS ?2
             ORDER BY sort_order, id",
        )?;
        let rows = stmt
            .query_map(params![space_id, owner_id], map_category)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
}

pub async fn links_for(
    db: &Database,
    space_id: i64,
    owner_id: Option<i64>,
) -> Result<Vec<LinkRecord>> {
    db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT l.id, l.space_id, l.owner_id, l.category_id, l.title, l.url,
                    l.description, i.hash, l.click_count, l.created_at, l.updated_at
             FROM links l
             LEFT JOIN icons i ON i.id = l.icon_id
             WHERE l.space_id = ?1 AND l.owner_id IS ?2
             ORDER BY l.sort_order, l.id",
        )?;
        let rows = stmt
            .query_map(params![space_id, owner_id], map_link)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
}

pub async fn link_by_id(db: &Database, link_id: i64) -> Result<Option<LinkRecord>> {
    db.call(move |conn| {
        let link = conn
            .query_row(
                "SELECT l.id, l.space_id, l.owner_id, l.category_id, l.title, l.url,
                        l.description, i.hash, l.click_count, l.created_at, l.updated_at
                 FROM links l
                 LEFT JOIN icons i ON i.id = l.icon_id
                 WHERE l.id = ?1",
                params![link_id],
                map_link,
            )
            .optional()?;
        Ok(link)
    })
    .await
}

pub struct NewLink<'a> {
    pub space_id: i64,
    pub owner_id: Option<i64>,
    pub category_id: Option<i64>,
    pub title: &'a str,
    pub url: &'a str,
    pub description: &'a str,
    pub created_by: i64,
}

pub async fn create_link(db: &Database, input: NewLink<'_>) -> Result<LinkRecord> {
    let now = now_rfc3339();
    let NewLink {
        space_id,
        owner_id,
        category_id,
        title,
        url,
        description,
        created_by,
    } = input;
    let title = title.to_string();
    let url = url.to_string();
    let description = description.to_string();

    db.call(move |conn| {
        if let Some(category_id) = category_id {
            ensure_category(conn, category_id, space_id, owner_id)?;
        }
        let sort_order: i64 = conn.query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 10 FROM links WHERE space_id = ?1 AND owner_id IS ?2",
            params![space_id, owner_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO links
                (space_id, owner_id, category_id, title, url, description, sort_order, created_by, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                space_id,
                owner_id,
                category_id,
                title,
                url,
                description,
                sort_order,
                created_by,
                now
            ],
        )?;
        let id = conn.last_insert_rowid();
        Ok(LinkRecord {
            id,
            space_id,
            owner_id,
            category_id,
            title,
            url,
            description,
            icon_hash: None,
            click_count: 0,
            created_at: now.clone(),
            updated_at: now,
        })
    })
    .await
}

pub struct LinkUpdate<'a> {
    pub category_id: Option<i64>,
    pub title: &'a str,
    pub url: &'a str,
    pub description: &'a str,
}

pub async fn update_link(
    db: &Database,
    link: &LinkRecord,
    input: LinkUpdate<'_>,
) -> Result<LinkRecord> {
    let now = now_rfc3339();
    let link_id = link.id;
    let space_id = link.space_id;
    let owner_id = link.owner_id;
    let old_url = link.url.clone();
    let LinkUpdate {
        category_id,
        title,
        url,
        description,
    } = input;
    let title = title.to_string();
    let url = url.to_string();
    let description = description.to_string();

    db.call(move |conn| {
        if let Some(category_id) = category_id {
            ensure_category(conn, category_id, space_id, owner_id)?;
        }
        let url_changed = old_url != url;
        if url_changed {
            conn.execute(
                "UPDATE links SET category_id = ?1, title = ?2, url = ?3, description = ?4,
                                  icon_id = NULL, updated_at = ?5
                 WHERE id = ?6",
                params![category_id, title, url, description, now, link_id],
            )?;
        } else {
            conn.execute(
                "UPDATE links SET category_id = ?1, title = ?2, url = ?3, description = ?4,
                                  updated_at = ?5
                 WHERE id = ?6",
                params![category_id, title, url, description, now, link_id],
            )?;
        }
        let updated = conn
            .query_row(
                "SELECT l.id, l.space_id, l.owner_id, l.category_id, l.title, l.url,
                        l.description, i.hash, l.click_count, l.created_at, l.updated_at
                 FROM links l
                 LEFT JOIN icons i ON i.id = l.icon_id
                 WHERE l.id = ?1",
                params![link_id],
                map_link,
            )
            .optional()?;
        updated.ok_or_else(|| anyhow::anyhow!("链接不存在"))
    })
    .await
}

pub async fn delete_link(db: &Database, link_id: i64) -> Result<bool> {
    db.call(move |conn| {
        let changed = conn.execute("DELETE FROM links WHERE id = ?1", params![link_id])?;
        Ok(changed > 0)
    })
    .await
}

pub async fn record_click(db: &Database, link_id: i64, user_id: i64) -> Result<()> {
    let now = now_rfc3339();
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO link_clicks (link_id, user_id, click_count, last_clicked_at)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(link_id, user_id) DO UPDATE SET
                 click_count = click_count + 1,
                 last_clicked_at = excluded.last_clicked_at",
            params![link_id, user_id, now],
        )?;
        conn.execute(
            "UPDATE links SET click_count = click_count + 1, last_clicked_at = ?1 WHERE id = ?2",
            params![now, link_id],
        )?;
        Ok(())
    })
    .await
}

pub async fn create_category(
    db: &Database,
    space_id: i64,
    owner_id: Option<i64>,
    name: &str,
) -> Result<Category> {
    let name = name.to_string();
    let now = now_rfc3339();
    db.call(move |conn| {
        let duplicate: Option<i64> = conn
            .query_row(
                "SELECT id FROM categories
                 WHERE space_id = ?1 AND owner_id IS ?2 AND lower(name) = lower(?3)",
                params![space_id, owner_id, name],
                |row| row.get(0),
            )
            .optional()?;
        if duplicate.is_some() {
            bail!("这个分类已经存在。");
        }
        let sort_order: i64 = conn.query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 10 FROM categories WHERE space_id = ?1 AND owner_id IS ?2",
            params![space_id, owner_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO categories (space_id, owner_id, name, sort_order, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![space_id, owner_id, name, sort_order, now],
        )?;
        Ok(Category {
            id: conn.last_insert_rowid(),
            space_id,
            owner_id,
            name,
        })
    })
    .await
}

pub async fn category_by_id(db: &Database, category_id: i64) -> Result<Option<Category>> {
    db.call(move |conn| {
        let category = conn
            .query_row(
                "SELECT id, space_id, owner_id, name FROM categories WHERE id = ?1",
                params![category_id],
                map_category,
            )
            .optional()?;
        Ok(category)
    })
    .await
}

pub async fn rename_category(db: &Database, category_id: i64, name: &str) -> Result<()> {
    let name = name.to_string();
    db.call(move |conn| {
        let category: Option<(i64, Option<i64>)> = conn
            .query_row(
                "SELECT space_id, owner_id FROM categories WHERE id = ?1",
                params![category_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((space_id, owner_id)) = category else {
            bail!("分类不存在。");
        };
        let duplicate: Option<i64> = conn
            .query_row(
                "SELECT id FROM categories
                 WHERE space_id = ?1 AND owner_id IS ?2 AND lower(name) = lower(?3) AND id <> ?4",
                params![space_id, owner_id, name, category_id],
                |row| row.get(0),
            )
            .optional()?;
        if duplicate.is_some() {
            bail!("这个分类已经存在。");
        }
        conn.execute(
            "UPDATE categories SET name = ?1 WHERE id = ?2",
            params![name, category_id],
        )?;
        Ok(())
    })
    .await
}

pub async fn delete_category(db: &Database, category_id: i64) -> Result<bool> {
    db.call(move |conn| {
        let changed = conn.execute("DELETE FROM categories WHERE id = ?1", params![category_id])?;
        Ok(changed > 0)
    })
    .await
}

pub async fn move_category(
    db: &Database,
    category_id: i64,
    direction: &str,
) -> Result<()> {
    let direction = direction.to_string();
    db.call(move |conn| {
        let tx = conn.transaction()?;
        let (space_id, owner_id): (i64, Option<i64>) = tx
            .query_row(
                "SELECT space_id, owner_id FROM categories WHERE id = ?1",
                params![category_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("分类不存在。"))?;
        let mut stmt = tx.prepare(
            "SELECT id FROM categories WHERE space_id = ?1 AND owner_id IS ?2 ORDER BY sort_order, id",
        )?;
        let mut ids = stmt
            .query_map(params![space_id, owner_id], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        let Some(position) = ids.iter().position(|id| *id == category_id) else {
            bail!("分类不存在。");
        };
        let target = if direction == "up" {
            position.checked_sub(1)
        } else {
            Some(position + 1).filter(|index| *index < ids.len())
        };
        let Some(target) = target else {
            return Ok(());
        };
        ids.swap(position, target);
        for (index, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE categories SET sort_order = ?1 WHERE id = ?2",
                params![((index as i64) + 1) * 10, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    })
    .await
}

pub async fn move_link(db: &Database, link_id: i64, direction: &str) -> Result<()> {
    let direction = direction.to_string();
    db.call(move |conn| {
        let tx = conn.transaction()?;
        let (space_id, owner_id, category_id): (i64, Option<i64>, Option<i64>) = tx
            .query_row(
                "SELECT space_id, owner_id, category_id FROM links WHERE id = ?1",
                params![link_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("链接不存在。"))?;
        let mut stmt = tx.prepare(
            "SELECT id FROM links
             WHERE space_id = ?1 AND owner_id IS ?2 AND category_id IS ?3
             ORDER BY sort_order, id",
        )?;
        let mut ids = stmt
            .query_map(params![space_id, owner_id, category_id], |row| {
                row.get::<_, i64>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        let Some(position) = ids.iter().position(|id| *id == link_id) else {
            bail!("链接不存在。");
        };
        let target = if direction == "up" {
            position.checked_sub(1)
        } else {
            Some(position + 1).filter(|index| *index < ids.len())
        };
        let Some(target) = target else {
            return Ok(());
        };
        ids.swap(position, target);
        for (index, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE links SET sort_order = ?1 WHERE id = ?2",
                params![((index as i64) + 1) * 10, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    })
    .await
}

#[derive(Debug, Clone)]
pub struct SearchableLink {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub description: String,
    pub category_id: Option<i64>,
    pub public: bool,
    pub icon_hash: Option<String>,
    pub click_count: i64,
    pub created_at: String,
    pub py: String,
    pub pyi: String,
}

pub async fn searchable_links(
    db: &Database,
    space_id: i64,
    user_id: Option<i64>,
) -> Result<Vec<SearchableLink>> {
    db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT l.id, l.title, l.url, l.description, l.category_id, c.name,
                    l.owner_id IS NULL, i.hash, COALESCE(lc.click_count, 0), l.created_at
             FROM links l
             LEFT JOIN categories c ON c.id = l.category_id
             LEFT JOIN icons i ON i.id = l.icon_id
             LEFT JOIN link_clicks lc ON lc.link_id = l.id AND lc.user_id = ?2
             WHERE l.space_id = ?1 AND (l.owner_id IS NULL OR l.owner_id IS ?2)
             ORDER BY l.owner_id IS NOT NULL, l.category_id,
                      COALESCE(lc.click_count, 0) DESC, l.sort_order, l.id",
        )?;
        let rows = stmt
            .query_map(params![space_id, user_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, bool>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, String>(9)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    title,
                    url,
                    description,
                    category_id,
                    category_name,
                    public,
                    icon_hash,
                    click_count,
                    created_at,
                )| {
                    let (py, pyi) = pinyin_index(&format!(
                        "{title} {}",
                        category_name.as_deref().unwrap_or("")
                    ));
                    SearchableLink {
                        id,
                        title,
                        url,
                        description,
                        category_id,
                        public,
                        icon_hash,
                        click_count,
                        created_at,
                        py,
                        pyi,
                    }
                },
            )
            .collect())
    })
    .await
}

fn ensure_category(
    conn: &rusqlite::Connection,
    category_id: i64,
    space_id: i64,
    owner_id: Option<i64>,
) -> Result<()> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT id FROM categories WHERE id = ?1 AND space_id = ?2 AND owner_id IS ?3",
            params![category_id, space_id, owner_id],
            |row| row.get(0),
        )
        .optional()?;
    if found.is_none() {
        bail!("分类不存在或不属于当前空间。");
    }
    Ok(())
}

fn map_category(row: &rusqlite::Row<'_>) -> rusqlite::Result<Category> {
    Ok(Category {
        id: row.get(0)?,
        space_id: row.get(1)?,
        owner_id: row.get(2)?,
        name: row.get(3)?,
    })
}

fn map_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<LinkRecord> {
    Ok(LinkRecord {
        id: row.get(0)?,
        space_id: row.get(1)?,
        owner_id: row.get(2)?,
        category_id: row.get(3)?,
        title: row.get(4)?,
        url: row.get(5)?,
        description: row.get(6)?,
        icon_hash: row.get(7)?,
        click_count: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}
