//! sample-svc：训练样本管理
//!
//! 详见 docs/modules/persona-iteration.md。

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::db::models::PersonaSample;
use crate::db::Db;
use crate::error::{AvcError, AvcResult};

/// 加一条 sample。
///
/// - kind: image / audio / behavior_text / feedback
/// - text: 文本（behavior_text 必填；feedback 可选；image/audio 用 uri）
/// - uri: 文件路径（image/audio 必填，行为可空，feedback 可空）
/// - source: user / vendor / system
/// - consent_proof: 同意证据路径（必填）
pub fn add(
    db: &Db,
    persona_name: &str,
    kind: &str,
    uri: Option<&Path>,
    text: Option<&str>,
    source: &str,
    consent_proof: Option<&Path>,
) -> AvcResult<String> {
    // 校验 kind
    match kind {
        "image" | "audio" | "behavior_text" | "feedback" => {}
        other => {
            return Err(AvcError::Arg(format!(
                "sample kind '{}' 非法；应为 image|audio|behavior_text|feedback",
                other
            )));
        }
    }
    // kind 必填项检查
    match kind {
        "image" | "audio" => {
            if uri.is_none() {
                return Err(AvcError::Arg(format!(
                    "sample kind '{}' 必须传 --uri <path>",
                    kind
                )));
            }
        }
        "behavior_text" => {
            if text.is_none() {
                return Err(AvcError::Arg(
                    "sample kind 'behavior_text' 必须传 --text <s>".into(),
                ));
            }
        }
        "feedback" => {
            // feedback 可选 uri 或 text
        }
        _ => unreachable!(),
    }
    let p = crate::svc::persona::get_persona(db, persona_name)?;
    let id = crate::svc::new_id("sm");
    let now = crate::svc::now_iso();

    let (blob, mime, sha) = if let Some(p) = uri {
        let bytes = std::fs::read(p)
            .map_err(|e| AvcError::Arg(format!("read sample uri '{}': {}", p.display(), e)))?;
        let m = guess_mime(p);
        let s = hex::encode(Sha256::digest(&bytes));
        (Some(bytes), Some(m), Some(s))
    } else {
        (None, None, None)
    };
    let consent_str = consent_proof.map(|p| p.display().to_string());
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO persona_samples
            (id, persona_model_id, kind, blob, blob_mime, text, source, consent_proof, sha256, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            &id,
            &p.id,
            kind,
            blob,
            mime,
            text,
            source,
            consent_str,
            sha,
            &now,
        ],
    )?;
    Ok(id)
}

pub fn list(db: &Db, persona_name: &str, kind: Option<&str>) -> AvcResult<Vec<PersonaSample>> {
    let p = crate::svc::persona::get_persona(db, persona_name)?;
    let conn = db.conn.lock().unwrap();
    let (sql, params): (String, Vec<String>) = match kind {
        Some(k) => (
            "SELECT id, persona_model_id, version_id_at_collection, kind, text, source,
                    consent_proof, quality_score, sha256, created_at
             FROM persona_samples WHERE persona_model_id = ?1 AND kind = ?2
             ORDER BY created_at"
                .to_string(),
            vec![p.id.clone(), k.to_string()],
        ),
        None => (
            "SELECT id, persona_model_id, version_id_at_collection, kind, text, source,
                    consent_proof, quality_score, sha256, created_at
             FROM persona_samples WHERE persona_model_id = ?1
             ORDER BY created_at"
                .to_string(),
            vec![p.id.clone()],
        ),
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        Ok(PersonaSample {
            id: r.get(0)?,
            persona_model_id: r.get(1)?,
            version_id_at_collection: r.get(2)?,
            kind: r.get(3)?,
            text: r.get(4)?,
            source: r.get(5)?,
            consent_proof: r.get(6)?,
            quality_score: r.get(7)?,
            sha256: r.get(8)?,
            created_at: r.get(9)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn show(db: &Db, sample_id: &str) -> AvcResult<PersonaSample> {
    let conn = db.conn.lock().unwrap();
    conn.query_row(
        "SELECT id, persona_model_id, version_id_at_collection, kind, text, source,
                consent_proof, quality_score, sha256, created_at
         FROM persona_samples WHERE id = ?1",
        [sample_id],
        |r| {
            Ok(PersonaSample {
                id: r.get(0)?,
                persona_model_id: r.get(1)?,
                version_id_at_collection: r.get(2)?,
                kind: r.get(3)?,
                text: r.get(4)?,
                source: r.get(5)?,
                consent_proof: r.get(6)?,
                quality_score: r.get(7)?,
                sha256: r.get(8)?,
                created_at: r.get(9)?,
            })
        },
    )
    .map_err(|_| AvcError::NotFound(format!("sample '{}'", sample_id)))
}

pub fn remove(db: &Db, sample_id: &str) -> AvcResult<()> {
    let conn = db.conn.lock().unwrap();
    let n = conn.execute("DELETE FROM persona_samples WHERE id = ?1", [sample_id])?;
    if n == 0 {
        return Err(AvcError::NotFound(format!("sample '{}'", sample_id)));
    }
    Ok(())
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
