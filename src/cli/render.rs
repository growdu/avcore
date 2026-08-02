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
            // 解析 --persona / --version / --topic / --*-provider
            // - 每个 value-required flag 后必须紧跟一个非空 argv 元素；末尾空 flag
            //   必须以 AvcError::Arg 拒绝（exit 2），避免静默执行整个 render DAG。
            // - 解析器使用 `argv[i+1]` 的存在性 + 非空检查（trim 后非空）；
            //   传入 "--" 这种纯占位也按缺失处理。
            // - 任何 token 以 '-' 开头但 *不是* 已知 flag（拼错的 provider 名 /
            //   完全未知 flag）必须以 AvcError::Arg 拒绝，避免 `--llm-providr mock`
            //   这种 typo 静默跑默认 provider。
            // - 已知 flag 的 *值* 即使以 '-' 开头（如 provider 名 `--weird`）也
            //   按原样接受——这是常规 CLI 行为，避免误伤合法 invocation。
            // - 解析完成时若还有未消费的 token（非已知 flag 且不以 '-' 开头），
            //   视为多余 positional token 拒绝（不再用 `_ => i += 1` 静默吞）。
            let mut persona: Option<String> = None;
            let mut version: Option<i64> = None;
            let mut topic: Option<String> = None;
            let mut llm_provider: Option<String> = None;
            let mut voice_provider: Option<String> = None;
            let mut avatar_provider: Option<String> = None;
            let mut video_provider: Option<String> = None;
            let mut i = 1;
            while i < argv.len() {
                let next = |i: usize| -> Option<String> {
                    argv.get(i + 1)
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                };
                match argv[i].as_str() {
                    "--persona" => {
                        persona = Some(require_value("--persona", next(i))?);
                        i += 2;
                    }
                    "--version" => {
                        let raw = require_value("--version", next(i))?;
                        version = Some(raw.parse().map_err(|_| {
                            AvcError::Arg(format!("--version: 需整数，got '{}'", raw))
                        })?);
                        i += 2;
                    }
                    "--topic" => {
                        topic = Some(require_value("--topic", next(i))?);
                        i += 2;
                    }
                    "--llm-provider" => {
                        llm_provider = Some(require_value("--llm-provider", next(i))?);
                        i += 2;
                    }
                    "--voice-provider" => {
                        voice_provider = Some(require_value("--voice-provider", next(i))?);
                        i += 2;
                    }
                    "--avatar-provider" => {
                        avatar_provider = Some(require_value("--avatar-provider", next(i))?);
                        i += 2;
                    }
                    "--video-provider" => {
                        video_provider = Some(require_value("--video-provider", next(i))?);
                        i += 2;
                    }
                    // 全局输出模式 flag：在 dispatch() 顶部已被消费，
                    // 这里显式跳过，避免误判为未知 flag。
                    "--json" | "--quiet" => {
                        i += 1;
                    }
                    tok if tok.starts_with('-') => {
                        // 拼错 / 未知的 flag：显式拒绝，stderr 命名该 token。
                        return Err(AvcError::Arg(format!(
                            "render run: 未知选项 '{}'；用法：avc render run --persona <name> [--version <n>] [--topic <text>] [--llm-provider <name>] [--voice-provider <name>] [--avatar-provider <name>] [--video-provider <name>]",
                            tok
                        )));
                    }
                    _ => {
                        // 非 '-' 开头的剩余 token = 多余 positional。
                        return Err(AvcError::Arg(format!(
                            "render run: 不接受位置参数 '{}'；用法：avc render run --persona <name> [...]",
                            argv[i]
                        )));
                    }
                }
            }
            let persona = persona.ok_or_else(|| AvcError::Arg("--persona <name> 必填".into()))?;
            let topic = topic.unwrap_or_else(|| "(no topic)".to_string());
            // version 不指定 = current
            let v = match version {
                Some(v) => v,
                None => {
                    let p = crate::svc::persona::get_persona(&db, &persona)?;
                    p.current_version
                }
            };
            let job_id = crate::svc::render::create_job(&db, &persona, v, &topic)?;
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
            crate::svc::pipeline::run(&db, &job_id, &spec, &topic)?;
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
            // - value-required flag 末尾空值必须以 AvcError::Arg 拒绝（exit 2），
            //   避免 `--topics-file /tmp/x --version` 静默跑整个 pack。
            // - 任何 token 以 '-' 开头但 *不是* 已知 flag（拼错的 flag 名 /
            //   完全未知 flag）必须以 AvcError::Arg 拒绝；同样 persona 之后
            //   出现多个非 '-' 前缀的 positional 视为多余 token 拒绝（不再
            //   用 `_ => i += 1` 静默吞）。
            let persona = argv
                .get(1)
                .ok_or_else(|| AvcError::Arg("render pack <persona> --topics-file <path>".into()))?
                .clone();
            let mut topics_file: Option<String> = None;
            let mut version: Option<i64> = None;
            let mut i = 2;
            while i < argv.len() {
                let next = |i: usize| -> Option<String> {
                    argv.get(i + 1)
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                };
                match argv[i].as_str() {
                    "--topics-file" => {
                        topics_file = Some(require_value("--topics-file", next(i))?);
                        i += 2;
                    }
                    "--version" => {
                        let raw = require_value("--version", next(i))?;
                        version = Some(raw.parse().map_err(|_| {
                            AvcError::Arg(format!("--version: 需整数，got '{}'", raw))
                        })?);
                        i += 2;
                    }
                    // 全局输出模式 flag：在 dispatch() 顶部已被消费，
                    // 这里显式跳过，避免误判为未知 flag。
                    "--json" | "--quiet" => {
                        i += 1;
                    }
                    tok if tok.starts_with('-') => {
                        return Err(AvcError::Arg(format!(
                            "render pack: 未知选项 '{}'；用法：avc render pack <persona> --topics-file <path> [--version <n>]",
                            tok
                        )));
                    }
                    _ => {
                        return Err(AvcError::Arg(format!(
                            "render pack: 不接受额外位置参数 '{}'；用法：avc render pack <persona> --topics-file <path> [...]",
                            argv[i]
                        )));
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

/// value-required CLI flag 的统一校验器：
/// - `next(i)` 为 `None` 或 trim 后为空 → `AvcError::Arg` 含 flag 名；
/// - 否则原样返回 trimmed 值（不规范化空白，让调用方拿到原始输入以便错误信息）。
/// - 失败 exit 2，避免静默执行后续重操作（render DAG / DB 写入）。
fn require_value(flag: &str, next: Option<String>) -> AvcResult<String> {
    match next {
        Some(v) if !v.is_empty() => Ok(v),
        _ => Err(AvcError::Arg(format!(
            "{} 需要值；用法：{} <value>",
            flag, flag
        ))),
    }
}
