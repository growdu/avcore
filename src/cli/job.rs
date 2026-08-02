//! `avc job <verb>`

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
        return Err(AvcError::Arg("job list|show|export|feedback ...".into()));
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
        "export" => {
            let id = argv
                .get(1)
                .ok_or_else(|| AvcError::Arg("job export <job_id> --out <dir>".into()))?;
            // 找 --out 后的值
            let out_idx = argv
                .iter()
                .position(|a| a == "--out")
                .ok_or_else(|| AvcError::Arg("job export: --out <dir> required".into()))?;
            let out = argv
                .get(out_idx + 1)
                .ok_or_else(|| AvcError::Arg("job export: --out <dir> required".into()))?;
            let out_path = std::path::Path::new(out);
            let (count, bytes) = crate::svc::render::export_artifacts(&db, id, out_path)?;
            print(
                mode,
                &serde_json::json!({
                    "job_id": id,
                    "out": out,
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
