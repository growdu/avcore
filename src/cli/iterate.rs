//! `avc iterate <verb>`

use crate::AvcError;
use crate::AvcResult;
use crate::db::Db;
use crate::output::{print, OutputMode};
use serde_json::json;

pub fn dispatch(argv: &[String]) -> AvcResult<()> {
    let mode = OutputMode::from_flags(
        argv.iter().any(|a| a == "--json"),
        argv.iter().any(|a| a == "--quiet"),
    );

    if argv.is_empty() {
        return Err(AvcError::Arg("iterate list|apply|show ...".into()));
    }

    let db = Db::open_default()?;
    let argv_ref: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();

    match argv_ref[0] {
        "list" => {
            let name = argv_ref.get(1).copied()
                .ok_or_else(|| AvcError::Arg("iterate list <persona>".into()))?;
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
            for r in rows { out.push(r?); }
            print(mode, &out)?;
        }
        "apply" => {
            let name = argv_ref.get(1).copied()
                .ok_or_else(|| AvcError::Arg("iterate apply <name> --version <v> --set-persona <json>".into()))?;
            let mut version: Option<i64> = None;
            let mut set_persona: Option<String> = None;
            let mut set_knowledge: Option<String> = None;
            let mut set_manifest: Option<String> = None;
            let mut i = 2;
            while i < argv_ref.len() {
                match argv_ref[i] {
                    "--version" => {
                        version = argv_ref.get(i+1).and_then(|s| s.parse().ok());
                        i += 2;
                    }
                    "--set-persona" => {
                        set_persona = argv_ref.get(i+1).map(|s| s.to_string());
                        i += 2;
                    }
                    "--set-knowledge" => {
                        set_knowledge = argv_ref.get(i+1).map(|s| s.to_string());
                        i += 2;
                    }
                    "--set-manifest" => {
                        set_manifest = argv_ref.get(i+1).map(|s| s.to_string());
                        i += 2;
                    }
                    _ => { i += 1; }
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
            print(mode, &json!({"iterate_job_id": job_id, "persona": name, "version": version}))?;
        }
        _ => return Err(AvcError::Arg(format!("iterate: unknown verb '{}'", argv_ref[0]))),
    }
    Ok(())
}
