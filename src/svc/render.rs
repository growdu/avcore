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

/// Phase 2: list_artifacts → 给 `avc job show --artifacts` 用。
pub fn list_artifacts(db: &Db, job_id: &str) -> AvcResult<Vec<(String, String, Option<i64>, Option<String>)>> {
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT kind, name, byte_size, mime FROM artifacts WHERE job_id = ? ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([job_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows { out.push(r?); }
    Ok(out)
}

/// Phase 2: 把一个 job 的所有 artifacts (BLOB) 落 FS 到 out_dir/<kind>__<name>__<id>.bin。
/// 返 (写出文件数, 累计 bytes)。
pub fn export_artifacts(db: &Db, job_id: &str, out_dir: &std::path::Path) -> AvcResult<(usize, u64)> {
    // 0. 校验 job 存在
    let exists: bool = db.conn.lock().unwrap().query_row(
        "SELECT 1 FROM jobs WHERE id = ?",
        [job_id],
        |r| r.get::<_, i64>(0).map(|_| true),
    ).optional()?.unwrap_or(false);
    if !exists {
        return Err(AvcError::NotFound(format!("job '{}'", job_id)));
    }

    // 1. mkdir -p
    std::fs::create_dir_all(out_dir).map_err(|e| AvcError::Db(format!(
        "mkdir {}: {}", out_dir.display(), e
    )))?;

    // 2. 读每条 artifact BLOB 写文件
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, kind, name, content FROM artifacts WHERE job_id = ? ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([job_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Vec<u8>>(3)?,
        ))
    })?;
    let mut count = 0usize;
    let mut total_bytes = 0u64;
    for r in rows {
        let (id, kind, name, blob) = r?;
        let safe_kind = kind.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect::<String>();
        let safe_name = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' { c } else { '_' }).collect::<String>();
        let path = out_dir.join(format!("{}__{}__{}.bin", safe_kind, safe_name, id));
        std::fs::write(&path, &blob).map_err(|e| AvcError::Db(format!(
            "write {}: {}", path.display(), e
        )))?;
        total_bytes += blob.len() as u64;
        count += 1;
    }
    Ok((count, total_bytes))
}

/// Phase 2: 反馈入口 — 用户标 "looks_unlike" → 写 persona_samples(kind='feedback', source='user_feedback')。
/// reason 可空（CLI --reason 留空就 NULL）。
/// 返 sample_id。
pub fn feedback(
    db: &Db,
    job_id: &str,
    looks_unlike: bool,
    reason: Option<&str>,
) -> AvcResult<String> {
    if !looks_unlike {
        return Err(AvcError::Arg(
            "feedback: only --looks-unlike is supported in Phase 2".into(),
        ));
    }
    let conn = db.conn.lock().unwrap();
    let (persona_id, persona_version, _script_id): (String, i64, Option<String>) = conn
        .query_row(
            "SELECT persona_model_id, persona_version, script_id FROM jobs WHERE id = ?",
            [job_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| AvcError::NotFound(format!("job '{}'", job_id)))?;
    drop(conn);

    let sample_id = crate::svc::new_id("smp");
    let now = crate::svc::now_iso();
    let text = reason.unwrap_or("looks_unlike");
    let conn2 = db.conn.lock().unwrap();
    conn2.execute(
        "INSERT INTO persona_samples (
            id, persona_model_id, version_id_at_collection, source, kind, text, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            &sample_id,
            &persona_id,
            persona_version,
            "user_feedback",
            "feedback",
            text,
            &now,
        ],
    )?;
    Ok(sample_id)
}
