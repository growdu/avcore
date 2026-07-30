//! `avc finetune <verb>`

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
        return Err(AvcError::Arg("finetune start|list|show ...".into()));
    }

    let db = Db::open_default()?;
    let argv_ref: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();

    match argv_ref[0] {
        "list" => {
            let name = argv_ref.get(1).copied()
                .ok_or_else(|| AvcError::Arg("finetune list <persona>".into()))?;
            let conn = db.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT fj.id, fj.persona_model_id, fj.base_version, fj.target_version, fj.status, fj.started_at
                 FROM finetune_jobs fj
                 JOIN persona_models pm ON pm.id = fj.persona_model_id
                 WHERE pm.name = ?
                 ORDER BY fj.started_at DESC",
            )?;
            let rows = stmt.query_map([name], |r| {
                Ok(json!({
                    "id": r.get::<_, String>(0)?,
                    "persona_model_id": r.get::<_, String>(1)?,
                    "base_version": r.get::<_, i64>(2)?,
                    "target_version": r.get::<_, Option<i64>>(3)?,
                    "status": r.get::<_, String>(4)?,
                    "started_at": r.get::<_, Option<String>>(5)?,
                }))
            })?;
            let mut out = Vec::new();
            for r in rows { out.push(r?); }
            print(mode, &out)?;
        }
        "start" => {
            let name = argv_ref.get(1).copied()
                .ok_or_else(|| AvcError::Arg("finetune start <name> --scope voice --base-version <v>".into()))?;
            let mut base: Option<i64> = None;
            let mut scope: Vec<String> = vec!["voice".into()];
            let mut threshold: f32 = 0.85;
            let mut i = 2;
            while i < argv_ref.len() {
                match argv_ref[i] {
                    "--base-version" => { base = argv_ref.get(i+1).and_then(|s| s.parse().ok()); i += 2; }
                    "--scope" => {
                        scope = argv_ref.get(i+1).map(|s| s.split(',').map(String::from).collect()).unwrap_or(scope);
                        i += 2;
                    }
                    "--threshold" => { threshold = argv_ref.get(i+1).and_then(|s| s.parse().ok()).unwrap_or(threshold); i += 2; }
                    _ => { i += 1; }
                }
            }
            let base = base.ok_or_else(|| AvcError::Arg("--base-version 必填".into()))?;
            let cfg = crate::svc::finetune::FinetuneConfig {
                full_retrain: false,
                epochs: 1,
                consistency_threshold: threshold,
            };
            let fj_id = crate::svc::finetune::start(&db, name, &scope, base, &cfg)?;
            print(mode, &json!({
                "finetune_job_id": fj_id,
                "persona": name,
                "base_version": base,
                "target_version": base + 1,
                "scope": scope,
                "threshold": threshold,
                "next_step": "Provider SFT 调用 (Phase 2+); 当前阶段需手动调 svc::finetune::publish(...)"
            }))?;
        }
        "publish" => {
            // 测试用：模拟 drift 报告，强制通过/失败
            let fj_id = argv_ref.get(1).copied()
                .ok_or_else(|| AvcError::Arg("finetune publish <fj_id> --passed|--failed".into()))?;
            let passed = argv_ref.iter().any(|a| *a == "--passed");
            let drift = crate::svc::finetune::DriftReport {
                face: 0.9, voice: 0.9, style: 0.9, avg: 0.9, passed,
            };
            crate::svc::finetune::publish(&db, fj_id, &drift)?;
            print(mode, &json!({"finetune_job_id": fj_id, "published": passed}))?;
        }
        _ => return Err(AvcError::Arg(format!("finetune: unknown verb '{}'", argv_ref[0]))),
    }
    Ok(())
}
