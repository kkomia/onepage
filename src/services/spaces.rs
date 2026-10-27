use anyhow::{anyhow, bail, Result};
use rusqlite::{params, OptionalExtension};

use crate::db::Database;
use crate::models::{Role, Space, SpaceAccess, SpaceMembership, User};
use crate::util::now_rfc3339;

#[derive(Debug, Clone)]
pub struct MemberRow {
    pub user_id: i64,
    pub username: String,
    pub display_name: String,
    pub role: Role,
    pub joined_at: String,
    pub last_login_at: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub struct SpaceSummary {
    pub space: Space,
    pub member_count: i64,
    pub public_links: i64,
    pub private_links: i64,
    pub admins: Vec<String>,
}

pub async fn find_space_by_slug(db: &Database, slug: &str) -> Result<Option<Space>> {
    let slug = slug.to_string();
    db.call(move |conn| {
        let space = conn
            .query_row(
                "SELECT id, slug, name, description, created_at FROM spaces WHERE slug = ?1",
                params![slug],
                map_space,
            )
            .optional()?;
        Ok(space)
    })
    .await
}

pub async fn space_access(
    db: &Database,
    user: &User,
    slug: &str,
) -> Result<Option<SpaceAccess>> {
    let Some(space) = find_space_by_slug(db, slug).await? else {
        return Ok(None);
    };
    let user_id = user.id;
    let space_id = space.id;
    let membership_role = db
        .call(move |conn| {
            let role: Option<String> = conn
                .query_row(
                    "SELECT role FROM memberships WHERE space_id = ?1 AND user_id = ?2",
                    params![space_id, user_id],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(role)
        })
        .await?;

    let member = membership_role.is_some();
    let role = membership_role
        .as_deref()
        .map(Role::from_db)
        .or(if user.is_super_admin {
            Some(Role::Admin)
        } else {
            None
        });

    if role.is_none() {
        return Ok(None);
    }

    Ok(Some(SpaceAccess {
        space,
        role,
        member,
    }))
}

pub async fn memberships_for_user(db: &Database, user_id: i64) -> Result<Vec<SpaceMembership>> {
    db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT s.id, s.slug, s.name, s.description, s.created_at, m.role
             FROM memberships m
             JOIN spaces s ON s.id = m.space_id
             WHERE m.user_id = ?1
             ORDER BY s.name COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map(params![user_id], |row| {
                Ok(SpaceMembership {
                    space: map_space(row)?,
                    role: Some(Role::from_db(&row.get::<_, String>(5)?)),
                    member: true,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
}

pub async fn all_spaces_for_switcher(db: &Database, user: &User) -> Result<Vec<SpaceMembership>> {
    if !user.is_super_admin {
        return memberships_for_user(db, user.id).await;
    }
    let user_id = user.id;
    db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT s.id, s.slug, s.name, s.description, s.created_at, m.role
             FROM spaces s
             LEFT JOIN memberships m ON m.space_id = s.id AND m.user_id = ?1
             ORDER BY s.name COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map(params![user_id], |row| {
                let role: Option<String> = row.get(5)?;
                let member = role.is_some();
                Ok(SpaceMembership {
                    space: map_space(row)?,
                    role: role
                        .as_deref()
                        .map(Role::from_db)
                        .or(Some(Role::Admin)),
                    member,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
}

pub async fn members_for_space(db: &Database, space_id: i64) -> Result<Vec<MemberRow>> {
    db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT u.id, u.username, u.display_name, m.role, m.created_at, u.last_login_at, u.is_active
             FROM memberships m
             JOIN users u ON u.id = m.user_id
             WHERE m.space_id = ?1
             ORDER BY CASE m.role WHEN 'admin' THEN 0 ELSE 1 END, u.display_name COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map(params![space_id], |row| {
                Ok(MemberRow {
                    user_id: row.get(0)?,
                    username: row.get(1)?,
                    display_name: row.get(2)?,
                    role: Role::from_db(&row.get::<_, String>(3)?),
                    joined_at: row.get(4)?,
                    last_login_at: row.get(5)?,
                    is_active: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
}

pub async fn add_member(
    db: &Database,
    space_id: i64,
    username: &str,
    role: Role,
) -> Result<MemberRow> {
    let username = username.trim().to_lowercase();
    let role_str = role.as_str().to_string();
    let now = now_rfc3339();
    db.call(move |conn| {
        let user: Option<(i64, String, String, bool, Option<String>)> = conn
            .query_row(
                "SELECT id, username, display_name, is_active, last_login_at FROM users WHERE username = ?1",
                params![username],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((user_id, username, display_name, is_active, last_login_at)) = user else {
            bail!("找不到这个账号，请先在“用户”里创建。");
        };
        if !is_active {
            bail!("这个账号已停用。");
        }
        let exists: Option<i64> = conn
            .query_row(
                "SELECT id FROM memberships WHERE space_id = ?1 AND user_id = ?2",
                params![space_id, user_id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_some() {
            bail!("这个账号已经是空间成员。");
        }
        conn.execute(
            "INSERT INTO memberships (space_id, user_id, role, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![space_id, user_id, role_str, now],
        )?;
        Ok(MemberRow {
            user_id,
            username,
            display_name,
            role: Role::from_db(&role_str),
            joined_at: now,
            last_login_at,
            is_active,
        })
    })
    .await
}

pub async fn set_member_role(
    db: &Database,
    space_id: i64,
    user_id: i64,
    role: Role,
) -> Result<()> {
    db.call(move |conn| {
        let tx = conn.transaction()?;
        let current: Option<String> = tx
            .query_row(
                "SELECT role FROM memberships WHERE space_id = ?1 AND user_id = ?2",
                params![space_id, user_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(current) = current else {
            bail!("成员不存在。");
        };
        if current == "admin" && role != Role::Admin {
            let admins: i64 = tx.query_row(
                "SELECT COUNT(*) FROM memberships WHERE space_id = ?1 AND role = 'admin'",
                params![space_id],
                |row| row.get(0),
            )?;
            if admins <= 1 {
                bail!("空间至少需要保留一位管理员。");
            }
        }
        tx.execute(
            "UPDATE memberships SET role = ?1 WHERE space_id = ?2 AND user_id = ?3",
            params![role.as_str(), space_id, user_id],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}

pub async fn remove_member(db: &Database, space_id: i64, user_id: i64) -> Result<()> {
    db.call(move |conn| {
        let tx = conn.transaction()?;
        let current: Option<String> = tx
            .query_row(
                "SELECT role FROM memberships WHERE space_id = ?1 AND user_id = ?2",
                params![space_id, user_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(current) = current else {
            bail!("成员不存在。");
        };
        if current == "admin" {
            let admins: i64 = tx.query_row(
                "SELECT COUNT(*) FROM memberships WHERE space_id = ?1 AND role = 'admin'",
                params![space_id],
                |row| row.get(0),
            )?;
            if admins <= 1 {
                bail!("空间至少需要保留一位管理员。");
            }
        }
        tx.execute(
            "DELETE FROM memberships WHERE space_id = ?1 AND user_id = ?2",
            params![space_id, user_id],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}

pub async fn create_space(
    db: &Database,
    name: &str,
    slug: &str,
    description: &str,
    admin_user_id: i64,
    created_by: i64,
) -> Result<Space> {
    let name = name.to_string();
    let slug = slug.to_string();
    let description = description.to_string();
    let now = now_rfc3339();
    db.call(move |conn| {
        let tx = conn.transaction()?;
        let user_active: Option<bool> = tx
            .query_row(
                "SELECT is_active FROM users WHERE id = ?1",
                params![admin_user_id],
                |row| row.get(0),
            )
            .optional()?;
        match user_active {
            Some(true) => {}
            Some(false) => bail!("指定管理员账号已停用。"),
            None => bail!("指定管理员账号不存在。"),
        }
        tx.execute(
            "INSERT INTO spaces (slug, name, description, created_by, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![slug, name, description, created_by, now],
        )
        .map_err(|error| match error {
            rusqlite::Error::SqliteFailure(_, Some(ref message)) if message.contains("UNIQUE") => {
                anyhow!("空间标识已被使用。")
            }
            _ => anyhow!(error),
        })?;
        let space_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO memberships (space_id, user_id, role, created_at)
             VALUES (?1, ?2, 'admin', ?3)",
            params![space_id, admin_user_id, now],
        )?;
        tx.commit()?;
        Ok(Space {
            id: space_id,
            slug,
            name,
            description,
            created_at: now,
        })
    })
    .await
}

pub async fn update_space(
    db: &Database,
    space_id: i64,
    name: &str,
    description: &str,
) -> Result<()> {
    let name = name.to_string();
    let description = description.to_string();
    db.call(move |conn| {
        conn.execute(
            "UPDATE spaces SET name = ?1, description = ?2 WHERE id = ?3",
            params![name, description, space_id],
        )?;
        Ok(())
    })
    .await
}

pub async fn summaries(db: &Database) -> Result<Vec<SpaceSummary>> {
    db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT s.id, s.slug, s.name, s.description, s.created_at,
                    (SELECT COUNT(*) FROM memberships m WHERE m.space_id = s.id),
                    (SELECT COUNT(*) FROM links l WHERE l.space_id = s.id AND l.owner_id IS NULL),
                    (SELECT COUNT(*) FROM links l WHERE l.space_id = s.id AND l.owner_id IS NOT NULL)
             FROM spaces s
             ORDER BY s.name COLLATE NOCASE",
        )?;
        let mut rows = stmt
            .query_map([], |row| {
                Ok((
                    map_space(row)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut result = Vec::with_capacity(rows.len());
        for (space, member_count, public_links, private_links) in rows.drain(..) {
            let mut admins_stmt = conn.prepare(
                "SELECT u.display_name FROM memberships m
                 JOIN users u ON u.id = m.user_id
                 WHERE m.space_id = ?1 AND m.role = 'admin'
                 ORDER BY u.display_name COLLATE NOCASE",
            )?;
            let admins = admins_stmt
                .query_map(params![space.id], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            result.push(SpaceSummary {
                space,
                member_count,
                public_links,
                private_links,
                admins,
            });
        }
        Ok(result)
    })
    .await
}

pub async fn find_space_by_id(db: &Database, space_id: i64) -> Result<Option<Space>> {
    db.call(move |conn| {
        let space = conn
            .query_row(
                "SELECT id, slug, name, description, created_at FROM spaces WHERE id = ?1",
                params![space_id],
                map_space,
            )
            .optional()?;
        Ok(space)
    })
    .await
}

fn map_space(row: &rusqlite::Row<'_>) -> rusqlite::Result<Space> {
    Ok(Space {
        id: row.get(0)?,
        slug: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        created_at: row.get(4)?,
    })
}
