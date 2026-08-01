//! `avc render <verb>`

use crate::AvcError;
use crate::AvcResult;
use crate::db::Db;
use crate::output::{print, OutputMode};

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
            let mut i = 1;
            while i < argv.len() {
                match argv[i].as_str() {
                    "--persona" => { persona = argv.get(i+1).map(|s| s.as_str()); i += 2; }
                    "--version" => {
                        version = argv.get(i+1).and_then(|s| s.parse().ok());
                        i += 2;
                    }
                    "--topic" => { topic = argv.get(i+1).map(|s| s.as_str()); i += 2; }
                    _ => { i += 1; }
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
            let spec = crate::svc::pipeline::render_publishment_spec();
            if let Err(e) = crate::svc::pipeline::run(&db, &job_id, &spec, topic) {
                // 打印到 stderr 但不 exit — 仍返回 job_id，让调用方决定查询 status。
                eprintln!("error: pipeline failed: {}", e);
                let _ = e; // suppress unused
            }
            if mode == OutputMode::Quiet {
                println!("{}", job_id);
            } else {
                print(mode, &serde_json::json!({"job_id": job_id}))?;
            }
        }
        _ => return Err(AvcError::Arg(format!("render: unknown verb '{}'", argv[0]))),
    }
    Ok(())
}
