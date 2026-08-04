//! persona-svc：PersonaModel 管理
//!
//! 详见 docs/modules/persona-modeling.md。

use serde::{Deserialize, Serialize};

use crate::db::models::{PersonaModel, PersonaVersion};
use crate::db::Db;
use crate::error::{AvcError, AvcResult};

type PersonaEmbeds = (Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>);
type PersonaDumpRow = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<Vec<u8>>,
    Option<String>,
    Option<Vec<u8>>,
    Option<String>,
);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaSummary {
    pub id: String,
    pub name: String,
    pub archetype: Option<String>,
    pub current_version: i64,
    pub status: String,
}

impl From<&PersonaModel> for PersonaSummary {
    fn from(p: &PersonaModel) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            archetype: p.archetype.clone(),
            current_version: p.current_version,
            status: p.status.clone(),
        }
    }
}

pub fn list_personas(db: &Db, status: Option<&str>) -> AvcResult<Vec<PersonaSummary>> {
    let conn = db.conn.lock().unwrap();
    let (sql, params): (&str, Vec<String>) = match status {
        Some(s) => (
            "SELECT id, name, archetype, current_version, status
             FROM persona_models WHERE status = ? ORDER BY created_at",
            vec![s.to_string()],
        ),
        None => (
            "SELECT id, name, archetype, current_version, status
             FROM persona_models ORDER BY created_at",
            vec![],
        ),
    };

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        Ok(PersonaSummary {
            id: r.get(0)?,
            name: r.get(1)?,
            archetype: r.get(2)?,
            current_version: r.get(3)?,
            status: r.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn get_persona(db: &Db, name: &str) -> AvcResult<PersonaModel> {
    let conn = db.conn.lock().unwrap();
    conn.query_row(
        "SELECT id, name, archetype, description, current_version, status, created_at, updated_at
         FROM persona_models WHERE name = ?",
        [name],
        |r| {
            Ok(PersonaModel {
                id: r.get(0)?,
                name: r.get(1)?,
                archetype: r.get(2)?,
                description: r.get(3)?,
                current_version: r.get(4)?,
                status: r.get(5)?,
                created_at: r.get(6)?,
                updated_at: r.get(7)?,
            })
        },
    )
    .map_err(|_| AvcError::NotFound(format!("persona '{}'", name)))
}

pub fn list_versions(db: &Db, name: &str) -> AvcResult<Vec<i64>> {
    let p = get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT version FROM persona_versions WHERE persona_model_id = ? ORDER BY version",
    )?;
    let rows = stmt.query_map([&p.id], |r| r.get::<_, i64>(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn get_version(db: &Db, name: &str, version: i64) -> AvcResult<PersonaVersion> {
    let p = get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();
    conn.query_row(
        "SELECT persona_model_id, version, parent_version, status,
                avatar_provider, voice_provider,
                persona_descriptor_json, knowledge_binding_json, manifest_json, created_at
         FROM persona_versions WHERE persona_model_id = ? AND version = ?",
        rusqlite::params![&p.id, version],
        |r| {
            Ok(PersonaVersion {
                persona_model_id: r.get(0)?,
                version: r.get(1)?,
                parent_version: r.get(2)?,
                status: r.get(3)?,
                avatar_provider: r.get(4)?,
                voice_provider: r.get(5)?,
                persona_descriptor_json: r.get(6)?,
                knowledge_binding_json: r.get(7)?,
                manifest_json: r.get(8)?,
                created_at: r.get(9)?,
            })
        },
    )
    .map_err(|_| AvcError::NotFound(format!("persona '{}' v{}", name, version)))
}

/// 创建 PersonaModel + v1 初始占位行
pub fn create(
    db: &Db,
    name: &str,
    archetype: Option<&str>,
    description: Option<&str>,
) -> AvcResult<PersonaModel> {
    let mut conn = db.conn.lock().unwrap();
    let tx = conn.transaction()?;

    // 已存在？
    let exists: Option<bool> = tx
        .query_row(
            "SELECT 1 FROM persona_models WHERE name = ?",
            [name],
            |_| Ok(true),
        )
        .optional()?;
    let exists = exists.unwrap_or(false);
    if exists {
        return Err(AvcError::Conflict(format!("persona '{}' 已存在", name)));
    }

    let id = crate::svc::new_id("pm");
    let now = crate::svc::now_iso();

    tx.execute(
        "INSERT INTO persona_models
            (id, name, archetype, description, current_version, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, 1, 'pending', ?, ?)",
        rusqlite::params![&id, name, archetype, description, &now, &now],
    )?;

    tx.execute(
        "INSERT INTO persona_versions
            (persona_model_id, version, parent_version, status, created_at)
         VALUES (?, 1, NULL, 'pending', ?)",
        rusqlite::params![&id, &now],
    )?;

    tx.commit()?;
    drop(conn);

    get_persona(db, name)
}

// ============================================================================
// v0.3.4 — persona 资源挂载 / refine helpers / 状态转换 / 集成 onboard
// ============================================================================

use std::path::Path;

use serde_json::json;
use sha2::{Digest, Sha256};

use rusqlite::OptionalExtension;

/// 写 avatar_primary BLOB/mime/sha256；avatar_provider 写元信息列。
/// 简化：单文件 ref；多视角（avatar_views_blobs / avatar_refs_blobs）暂不接。
pub fn attach_avatar(
    db: &Db,
    name: &str,
    version: i64,
    ref_path: &Path,
    provider: Option<&str>,
) -> AvcResult<()> {
    let p = get_persona(db, name)?;
    let bytes = std::fs::read(ref_path)
        .map_err(|e| AvcError::Arg(format!("read avatar ref '{}': {}", ref_path.display(), e)))?;
    let mime = guess_mime(ref_path);
    let sha = hex::encode(Sha256::digest(&bytes));
    let conn = db.conn.lock().unwrap();
    let n = conn.execute(
        "UPDATE persona_versions
         SET avatar_primary = ?1,
             avatar_primary_mime = ?2,
             avatar_primary_sha256 = ?3,
             avatar_provider = ?4
         WHERE persona_model_id = ?5 AND version = ?6",
        rusqlite::params![&bytes, &mime, &sha, provider, &p.id, version],
    )?;
    if n == 0 {
        return Err(AvcError::NotFound(format!(
            "persona '{}' v{}",
            name, version
        )));
    }
    Ok(())
}

/// 写 voice_sample BLOB/mime/sha256。
pub fn attach_voice(
    db: &Db,
    name: &str,
    version: i64,
    ref_path: &Path,
    provider: Option<&str>,
) -> AvcResult<()> {
    let p = get_persona(db, name)?;
    let bytes = std::fs::read(ref_path)
        .map_err(|e| AvcError::Arg(format!("read voice ref '{}': {}", ref_path.display(), e)))?;
    let mime = guess_mime(ref_path);
    let sha = hex::encode(Sha256::digest(&bytes));
    let conn = db.conn.lock().unwrap();
    let n = conn.execute(
        "UPDATE persona_versions
         SET voice_sample = ?1,
             voice_sample_mime = ?2,
             voice_sample_sha256 = ?3,
             voice_provider = ?4
         WHERE persona_model_id = ?5 AND version = ?6",
        rusqlite::params![&bytes, &mime, &sha, provider, &p.id, version],
    )?;
    if n == 0 {
        return Err(AvcError::NotFound(format!(
            "persona '{}' v{}",
            name, version
        )));
    }
    Ok(())
}

/// 写 persona_descriptor_json（整段 JSON 对象 merge 到现有行）。
pub fn attach_persona(
    db: &Db,
    name: &str,
    version: i64,
    descriptor: &serde_json::Value,
) -> AvcResult<()> {
    let p = get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();
    let existing: Option<String> = conn
        .query_row(
            "SELECT persona_descriptor_json FROM persona_versions
             WHERE persona_model_id = ?1 AND version = ?2",
            rusqlite::params![&p.id, version],
            |r| r.get(0),
        )
        .map_err(|_| AvcError::NotFound(format!("persona '{}' v{}", name, version)))?;
    let merged = merge_descriptor(existing.as_deref(), descriptor.clone());
    let n = conn.execute(
        "UPDATE persona_versions SET persona_descriptor_json = ?1
         WHERE persona_model_id = ?2 AND version = ?3",
        rusqlite::params![Some(merged.as_str()), &p.id, version],
    )?;
    if n == 0 {
        return Err(AvcError::NotFound(format!(
            "persona '{}' v{}",
            name, version
        )));
    }
    Ok(())
}

/// 写 knowledge_binding_json（绑定 corpus）。
pub fn attach_knowledge(db: &Db, name: &str, version: i64, corpus_id: &str) -> AvcResult<()> {
    let p = get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();
    let existing: Option<String> = conn
        .query_row(
            "SELECT knowledge_binding_json FROM persona_versions
             WHERE persona_model_id = ?1 AND version = ?2",
            rusqlite::params![&p.id, version],
            |r| r.get(0),
        )
        .map_err(|_| AvcError::NotFound(format!("persona '{}' v{}", name, version)))?;
    let mut base: serde_json::Value = existing
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| json!({}));
    if !base.is_object() {
        base = json!({});
    }
    base.as_object_mut().unwrap().insert(
        "corpus".to_string(),
        serde_json::Value::String(corpus_id.to_string()),
    );
    base.as_object_mut().unwrap().insert(
        "attached_at".to_string(),
        serde_json::Value::String(crate::svc::now_iso()),
    );
    let s = serde_json::to_string(&base)?;
    let n = conn.execute(
        "UPDATE persona_versions SET knowledge_binding_json = ?1
         WHERE persona_model_id = ?2 AND version = ?3",
        rusqlite::params![Some(s.as_str()), &p.id, version],
    )?;
    if n == 0 {
        return Err(AvcError::NotFound(format!(
            "persona '{}' v{}",
            name, version
        )));
    }
    Ok(())
}

/// refine: 覆盖式 set persona_descriptor.traits（不是 merge，是替换数组）。
pub fn set_traits(db: &Db, name: &str, version: i64, traits: &[String]) -> AvcResult<()> {
    let arr = serde_json::Value::Array(
        traits
            .iter()
            .cloned()
            .map(serde_json::Value::String)
            .collect(),
    );
    let patch = json!({ "traits": arr });
    attach_persona(db, name, version, &patch)
}

/// refine: catchphrases 增 / 删。
pub fn set_catchphrase(
    db: &Db,
    name: &str,
    version: i64,
    add: &[String],
    remove: &[String],
) -> AvcResult<()> {
    let p = get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();
    let existing: Option<String> = conn
        .query_row(
            "SELECT persona_descriptor_json FROM persona_versions
             WHERE persona_model_id = ?1 AND version = ?2",
            rusqlite::params![&p.id, version],
            |r| r.get(0),
        )
        .map_err(|_| AvcError::NotFound(format!("persona '{}' v{}", name, version)))?;
    drop(conn);
    let mut base: serde_json::Value = existing
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| json!({}));
    if !base.is_object() {
        base = json!({});
    }
    let phrases = base
        .as_object_mut()
        .unwrap()
        .entry("catchphrases".to_string())
        .or_insert_with(|| json!([]));
    if !phrases.is_array() {
        *phrases = json!([]);
    }
    {
        let arr = phrases.as_array_mut().unwrap();
        for s in remove {
            arr.retain(|v| v.as_str() != Some(s.as_str()));
        }
        for s in add {
            if !arr.iter().any(|v| v.as_str() == Some(s.as_str())) {
                arr.push(serde_json::Value::String(s.clone()));
            }
        }
    }
    let s = serde_json::to_string(&base)?;
    let conn = db.conn.lock().unwrap();
    let n = conn.execute(
        "UPDATE persona_versions SET persona_descriptor_json = ?1
         WHERE persona_model_id = ?2 AND version = ?3",
        rusqlite::params![Some(s.as_str()), &p.id, version],
    )?;
    if n == 0 {
        return Err(AvcError::NotFound(format!(
            "persona '{}' v{}",
            name, version
        )));
    }
    Ok(())
}

/// refine: 改 manifest_json.render_options.<key> = <value>。
pub fn set_render_option(
    db: &Db,
    name: &str,
    version: i64,
    key: &str,
    value: &str,
) -> AvcResult<()> {
    let p = get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();
    let existing: Option<String> = conn
        .query_row(
            "SELECT manifest_json FROM persona_versions
             WHERE persona_model_id = ?1 AND version = ?2",
            rusqlite::params![&p.id, version],
            |r| r.get(0),
        )
        .map_err(|_| AvcError::NotFound(format!("persona '{}' v{}", name, version)))?;
    drop(conn);
    let mut base: serde_json::Value = existing
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| json!({}));
    if !base.is_object() {
        base = json!({});
    }
    let ro = base
        .as_object_mut()
        .unwrap()
        .entry("render_options".to_string())
        .or_insert_with(|| json!({}));
    if !ro.is_object() {
        *ro = json!({});
    }
    ro.as_object_mut().unwrap().insert(
        key.to_string(),
        serde_json::Value::String(value.to_string()),
    );
    let s = serde_json::to_string(&base)?;
    let conn = db.conn.lock().unwrap();
    let n = conn.execute(
        "UPDATE persona_versions SET manifest_json = ?1
         WHERE persona_model_id = ?2 AND version = ?3",
        rusqlite::params![Some(s.as_str()), &p.id, version],
    )?;
    if n == 0 {
        return Err(AvcError::NotFound(format!(
            "persona '{}' v{}",
            name, version
        )));
    }
    Ok(())
}

/// pending / building → ready。
/// 同时把当前 voice/face/style embed 复制到 anchor_*_emb 列（如果有），写 anchor_anchor_sha256。
/// 没有 embed 时 anchor_*_emb 留空（drift eval 走 fallback）。
pub fn commit(db: &Db, name: &str, version: i64) -> AvcResult<()> {
    let p = get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();
    // 校验存在 + 状态合法
    let cur_status: String = conn
        .query_row(
            "SELECT status FROM persona_versions WHERE persona_model_id = ?1 AND version = ?2",
            rusqlite::params![&p.id, version],
            |r| r.get(0),
        )
        .map_err(|_| AvcError::NotFound(format!("persona '{}' v{}", name, version)))?;
    if cur_status == "ready" {
        return Err(AvcError::Conflict(format!(
            "persona '{}' v{} 已 ready，不能再 commit",
            name, version
        )));
    }
    if cur_status == "deprecated" {
        return Err(AvcError::Conflict(format!(
            "persona '{}' v{} 已 deprecated，不能 commit",
            name, version
        )));
    }

    // 读 face/voice/style embed（如有），复制到 anchor_*_emb
    let (face, voice, style): PersonaEmbeds = conn
        .query_row(
            "SELECT face_embed, voice_embed, style_embed FROM persona_versions
             WHERE persona_model_id = ?1 AND version = ?2",
            rusqlite::params![&p.id, version],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| AvcError::NotFound(format!("persona '{}' v{}", name, version)))?;

    // anchor_anchor_sha256 = sha256(face||voice||style) 拼接（空字段当 None 算）
    let mut hasher = Sha256::new();
    let mut any = false;
    if let Some(f) = &face {
        hasher.update(b"face:");
        hasher.update(f);
        any = true;
    }
    if let Some(v) = &voice {
        hasher.update(b"voice:");
        hasher.update(v);
        any = true;
    }
    if let Some(s) = &style {
        hasher.update(b"style:");
        hasher.update(s);
        any = true;
    }
    let anchor_sha = if any {
        Some(hex::encode(hasher.finalize()))
    } else {
        None
    };

    let now = crate::svc::now_iso();
    let n = conn.execute(
        "UPDATE persona_versions
         SET status = 'ready',
             anchor_face_emb = ?1,
             anchor_voice_emb = ?2,
             anchor_style_emb = ?3,
             anchor_anchor_sha256 = ?4
         WHERE persona_model_id = ?5 AND version = ?6",
        rusqlite::params![face, voice, style, anchor_sha, &p.id, version],
    )?;
    if n == 0 {
        return Err(AvcError::NotFound(format!(
            "persona '{}' v{}",
            name, version
        )));
    }

    // persona_models 也升 status='active'（如还在 pending）
    if p.status == "pending" {
        conn.execute(
            "UPDATE persona_models SET status = 'active', updated_at = ?1 WHERE id = ?2",
            rusqlite::params![&now, &p.id],
        )?;
    } else {
        conn.execute(
            "UPDATE persona_models SET updated_at = ?1 WHERE id = ?2",
            rusqlite::params![&now, &p.id],
        )?;
    }

    Ok(())
}

/// 把 persona_models.current_version 改到 to_v。
/// - to_v 必须存在
/// - to_v 不能是 deprecated
/// - persona 不能 archived
pub fn promote(db: &Db, name: &str, to_version: i64) -> AvcResult<()> {
    let p = get_persona(db, name)?;
    if p.status == "archived" {
        return Err(AvcError::Conflict(format!(
            "persona '{}' 已 archived，不能 promote",
            name
        )));
    }
    let conn = db.conn.lock().unwrap();
    let status: String = conn
        .query_row(
            "SELECT status FROM persona_versions WHERE persona_model_id = ?1 AND version = ?2",
            rusqlite::params![&p.id, to_version],
            |r| r.get::<_, String>(0),
        )
        .map_err(|_| AvcError::NotFound(format!("persona '{}' v{}", name, to_version)))?;
    if status == "deprecated" {
        return Err(AvcError::Conflict(format!(
            "persona '{}' v{} 已 deprecated，不能 promote",
            name, to_version
        )));
    }
    let now = crate::svc::now_iso();
    conn.execute(
        "UPDATE persona_models SET current_version = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![to_version, &now, &p.id],
    )?;
    Ok(())
}

/// 把该 version 标 deprecated。
pub fn demote(db: &Db, name: &str, version: i64) -> AvcResult<()> {
    let p = get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();
    let n = conn.execute(
        "UPDATE persona_versions SET status = 'deprecated' WHERE persona_model_id = ?1 AND version = ?2",
        rusqlite::params![&p.id, version],
    )?;
    if n == 0 {
        return Err(AvcError::NotFound(format!(
            "persona '{}' v{}",
            name, version
        )));
    }
    Ok(())
}

/// 软删除：persona_models.status = 'archived'。
pub fn archive(db: &Db, name: &str) -> AvcResult<()> {
    let p = get_persona(db, name)?;
    if p.status == "archived" {
        return Err(AvcError::Conflict(format!(
            "persona '{}' 已 archived",
            name
        )));
    }
    let now = crate::svc::now_iso();
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "UPDATE persona_models SET status = 'archived', updated_at = ?1 WHERE id = ?2",
        rusqlite::params![&now, &p.id],
    )?;
    Ok(())
}

/// 硬删除：删 persona_models + persona_versions + persona_samples + iterate/finetune/jobs/artifacts。
/// confirm=true 才执行；false → Conflict。
pub fn delete(db: &Db, name: &str, confirm: bool) -> AvcResult<()> {
    if !confirm {
        return Err(AvcError::Conflict(
            "delete 需要 --confirm 才执行（防止误删）".into(),
        ));
    }
    let p = get_persona(db, name)?;
    let mut conn = db.conn.lock().unwrap();
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM artifacts WHERE job_id IN
         (SELECT id FROM jobs WHERE persona_model_id = ?1)",
        rusqlite::params![&p.id],
    )?;
    tx.execute(
        "DELETE FROM jobs WHERE persona_model_id = ?1",
        rusqlite::params![&p.id],
    )?;
    tx.execute(
        "DELETE FROM finetune_jobs WHERE persona_model_id = ?1",
        rusqlite::params![&p.id],
    )?;
    tx.execute(
        "DELETE FROM iterate_jobs WHERE persona_model_id = ?1",
        rusqlite::params![&p.id],
    )?;
    tx.execute(
        "DELETE FROM persona_samples WHERE persona_model_id = ?1",
        rusqlite::params![&p.id],
    )?;
    tx.execute(
        "DELETE FROM persona_versions WHERE persona_model_id = ?1",
        rusqlite::params![&p.id],
    )?;
    tx.execute(
        "DELETE FROM persona_models WHERE id = ?1",
        rusqlite::params![&p.id],
    )?;
    tx.commit()?;
    Ok(())
}

/// 读 current_version。
pub fn current_version(db: &Db, name: &str) -> AvcResult<i64> {
    Ok(get_persona(db, name)?.current_version)
}

/// 一次性导出可读目录：
/// - descriptor.json / knowledge.json / manifest.json
/// - avatar.bin / voice.bin（如有 BLOB）
pub fn dump(db: &Db, name: &str, version: i64, out_dir: &Path) -> AvcResult<()> {
    let p = get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();
    let row: PersonaDumpRow = conn
        .query_row(
            "SELECT persona_descriptor_json, knowledge_binding_json, manifest_json,
                    avatar_primary, avatar_primary_mime, voice_sample, voice_sample_mime
             FROM persona_versions WHERE persona_model_id = ?1 AND version = ?2",
            rusqlite::params![&p.id, version],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .map_err(|_| AvcError::NotFound(format!("persona '{}' v{}", name, version)))?;
    drop(conn);
    std::fs::create_dir_all(out_dir).map_err(|e| {
        AvcError::Internal(format!("create dump dir '{}': {}", out_dir.display(), e))
    })?;
    let write_json = |fname: &str, raw: Option<String>| -> AvcResult<()> {
        let path = out_dir.join(fname);
        let body = raw.unwrap_or_else(|| "{}".to_string());
        std::fs::write(&path, body)
            .map_err(|e| AvcError::Internal(format!("write '{}': {}", path.display(), e)))?;
        Ok(())
    };
    write_json("descriptor.json", row.0)?;
    write_json("knowledge.json", row.1)?;
    write_json("manifest.json", row.2)?;
    if let (Some(b), _) = (row.3.as_ref(), row.4.as_ref()) {
        std::fs::write(out_dir.join("avatar.bin"), b)
            .map_err(|e| AvcError::Internal(format!("write avatar.bin: {}", e)))?;
    }
    if let (Some(b), _) = (row.5.as_ref(), row.6.as_ref()) {
        std::fs::write(out_dir.join("voice.bin"), b)
            .map_err(|e| AvcError::Internal(format!("write voice.bin: {}", e)))?;
    }
    // 加一个 README.txt 说明版本
    let readme = format!("persona: {}\nversion: {}\n", name, version);
    std::fs::write(out_dir.join("README.txt"), readme)
        .map_err(|e| AvcError::Internal(format!("write README.txt: {}", e)))?;
    Ok(())
}

/// onboard 集成：create + attach-* + commit。
#[derive(Default)]
pub struct OnboardSpec {
    pub archetype: Option<String>,
    pub description: Option<String>,
    pub avatar_ref: Option<std::path::PathBuf>,
    pub avatar_provider: Option<String>,
    pub voice_ref: Option<std::path::PathBuf>,
    pub voice_provider: Option<String>,
    pub descriptor: Option<serde_json::Value>,
    pub corpus_id: Option<String>,
}

pub fn onboard(db: &Db, name: &str, spec: OnboardSpec) -> AvcResult<()> {
    // 1. create
    create(
        db,
        name,
        spec.archetype.as_deref(),
        spec.description.as_deref(),
    )?;
    // 2. attach-avatar
    if let Some(p) = &spec.avatar_ref {
        attach_avatar(db, name, 1, p, spec.avatar_provider.as_deref())?;
    }
    // 3. attach-voice
    if let Some(p) = &spec.voice_ref {
        attach_voice(db, name, 1, p, spec.voice_provider.as_deref())?;
    }
    // 4. attach-persona
    if let Some(d) = &spec.descriptor {
        attach_persona(db, name, 1, d)?;
    }
    // 5. attach-knowledge（可选）
    if let Some(c) = &spec.corpus_id {
        attach_knowledge(db, name, 1, c)?;
    }
    // 6. commit → ready
    commit(db, name, 1)?;
    Ok(())
}

// ---- helpers ----

fn merge_descriptor(existing: Option<&str>, patch: serde_json::Value) -> String {
    let mut base: serde_json::Value = existing
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| json!({}));
    if !base.is_object() {
        base = json!({});
    }
    if let serde_json::Value::Object(p) = patch {
        for (k, v) in p {
            base.as_object_mut().unwrap().insert(k, v);
        }
    }
    serde_json::to_string(&base).unwrap_or_else(|_| "{}".to_string())
}

fn guess_mime(path: &Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png".into(),
        Some("jpg") | Some("jpeg") => "image/jpeg".into(),
        Some("webp") => "image/webp".into(),
        Some("gif") => "image/gif".into(),
        Some("wav") => "audio/wav".into(),
        Some("mp3") => "audio/mpeg".into(),
        Some("m4a") => "audio/mp4".into(),
        Some("flac") => "audio/flac".into(),
        Some("ogg") => "audio/ogg".into(),
        Some("txt") => "text/plain".into(),
        Some("json") => "application/json".into(),
        _ => "application/octet-stream".into(),
    }
}
