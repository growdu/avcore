//! render-svc：渲染出片
//!
//! Phase 1：仅骨架 + 与 artifacts 表交互。
//! 详见 docs/modules/video-generation.md。

use crate::db::Db;
use crate::error::{AvcError, AvcResult};
use rusqlite::OptionalExtension;

pub fn create_job(db: &Db, name: &str, version: i64, _topic: &str) -> AvcResult<String> {
    let p = crate::svc::persona::get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();

    // Task 3：在任何 INSERT 前、同一连接内校验 version 状态。
    // - 无行 → NotFound("persona '<name>' version <n>")
    // - status 既不是 'ready' 也不是 'pending' → Conflict (信息含 version/status)
    let ver_status: Option<String> = conn
        .query_row(
            "SELECT status FROM persona_versions
             WHERE persona_model_id = ? AND version = ?",
            rusqlite::params![&p.id, version],
            |r| r.get(0),
        )
        .optional()?;
    match ver_status {
        None => {
            return Err(AvcError::NotFound(format!(
                "persona '{}' version {}",
                name, version
            )));
        }
        Some(s) if s != "ready" && s != "pending" => {
            return Err(AvcError::Conflict(format!(
                "persona '{}' version {} is not stable (status: {})",
                name, version, s
            )));
        }
        _ => {} // ready 或 pending，放行
    }

    let job_id = crate::svc::new_id("job");
    let now = crate::svc::now_iso();
    conn.execute(
        "INSERT INTO jobs (id, script_id, persona_model_id, persona_version, status, progress, created_at)
         VALUES (?, NULL, ?, ?, 'queued', 0, ?)",
        rusqlite::params![&job_id, &p.id, version, &now],
    )?;
    Ok(job_id)
}

pub fn list_jobs(db: &Db, name: &str) -> AvcResult<Vec<String>> {
    let p = crate::svc::persona::get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id FROM jobs WHERE persona_model_id = ? ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([&p.id], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows { out.push(r?); }
    Ok(out)
}

pub fn get_job(db: &Db, job_id: &str) -> AvcResult<String> {
    let conn = db.conn.lock().unwrap();
    let status: String = conn.query_row(
        "SELECT status FROM jobs WHERE id = ?",
        [job_id],
        |r| r.get(0),
    ).map_err(|_| AvcError::NotFound(format!("job '{}'", job_id)))?;
    Ok(status)
}
