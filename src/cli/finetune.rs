//! `avc finetune <verb>`

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
        return Err(AvcError::Arg(
            "finetune start|list|show|publish|drift|run|report|cancel ...".into(),
        ));
    }

    let db = Db::open_default()?;
    let argv_ref: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();

    match argv_ref[0] {
        "list" => {
            let name = argv_ref
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("finetune list <persona>".into()))?;
            // 先验 persona 存在：未知 -> NotFound (exit 3)，与 iterate list / persona show 一致
            crate::svc::persona::get_persona(&db, name)?;
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
            for r in rows {
                out.push(r?);
            }
            print(mode, &out)?;
        }
        "show" => {
            // finetune show <fj_id> → fj 详情（base/target/scope/status/started_at/finished_at）
            let fj_id = argv_ref
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("finetune show <fj_id>".into()))?;
            let conn = db.conn.lock().unwrap();
            type FjRow = (
                String,
                String,
                i64,
                Option<i64>,
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
            );
            let row: Result<FjRow, _> =
                conn.query_row(
                    "SELECT fj.id, pm.name, fj.base_version, fj.target_version,
                            fj.status, fj.scope_json, fj.started_at, fj.finished_at, fj.drift_report_json
                     FROM finetune_jobs fj
                     JOIN persona_models pm ON pm.id = fj.persona_model_id
                     WHERE fj.id = ?",
                    [fj_id],
                    |r| {
                        Ok((
                            r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?,
                            r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?,
                        ))
                    },
                );
            drop(conn);
            let (id, name, base_v, target_v, status, scope, started, finished, drift_json) =
                row.map_err(|_| AvcError::NotFound(format!("finetune job '{}'", fj_id)))?;
            print(
                mode,
                &serde_json::json!({
                    "id": id,
                    "persona": name,
                    "base_version": base_v,
                    "target_version": target_v,
                    "status": status,
                    "scope": serde_json::from_str::<Vec<String>>(&scope).unwrap_or_default(),
                    "started_at": started,
                    "finished_at": finished,
                    "drift_report": drift_json.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()),
                }),
            )?;
        }
        "report" => {
            // finetune report <fj_id> → drift_report_json 单独输出（便于管道）
            let fj_id = argv_ref
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("finetune report <fj_id>".into()))?;
            let conn = db.conn.lock().unwrap();
            let drift_json: Option<String> = conn
                .query_row(
                    "SELECT drift_report_json FROM finetune_jobs WHERE id = ?",
                    [fj_id],
                    |r| r.get(0),
                )
                .map_err(|_| AvcError::NotFound(format!("finetune job '{}'", fj_id)))?;
            drop(conn);
            match drift_json {
                None => {
                    return Err(AvcError::Conflict(format!(
                        "finetune job '{}' has no drift_report (尚未 run 或 publish)",
                        fj_id
                    )));
                }
                Some(s) => {
                    let parsed: serde_json::Value =
                        serde_json::from_str(&s).unwrap_or(serde_json::Value::String(s));
                    print(mode, &parsed)?;
                }
            }
        }
        "cancel" => {
            // finetune cancel <fj_id> → 标 status='cancelled'（仅 queued/running；已
            // succeeded/failed_drift/cancelled 拒）；外部 vendor SFT 不会被本命令停，
            // 仅记账。需要外部 cleanup 时手动。
            let fj_id = argv_ref
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("finetune cancel <fj_id>".into()))?;
            let mut conn = db.conn.lock().unwrap();
            let tx = conn.transaction()?;
            let status: String = tx
                .query_row(
                    "SELECT status FROM finetune_jobs WHERE id = ?",
                    [fj_id],
                    |r| r.get(0),
                )
                .map_err(|_| AvcError::NotFound(format!("finetune job '{}'", fj_id)))?;
            if status == "succeeded" || status == "failed_drift" || status == "cancelled" {
                return Err(AvcError::Conflict(format!(
                    "finetune job '{}' is in '{}' state; cannot cancel",
                    fj_id, status
                )));
            }
            tx.execute(
                "UPDATE finetune_jobs SET status = 'cancelled', finished_at = ? WHERE id = ?",
                rusqlite::params![crate::svc::now_iso(), fj_id],
            )?;
            tx.commit()?;
            print(
                mode,
                &serde_json::json!({"finetune_job_id": fj_id, "status": "cancelled"}),
            )?;
        }
        "start" => {
            let name = argv_ref.get(1).copied().ok_or_else(|| {
                AvcError::Arg("finetune start <name> --scope voice --base-version <v>".into())
            })?;
            let mut base: Option<i64> = None;
            let mut scope: Vec<String> = vec!["voice".into()];
            let mut threshold: f32 = 0.85;
            let mut i = 2;
            while i < argv_ref.len() {
                match argv_ref[i] {
                    "--base-version" => {
                        base = argv_ref.get(i + 1).and_then(|s| s.parse().ok());
                        i += 2;
                    }
                    "--scope" => {
                        scope = argv_ref
                            .get(i + 1)
                            .map(|s| s.split(',').map(String::from).collect())
                            .unwrap_or(scope);
                        i += 2;
                    }
                    "--threshold" => {
                        threshold = argv_ref
                            .get(i + 1)
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(threshold);
                        i += 2;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            let base = base.ok_or_else(|| AvcError::Arg("--base-version 必填".into()))?;
            let cfg = crate::svc::finetune::FinetuneConfig {
                full_retrain: false,
                epochs: 1,
                consistency_threshold: threshold,
            };
            let fj_id = crate::svc::finetune::start(&db, name, &scope, base, &cfg)?;
            print(
                mode,
                &json!({
                    "finetune_job_id": fj_id,
                    "persona": name,
                    "base_version": base,
                    "target_version": base + 1,
                    "scope": scope,
                    "threshold": threshold,
                    "next_step": "Provider SFT 调用 (Phase 2+); 当前阶段需手动调 svc::finetune::publish(...)"
                }),
            )?;
        }
        "publish" => {
            // 测试用：模拟 drift 报告，强制通过/失败
            let fj_id = argv_ref.get(1).copied().ok_or_else(|| {
                AvcError::Arg("finetune publish <fj_id> --passed|--failed".into())
            })?;
            let passed = argv_ref.contains(&"--passed");
            let drift = crate::svc::finetune::DriftReport {
                face: 0.9,
                voice: 0.9,
                style: 0.9,
                avg: 0.9,
                passed,
            };
            crate::svc::finetune::publish(&db, fj_id, &drift)?;
            print(
                mode,
                &json!({"finetune_job_id": fj_id, "published": passed}),
            )?;
        }
        "drift" => {
            // finetune drift <fj_id> [--embed <name>] [--threshold <f>]
            // 不动 finetune_jobs.status；只读 base/target 的 voice_embed 算 cosine，
            // 并按需调 embed.<name>.embed() 真算 new_vec。
            let sub = argv_ref.get(1).copied().unwrap_or("");
            if sub != "eval" {
                return Err(AvcError::Arg("finetune drift eval <fj_id> ...".into()));
            }
            let fj_id = argv_ref
                .get(2)
                .copied()
                .ok_or_else(|| AvcError::Arg("finetune drift eval <fj_id>".into()))?;
            let mut embed_name: Option<&str> = None;
            let mut threshold: f32 = 0.85;
            let mut i = 3;
            while i < argv_ref.len() {
                match argv_ref[i] {
                    "--embed" => {
                        embed_name = argv_ref.get(i + 1).copied();
                        i += 2;
                    }
                    "--threshold" => {
                        threshold = argv_ref
                            .get(i + 1)
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(threshold);
                        i += 2;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            // 查 fj 拿到 base/target/name
            let (name, base_v, target_v): (String, i64, Option<i64>) = {
                let conn = db.conn.lock().unwrap();
                conn.query_row(
                    "SELECT pm.name, fj.base_version, fj.target_version
                     FROM finetune_jobs fj JOIN persona_models pm ON pm.id = fj.persona_model_id
                     WHERE fj.id = ?",
                    [fj_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(|_| AvcError::NotFound(format!("finetune job '{}'", fj_id)))?
            };
            let target_v = target_v.ok_or_else(|| {
                AvcError::Conflict(format!("finetune job '{}' has no target_version", fj_id))
            })?;
            // 先从 DB 取 base voice embed
            let base =
                crate::svc::drift::fetch_voice_embed(&db, &name, base_v)?.ok_or_else(|| {
                    AvcError::Conflict(format!(
                        "persona '{}' version {} has no voice_embed",
                        name, base_v
                    ))
                })?;
            let db_cosine =
                crate::svc::drift::eval_voice_from_db(&db, &name, base_v, target_v)?.cosine;
            // 若传 --embed，调 Provider 真算
            let provider_cosine = if let Some(ename) = embed_name {
                let cfg =
                    crate::config::Config::load(&crate::config::Config::default_config_path()?)?;
                let provider = crate::provider::real::make_embed(&cfg, ename)?;
                let seed_text = format!("persona:{}:target:{}", name, target_v);
                let seed = {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| AvcError::Internal(format!("drift tokio: {}", e)))?;
                    rt.block_on(async move {
                        provider
                            .embed(&[&seed_text])
                            .await
                            .map_err(|e| AvcError::ProviderUpstream(format!("embed eval: {}", e)))
                    })
                }?
                .into_iter()
                .next()
                .unwrap_or_default();
                crate::svc::drift::cosine_similarity(&base, &seed)
            } else {
                None
            };
            let reported_cosine = provider_cosine.or(db_cosine);
            let passed = reported_cosine.map(|c| c >= threshold).unwrap_or(false);
            let payload = json!({
                "finetune_job_id": fj_id,
                "persona": name,
                "base_version": base_v,
                "target_version": target_v,
                "embed_provider": embed_name,
                "threshold": threshold,
                "cosine_db": db_cosine,
                "cosine_provider": provider_cosine,
                "passed": passed,
            });
            print(mode, &payload)?;
            if !passed {
                return Err(AvcError::Conflict(format!(
                    "drift not passed: cosine={:?} < threshold={}",
                    reported_cosine, threshold
                )));
            }
        }
        "run" => {
            // finetune run <fj_id> [--embed <name>]
            // 端到端：调 voice/avatar Provider SFT → 算 voice drift → publish/rollback
            let fj_id = argv_ref
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("finetune run <fj_id> [--embed <name>]".into()))?;
            let mut embed_name: Option<&str> = None;
            let mut i = 2;
            while i < argv_ref.len() {
                match argv_ref[i] {
                    "--embed" => {
                        embed_name = argv_ref.get(i + 1).copied();
                        i += 2;
                    }
                    _ => {
                        return Err(AvcError::Arg(format!(
                            "finetune run: unknown flag '{}'",
                            argv_ref[i]
                        )));
                    }
                }
            }
            let cfg = crate::config::Config::load(&crate::config::Config::default_config_path()?)?;
            let report = crate::svc::finetune::run(&db, &cfg, fj_id, embed_name)?;
            // 状态语义：succeeded → exit 0；failed_drift → exit 4 (Conflict)
            // CLI 表面把 provider 错误（network/binary）原样返 ProviderUpstream 退出码。
            if report.status == "failed_drift" {
                print(
                    mode,
                    &serde_json::json!({
                        "finetune_job_id": report.finetune_job_id,
                        "persona": report.persona,
                        "base_version": report.base_version,
                        "target_version": report.target_version,
                        "scopes_processed": report.scopes_processed,
                        "status": report.status,
                        "voice_cosine": report.voice_cosine,
                        "face_cosine": report.face_cosine,
                        "style_cosine": report.style_cosine,
                        "threshold": report.threshold,
                        "samples_used": report.samples_used,
                    }),
                )?;
                return Err(AvcError::Conflict(format!(
                    "drift below threshold: voice={:?} face={:?} style={:?} < {}",
                    report.voice_cosine, report.face_cosine, report.style_cosine, report.threshold
                )));
            }
            print(
                mode,
                &serde_json::json!({
                    "finetune_job_id": report.finetune_job_id,
                    "persona": report.persona,
                    "base_version": report.base_version,
                    "target_version": report.target_version,
                    "scopes_processed": report.scopes_processed,
                    "status": report.status,
                    "voice_cosine": report.voice_cosine,
                    "face_cosine": report.face_cosine,
                    "style_cosine": report.style_cosine,
                    "threshold": report.threshold,
                    "samples_used": report.samples_used,
                    "next_step": "avc persona set-current <name> --to <target> or render run with --version <target>",
                }),
            )?;
        }
        _ => {
            return Err(AvcError::Arg(format!(
                "finetune: unknown verb '{}'",
                argv_ref[0]
            )))
        }
    }
    Ok(())
}
