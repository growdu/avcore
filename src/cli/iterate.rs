//! `avc iterate <verb>`

use crate::db::Db;
use crate::output::{print, OutputMode};
use crate::AvcError;
use crate::AvcResult;
use serde_json::json;

pub fn dispatch(argv: &[String]) -> AvcResult<()> {
    let mode = OutputMode::from_flags(
        argv.iter().any(|a| a == "--json"),
        argv.iter().any(|a| a == "--quiet"),
    );

    if argv.is_empty() {
        return Err(AvcError::Arg("iterate list|apply|show|cancel ...".into()));
    }

    let db = Db::open_default()?;
    let argv_ref: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();

    match argv_ref[0] {
        "list" => {
            let name = argv_ref
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("iterate list <persona>".into()))?;
            // 先验 persona 存在：未知 -> NotFound (exit 3)，与 persona show / apply 一致
            crate::svc::persona::get_persona(&db, name)?;
            let conn = db.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT ij.id, ij.persona_model_id, ij.target_version, ij.status, ij.started_at
                 FROM iterate_jobs ij
                 JOIN persona_models pm ON pm.id = ij.persona_model_id
                 WHERE pm.name = ?
                 ORDER BY ij.started_at DESC",
            )?;
            let rows = stmt.query_map([name], |r| {
                Ok(json!({
                    "id": r.get::<_, String>(0)?,
                    "persona_model_id": r.get::<_, String>(1)?,
                    "target_version": r.get::<_, i64>(2)?,
                    "status": r.get::<_, String>(3)?,
                    "started_at": r.get::<_, Option<String>>(4)?,
                }))
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            print(mode, &out)?;
        }
        "show" => {
            // iterate show <ij_id> → ij 详情（changes_json / status / started_at / finished_at）
            let ij_id = argv_ref
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("iterate show <ij_id>".into()))?;
            let conn = db.conn.lock().unwrap();
            type IjRow = (
                String,
                String,
                i64,
                String,
                String,
                Option<String>,
                Option<String>,
            );
            let row: Result<IjRow, _> = conn.query_row(
                "SELECT ij.id, pm.name, ij.target_version, ij.changes_json, ij.status,
                            ij.started_at, ij.finished_at
                     FROM iterate_jobs ij
                     JOIN persona_models pm ON pm.id = ij.persona_model_id
                     WHERE ij.id = ?",
                [ij_id],
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
            );
            drop(conn);
            let (id, name, target_v, changes, status, started, finished) =
                row.map_err(|_| AvcError::NotFound(format!("iterate job '{}'", ij_id)))?;
            let changes_parsed: serde_json::Value =
                serde_json::from_str(&changes).unwrap_or(serde_json::Value::String(changes));
            print(
                mode,
                &serde_json::json!({
                    "id": id,
                    "persona": name,
                    "target_version": target_v,
                    "status": status,
                    "changes": changes_parsed,
                    "started_at": started,
                    "finished_at": finished,
                }),
            )?;
        }
        "cancel" => {
            // iterate cancel <ij_id> → 标 status='cancelled'（仅 queued/running；succeeded/failed/cancelled 拒）
            let ij_id = argv_ref
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("iterate cancel <ij_id>".into()))?;
            let mut conn = db.conn.lock().unwrap();
            let tx = conn.transaction()?;
            let status: String = tx
                .query_row(
                    "SELECT status FROM iterate_jobs WHERE id = ?",
                    [ij_id],
                    |r| r.get(0),
                )
                .map_err(|_| AvcError::NotFound(format!("iterate job '{}'", ij_id)))?;
            if status != "queued" && status != "running" {
                return Err(AvcError::Conflict(format!(
                    "iterate job '{}' is in '{}' state; cannot cancel",
                    ij_id, status
                )));
            }
            tx.execute(
                "UPDATE iterate_jobs SET status = 'cancelled', finished_at = ? WHERE id = ?",
                rusqlite::params![crate::svc::now_iso(), ij_id],
            )?;
            tx.commit()?;
            print(
                mode,
                &serde_json::json!({"iterate_job_id": ij_id, "status": "cancelled"}),
            )?;
        }
        "apply" => {
            let name = argv_ref.get(1).copied().ok_or_else(|| {
                AvcError::Arg("iterate apply <name> --version <v> --set-persona <json>".into())
            })?;
            let mut version: Option<i64> = None;
            let mut set_persona: Option<String> = None;
            let mut set_knowledge: Option<String> = None;
            let mut set_manifest: Option<String> = None;
            let mut i = 2;
            while i < argv_ref.len() {
                match argv_ref[i] {
                    "--version" => {
                        version = argv_ref.get(i + 1).and_then(|s| s.parse().ok());
                        i += 2;
                    }
                    "--set-persona" => {
                        set_persona = argv_ref.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    "--set-knowledge" => {
                        set_knowledge = argv_ref.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    "--set-manifest" => {
                        set_manifest = argv_ref.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            let version = version.ok_or_else(|| AvcError::Arg("--version 必填".into()))?;
            let mut changes = crate::svc::iterate::RefineChanges::default();
            if let Some(s) = set_persona {
                changes.persona_descriptor = serde_json::from_str(&s)?;
            }
            if let Some(s) = set_knowledge {
                changes.knowledge_binding = serde_json::from_str(&s)?;
            }
            if let Some(s) = set_manifest {
                changes.manifest = serde_json::from_str(&s)?;
            }
            let job_id = crate::svc::iterate::apply(&db, name, version, &changes)?;
            print(
                mode,
                &json!({"iterate_job_id": job_id, "persona": name, "version": version}),
            )?;
        }
        _ => {
            return Err(AvcError::Arg(format!(
                "iterate: unknown verb '{}'",
                argv_ref[0]
            )))
        }
    }
    Ok(())
}
