//! iterate-svc：refine（数据迭代）
//!
//! 详见 docs/modules/persona-iteration.md §3。

use crate::db::Db;
use crate::error::AvcResult;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RefineChanges {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona_descriptor: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge_binding: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<serde_json::Value>,
}

/// 把 changes 应用到 persona_versions(version=N) 同行的可改列。
///
/// 不新增版本号；不调 Provider；不需要漂移兜底。
pub fn apply(db: &Db, name: &str, version: i64, changes: &RefineChanges) -> AvcResult<String> {
    let p = crate::svc::persona::get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();

    // 读现有 JSON 列
    let existing: (Option<String>, Option<String>, Option<String>) = conn.query_row(
        "SELECT persona_descriptor_json, knowledge_binding_json, manifest_json
         FROM persona_versions WHERE persona_model_id = ? AND version = ?",
        rusqlite::params![&p.id, version],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).map_err(|_| crate::error::AvcError::NotFound(format!("persona '{}' v{}", name, version)))?;

    let new_persona = merge_json(existing.0.as_deref(), changes.persona_descriptor.clone())?;
    let new_know = merge_json(existing.1.as_deref(), changes.knowledge_binding.clone())?;
    let new_manifest = merge_json(existing.2.as_deref(), changes.manifest.clone())?;

    conn.execute(
        "UPDATE persona_versions
         SET persona_descriptor_json = ?,
             knowledge_binding_json = ?,
             manifest_json = ?
         WHERE persona_model_id = ? AND version = ?",
        rusqlite::params![
            new_persona.as_deref(),
            new_know.as_deref(),
            new_manifest.as_deref(),
            &p.id,
            version,
        ],
    )?;

    // 写 iterate_jobs 账本
    let job_id = crate::svc::new_id("ij");
    let changes_json = serde_json::to_string(changes)?;
    let now = crate::svc::now_iso();
    conn.execute(
        "INSERT INTO iterate_jobs (id, persona_model_id, target_version, changes_json, status, started_at, finished_at)
         VALUES (?, ?, ?, ?, 'succeeded', ?, ?)",
        rusqlite::params![&job_id, &p.id, version, &changes_json, &now, &now],
    )?;

    Ok(job_id)
}

fn merge_json(existing: Option<&str>, patch: Option<serde_json::Value>) -> AvcResult<Option<String>> {
    let mut base: serde_json::Value = match existing {
        Some(s) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
        None => json!({}),
    };
    if let Some(p) = patch {
        merge_value(&mut base, &p);
    }
    if base.is_null() { return Ok(None); }
    Ok(Some(serde_json::to_string(&base)?))
}

fn merge_value(base: &mut serde_json::Value, patch: &serde_json::Value) {
    if let serde_json::Value::Object(p) = patch {
        // 先取出 entries 以避免借用 base
        let entries: Vec<(String, serde_json::Value)> = p.iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if !base.is_object() {
            *base = serde_json::Value::Object(serde_json::Map::new());
        }
        if let serde_json::Value::Object(b) = base {
            for (k, v) in entries {
                if v.is_null() {
                    b.remove(&k);
                } else {
                    merge_value(b.entry(k).or_insert(json!({})), &v);
                }
            }
        }
    } else {
        *base = patch.clone();
    }
}
