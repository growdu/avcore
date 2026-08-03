//! Schema migrations
//!
//! 启动时按版本顺序跑；已跑过的版本跳过。
//! 详见 docs/storage.md §4。

use crate::db::Db;
use crate::error::AvcResult;

const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../../migrations/0001_init.sql")),
    (
        "0002_drift_dimensions",
        include_str!("../../migrations/0002_drift_dimensions.sql"),
    ),
    (
        "0003_provider_health",
        include_str!("../../migrations/0003_provider_health.sql"),
    ),
];

pub fn migrate(db: &Db) -> AvcResult<()> {
    let mut conn = db.conn.lock().unwrap();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            id TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL
        )",
        [],
    )?;

    for (id, sql) in MIGRATIONS {
        let applied: Option<bool> = conn
            .query_row("SELECT 1 FROM schema_migrations WHERE id = ?", [id], |_| {
                Ok(true)
            })
            .optional()?;
        let applied = applied.unwrap_or(false);

        if applied {
            continue;
        }

        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute(
            "INSERT INTO schema_migrations (id, applied_at) VALUES (?, datetime('now'))",
            [id],
        )?;
        tx.commit()?;
    }
    Ok(())
}

// 让 optional() 可用
use rusqlite::OptionalExtension;
