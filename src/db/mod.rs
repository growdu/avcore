//! 存储层：单一 SQLite 文件 = 全部状态
//!
//! 详见 docs/storage.md。

pub mod models;
pub mod schema;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::Connection;
use std::sync::Mutex;

use crate::error::AvcResult;

pub type DbPool = Arc<Mutex<Connection>>;

pub struct Db {
    pub conn: DbPool,
    pub path: PathBuf,
}

impl Db {
    /// 打开数据库 + 跑迁移（幂等）
    pub fn open(path: &Path) -> AvcResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        let conn = Arc::new(Mutex::new(conn));
        let db = Db {
            conn,
            path: path.to_path_buf(),
        };
        schema::migrate(&db)?;
        Ok(db)
    }

    pub fn open_default() -> AvcResult<Self> {
        let path = crate::config::Config::default_db_path()?;
        Self::open(&path)
    }
}

/// 打开默认 DB 文件 + 跑迁移并把裸 `Connection` 交出去。
///
/// 用途：provider hook 在 token 鉴权 / 限速 / 超时 / 上游错时**被动**落库；hook
/// 不持有 `Db` 句柄，且必须能容忍"无 DB / 无 schema"的情况（best-effort）。详见
/// `src/provider/real.rs::db_conn`。
pub fn open_default() -> AvcResult<Connection> {
    use std::sync::Arc;
    let path = crate::config::Config::default_db_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let conn = Connection::open(&path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    let pool: DbPool = Arc::new(Mutex::new(conn));
    let db = Db {
        conn: pool,
        path: path.clone(),
    };
    schema::migrate(&db)?;
    // 取出里面那个 Connection（hook 场景下没人共享，try_unwrap 必成功）
    let conn = Arc::try_unwrap(db.conn)
        .map_err(|_| crate::error::AvcError::Db("connection still shared".into()))?
        .into_inner()
        .map_err(|_| crate::error::AvcError::Db("lock poisoned".into()))?;
    Ok(conn)
}
