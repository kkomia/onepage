use std::path::Path;

use anyhow::{Context, Result};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use tokio::task::spawn_blocking;

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/001_init.sql")),
    (2, include_str!("../migrations/002_link_clicks.sql")),
];

#[derive(Clone)]
pub struct Database {
    pool: Pool<SqliteConnectionManager>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let manager = SqliteConnectionManager::file(path).with_init(|conn| {
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA foreign_keys = ON;
                 PRAGMA busy_timeout = 5000;
                 PRAGMA temp_store = MEMORY;",
            )
        });

        let pool = Pool::builder()
            .max_size(6)
            .min_idle(Some(1))
            .build(manager)
            .with_context(|| format!("无法打开数据库 {}", path.display()))?;

        {
            let mut conn = pool.get().context("无法获取数据库连接")?;
            migrate(&mut conn)?;
        }

        Ok(Self { pool })
    }

    pub async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let pool = self.pool.clone();
        spawn_blocking(move || {
            let mut conn = pool.get().context("无法获取数据库连接")?;
            f(&mut conn)
        })
        .await
        .context("数据库任务异常")?
    }

    pub async fn health(&self) -> bool {
        self.call(|conn| {
            let one: i64 = conn.query_row("SELECT 1", [], |row| row.get(0))?;
            Ok(one == 1)
        })
        .await
        .unwrap_or(false)
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .context("无法读取数据库版本")?;

    for (version, sql) in MIGRATIONS {
        if *version <= current {
            continue;
        }
        let tx = conn.transaction().context("无法开始数据库迁移")?;
        tx.execute_batch(sql)
            .with_context(|| format!("数据库迁移 v{version} 失败"))?;
        tx.pragma_update(None, "user_version", version)
            .context("无法写入数据库版本")?;
        tx.commit().context("无法提交数据库迁移")?;
        tracing::info!(version, "数据库迁移完成");
    }
    Ok(())
}
