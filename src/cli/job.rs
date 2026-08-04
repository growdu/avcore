//! `avc job <verb>`
use rusqlite::OptionalExtension;

use crate::db::Db;
use crate::output::{print, OutputMode};
use crate::AvcError;
use crate::AvcResult;

pub fn dispatch(argv: &[String]) -> AvcResult<()> {
    let mode = OutputMode::from_flags(
        argv.iter().any(|a| a == "--json"),
        argv.iter().any(|a| a == "--quiet"),
    );

    if argv.is_empty() {
        return Err(AvcError::Arg(
            "job list|show|export|feedback|wait|cancel ...".into(),
        ));
    }

    let db = Db::open_default()?;
    match argv[0].as_str() {
        "list" => {
            let name = argv
                .get(1)
                .ok_or_else(|| AvcError::Arg("job list <persona>".into()))?;
            let ids = crate::svc::render::list_jobs(&db, name)?;
            print(mode, &ids)?;
        }
        "show" => {
            let id = argv
                .get(1)
                .ok_or_else(|| AvcError::Arg("job show <job_id> [--artifacts]".into()))?;
            let want_artifacts = argv.iter().any(|a| a == "--artifacts");
            if want_artifacts {
                let arts = crate::svc::render::list_artifacts(&db, id)?;
                let arr: Vec<serde_json::Value> = arts
                    .iter()
                    .map(|a| {
                        serde_json::json!({
                            "kind": a.0,
                            "name": a.1,
                            "byte_size": a.2,
                            "mime": a.3,
                        })
                    })
                    .collect();
                print(mode, &serde_json::json!({"id": id, "artifacts": arr}))?;
            } else {
                let status = crate::svc::render::get_job(&db, id)?;
                print(mode, &serde_json::json!({"id": id, "status": status}))?;
            }
        }
        "wait" => {
            // job wait <job_id> --until <status> [--timeout <secs>] [--poll <ms>]
            // 阻塞轮询 jobs.status，直到等于 --until 指定的 status（succeeded / failed /
            // cancelled）。缺省 timeout=600s、poll=500ms。timeout 超时 → exit 4 (Conflict)，
            // CI 探测友好。
            let id = argv
                .get(1)
                .ok_or_else(|| AvcError::Arg("job wait <job_id> --until <status>".into()))?;
            let mut until: Option<String> = None;
            let mut timeout_secs: u64 = 600;
            let mut poll_ms: u64 = 500;
            let mut i = 2;
            while i < argv.len() {
                match argv[i].as_str() {
                    "--until" => {
                        until = argv.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    "--timeout" => {
                        timeout_secs = argv
                            .get(i + 1)
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(timeout_secs);
                        i += 2;
                    }
                    "--poll" => {
                        poll_ms = argv
                            .get(i + 1)
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(poll_ms);
                        i += 2;
                    }
                    _ => {
                        return Err(AvcError::Arg(format!(
                            "job wait: unknown flag '{}'",
                            argv[i]
                        )));
                    }
                }
            }
            let until =
                until.ok_or_else(|| AvcError::Arg("job wait: --until <status> required".into()))?;
            // 校验 job 存在
            let exists: bool = {
                let conn = db.conn.lock().unwrap();
                conn.query_row("SELECT 1 FROM jobs WHERE id = ?", [id], |r| {
                    r.get::<_, i64>(0).map(|_| true)
                })
                .optional()?
                .unwrap_or(false)
            };
            if !exists {
                return Err(AvcError::NotFound(format!("job '{}'", id)));
            }
            let started = std::time::Instant::now();
            let timeout = std::time::Duration::from_secs(timeout_secs);
            let poll = std::time::Duration::from_millis(poll_ms);
            loop {
                let status: String = {
                    let conn = db.conn.lock().unwrap();
                    conn.query_row("SELECT status FROM jobs WHERE id = ?", [id], |r| r.get(0))?
                };
                if status == until {
                    print(
                        mode,
                        &serde_json::json!({
                            "job_id": id,
                            "status": status,
                            "elapsed_secs": started.elapsed().as_secs(),
                        }),
                    )?;
                    return Ok(());
                }
                if status == "failed" || status == "cancelled" {
                    // 等的是 succeeded 但实际 failed → 退出 4
                    print(
                        mode,
                        &serde_json::json!({
                            "job_id": id,
                            "status": status,
                            "elapsed_secs": started.elapsed().as_secs(),
                        }),
                    )?;
                    return Err(AvcError::Conflict(format!(
                        "job '{}' reached terminal state '{}' (expected '{}')",
                        id, status, until
                    )));
                }
                if started.elapsed() > timeout {
                    return Err(AvcError::Conflict(format!(
                        "job wait: timeout after {}s (status still '{}', expected '{}')",
                        timeout_secs, status, until
                    )));
                }
                std::thread::sleep(poll);
            }
        }
        "cancel" => {
            // job cancel <job_id> → 标 jobs.status='cancelled'（仅 queued；succeeded/failed/running/cancelled 拒）。
            // running 状态的 job 不主动停（vendor DAG 跑在 tokio runtime 里），由用户手工收尾。
            let id = argv
                .get(1)
                .ok_or_else(|| AvcError::Arg("job cancel <job_id>".into()))?;
            let mut conn = db.conn.lock().unwrap();
            let tx = conn.transaction()?;
            let status: String = tx
                .query_row("SELECT status FROM jobs WHERE id = ?", [id], |r| r.get(0))
                .map_err(|_| AvcError::NotFound(format!("job '{}'", id)))?;
            if status != "queued" {
                return Err(AvcError::Conflict(format!(
                    "job '{}' is in '{}' state; only 'queued' is cancellable",
                    id, status
                )));
            }
            tx.execute(
                "UPDATE jobs SET status = 'cancelled', finished_at = ? WHERE id = ?",
                rusqlite::params![crate::svc::now_iso(), id],
            )?;
            tx.commit()?;
            print(
                mode,
                &serde_json::json!({"job_id": id, "status": "cancelled"}),
            )?;
        }
        "export" => {
            let id = argv.get(1).ok_or_else(|| {
                AvcError::Arg(
                    "job export <job_id> --out <dir> | --target s3://bucket/prefix/".into(),
                )
            })?;
            // 找 --out 或 --target 后的值（互斥）
            let out_idx = argv.iter().position(|a| a == "--out");
            let target_idx = argv.iter().position(|a| a == "--target");
            if out_idx.is_some() && target_idx.is_some() {
                return Err(AvcError::Arg(
                    "job export: --out and --target are mutually exclusive".into(),
                ));
            }
            let (count, bytes, target_label) = if let Some(i) = out_idx {
                let out = argv
                    .get(i + 1)
                    .ok_or_else(|| AvcError::Arg("job export: --out <dir> required".into()))?;
                let out_path = std::path::Path::new(out);
                let (c, b) = crate::svc::render::export_artifacts(
                    &db,
                    id,
                    crate::svc::render::ExportTarget::Local(out_path),
                )?;
                (c, b, out.to_string())
            } else if let Some(i) = target_idx {
                // --target s3://bucket/prefix/  拆 bucket / prefix + 从 config 拿 upload_cmd
                let raw = argv.get(i + 1).ok_or_else(|| {
                    AvcError::Arg("job export: --target s3://bucket/prefix/ required".into())
                })?;
                let (bucket, prefix, upload_cmd) = parse_s3_target(raw)?;
                let (c, b) = crate::svc::render::export_artifacts(
                    &db,
                    id,
                    crate::svc::render::ExportTarget::S3 {
                        bucket: &bucket,
                        prefix: &prefix,
                        upload_cmd: &upload_cmd,
                    },
                )?;
                (c, b, raw.to_string())
            } else {
                return Err(AvcError::Arg(
                    "job export: --out <dir> OR --target s3://bucket/prefix/ required".into(),
                ));
            };
            print(
                mode,
                &serde_json::json!({
                    "job_id": id,
                    "target": target_label,
                    "files": count,
                    "bytes": bytes,
                }),
            )?;
        }
        "feedback" => {
            let id = argv.get(1).ok_or_else(|| {
                AvcError::Arg("job feedback <job_id> --looks-unlike [--reason <text>]".into())
            })?;
            let looks_unlike = argv.iter().any(|a| a == "--looks-unlike");
            let reason = argv
                .iter()
                .position(|a| a == "--reason")
                .and_then(|i| argv.get(i + 1))
                .map(|s| s.as_str());
            let sample_id = crate::svc::render::feedback(&db, id, looks_unlike, reason)?;
            print(
                mode,
                &serde_json::json!({
                    "job_id": id,
                    "sample_id": sample_id,
                    "looks_unlike": looks_unlike,
                    "reason": reason,
                }),
            )?;
        }
        _ => return Err(AvcError::Arg(format!("job: unknown verb '{}'", argv[0]))),
    }
    Ok(())
}

/// 解析 `s3://bucket/prefix/`，返 (bucket, prefix, upload_cmd)。
///
/// upload_cmd 来自 `[export.s3]` config 段：缺省 `aws s3 cp --region us-east-1
/// {local} s3://{bucket}/{prefix}{name}`（占位符在 svc::render::export_artifacts
/// 内替换）。prefix 以 `/` 结尾；空 prefix 合法。
fn parse_s3_target(raw: &str) -> AvcResult<(String, String, String)> {
    let stripped = raw
        .strip_prefix("s3://")
        .ok_or_else(|| AvcError::Arg(format!("--target must start with s3://: got {}", raw)))?;
    let (bucket, rest) = stripped
        .split_once('/')
        .ok_or_else(|| AvcError::Arg(format!("--target s3://<bucket>/<prefix>/: got {}", raw)))?;
    if bucket.is_empty() {
        return Err(AvcError::Arg(format!("--target: empty bucket: {}", raw)));
    }
    // prefix：保留开头和结尾的 /，但去掉多余
    let prefix = rest.to_string();
    let cfg = crate::config::Config::load(&crate::config::Config::default_config_path()?)?;
    let upload_cmd = cfg
        .export
        .as_ref()
        .and_then(|e| e.s3.as_ref())
        .map(|s| s.upload_cmd.clone())
        .unwrap_or_else(|| {
            "aws s3 cp --region us-east-1 {local} s3://{bucket}/{prefix}{name}".to_string()
        });
    Ok((bucket.to_string(), prefix, upload_cmd))
}
