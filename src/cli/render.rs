//! `avc render <verb>`

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
        return Err(AvcError::Arg("render script|video|run ...".into()));
    }

    let db = Db::open_default()?;
    match argv[0].as_str() {
        "run" => {
            // 解析 --persona / --version / --topic
            let mut persona = None;
            let mut version: Option<i64> = None;
            let mut topic: Option<&str> = None;
            let mut llm_provider: Option<String> = None;
            let mut voice_provider: Option<String> = None;
            let mut avatar_provider: Option<String> = None;
            let mut video_provider: Option<String> = None;
            let mut i = 1;
            while i < argv.len() {
                match argv[i].as_str() {
                    "--persona" => {
                        persona = argv.get(i + 1).map(|s| s.as_str());
                        i += 2;
                    }
                    "--version" => {
                        version = argv.get(i + 1).and_then(|s| s.parse().ok());
                        i += 2;
                    }
                    "--topic" => {
                        topic = argv.get(i + 1).map(|s| s.as_str());
                        i += 2;
                    }
                    "--llm-provider" => {
                        llm_provider = argv.get(i + 1).cloned();
                        i += 2;
                    }
                    "--voice-provider" => {
                        voice_provider = argv.get(i + 1).cloned();
                        i += 2;
                    }
                    "--avatar-provider" => {
                        avatar_provider = argv.get(i + 1).cloned();
                        i += 2;
                    }
                    "--video-provider" => {
                        video_provider = argv.get(i + 1).cloned();
                        i += 2;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            let persona = persona.ok_or_else(|| AvcError::Arg("--persona <name> 必填".into()))?;
            let topic = topic.unwrap_or("(no topic)");
            // version 不指定 = current
            let v = match version {
                Some(v) => v,
                None => {
                    let p = crate::svc::persona::get_persona(&db, persona)?;
                    p.current_version
                }
            };
            let job_id = crate::svc::render::create_job(&db, persona, v, topic)?;
            // Wave B：render run 真跑 DAG 五节点 (script_gen → tts+img_gen → i2v → compose)
            // 节点 BLOB 落 artifacts 表；失败 → job status='failed' + error_json。
            let mut spec = crate::svc::pipeline::render_publishment_spec();
            if let Some(provider) = llm_provider {
                spec.nodes[0].config["llm_provider"] = serde_json::Value::String(provider);
            }
            if let Some(provider) = voice_provider {
                spec.nodes[1].config["voice_provider"] = serde_json::Value::String(provider);
            }
            if let Some(provider) = avatar_provider {
                spec.nodes[2].config["avatar_provider"] = serde_json::Value::String(provider);
            }
            if let Some(provider) = video_provider {
                spec.nodes[3].config["video_provider"] = serde_json::Value::String(provider);
            }
            crate::svc::pipeline::run(&db, &job_id, &spec, topic)?;
            if mode == OutputMode::Quiet {
                println!("{}", job_id);
            } else {
                print(mode, &serde_json::json!({"job_id": job_id}))?;
            }
        }
        "pack" => {
            // avc render pack <persona> --topics-file <path> [--version <n>]
            // - topics-file: 每行一个 topic（`#` 开头 / 空行 跳过）
            // - 默认 version = current；可手动覆盖
            let persona = argv
                .get(1)
                .ok_or_else(|| AvcError::Arg("render pack <persona> --topics-file <path>".into()))?
                .clone();
            let mut topics_file: Option<String> = None;
            let mut version: Option<i64> = None;
            let mut i = 2;
            while i < argv.len() {
                match argv[i].as_str() {
                    "--topics-file" => {
                        topics_file = argv.get(i + 1).cloned();
                        i += 2;
                    }
                    "--version" => {
                        version = argv.get(i + 1).and_then(|s| s.parse().ok());
                        i += 2;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            let tf = topics_file.ok_or_else(|| {
                AvcError::Arg("render pack: --topics-file <path> required".into())
            })?;
            let (job_ids, errors) =
                crate::svc::render::pack(&db, &persona, version, std::path::Path::new(&tf))?;
            let errs_json: Vec<serde_json::Value> = errors
                .iter()
                .map(|(t, e)| serde_json::json!({"topic": t, "error": e}))
                .collect();
            print(
                mode,
                &serde_json::json!({
                    "persona": persona,
                    "version": version,
                    "topics_file": tf,
                    "jobs": job_ids,
                    "job_count": job_ids.len(),
                    "failed_count": errors.len(),
                    "errors": errs_json,
                }),
            )?;
            // 任一失败 → exit 4 (Conflict)，让 CI/script 能探测
            if !errors.is_empty() {
                std::process::exit(4);
            }
        }
        _ => return Err(AvcError::Arg(format!("render: unknown verb '{}'", argv[0]))),
    }
    Ok(())
}
