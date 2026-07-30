//! persona-svc：PersonaModel 管理
//!
//! 详见 docs/modules/persona-modeling.md。

use serde::{Deserialize, Serialize};

use crate::db::models::{PersonaModel, PersonaVersion};
use crate::db::Db;
use crate::error::{AvcError, AvcResult};

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
    for row in rows { out.push(row?); }
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
    ).map_err(|_| AvcError::NotFound(format!("persona '{}'", name)))
}

pub fn list_versions(db: &Db, name: &str) -> AvcResult<Vec<i64>> {
    let p = get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT version FROM persona_versions WHERE persona_model_id = ? ORDER BY version",
    )?;
    let rows = stmt.query_map([&p.id], |r| r.get::<_, i64>(0))?;
    let mut out = Vec::new();
    for row in rows { out.push(row?); }
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
    ).map_err(|_| AvcError::NotFound(format!("persona '{}' v{}", name, version)))
}

/// 创建 PersonaModel + v1 初始占位行
pub fn create(db: &Db, name: &str, archetype: Option<&str>, description: Option<&str>) -> AvcResult<PersonaModel> {
    let mut conn = db.conn.lock().unwrap();
    let tx = conn.transaction()?;

    // 已存在？
    let exists: Option<bool> = tx.query_row(
        "SELECT 1 FROM persona_models WHERE name = ?",
        [name],
        |_| Ok(true),
    ).optional()?;
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

use rusqlite::OptionalExtension;
