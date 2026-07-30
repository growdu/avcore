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
