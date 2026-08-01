//! ask 模式：非交互式 NL
//!
//! Phase 1 起：拿到配置的 `provider.llm.<name>` 后，真发请求到 OpenAI 兼容 chat
//! 端点，回显 assistant 回复。
//!
//! 安全姿态（持续到 Phase 1+）：
//! - 默认**只读**：把 NL 当 user message 发出去；不回写 SQLite、不动 persona。
//! - 含创建/删除/微调/出片关键词的输入，在非 TTY 下必须 `--yes`，否则拒绝。
//! - 完整 NL→原子计划 → 自动执行仍属 Phase 2+（独立 plan），不在本路径。

use std::io::IsTerminal;

use crate::AvcError;
use crate::AvcResult;
use crate::config::Config;
use crate::provider::real::make_llm;
use crate::provider::ChatMessage;

pub fn run(args: &[String]) -> AvcResult<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AvcError::Internal(format!("ask tokio: {}", e)))?;
    rt.block_on(run_async(args))
}

async fn run_async(args: &[String]) -> AvcResult<()> {
    // 跳过 argv[0]=avc, argv[1]=ask
    let argv: Vec<&str> = args.iter().skip(2).map(|s| s.as_str()).collect();

    if argv.is_empty() {
        return Err(AvcError::Arg("avc ask \"...\"".into()));
    }

    // 解析 flags
    let mut nl: Option<&str> = None;
    let mut json = false;
    let mut dry_run = false;
    let mut yes = false;
    for a in &argv {
        match *a {
            "--json" => json = true,
            "--dry-run" => dry_run = true,
            "--yes" | "-y" => yes = true,
            other if !other.starts_with("--") => {
                if nl.is_none() {
                    nl = Some(other);
                }
            }
            _ => {}
        }
    }

    let nl = nl.ok_or_else(|| AvcError::Arg("缺少自然语言输入".into()))?;

    let cfg = Config::load(&Config::default_config_path()?)?;
    if cfg.provider.llm.is_empty() {
        return Err(AvcError::NlModelMissing(
            "未配置 provider.llm.* ，无法做 NL 解析；可直接用原子命令".into(),
        ));
    }

    // 安全姿态：含写操作关键词 + 非 TTY + 无 --yes → 拒绝。
    let low = nl.to_lowercase();
    let looks_writing = low.contains("create ")
        || low.contains("delete")
        || low.contains("finetune")
        || low.contains("render run")
        || low.contains("archive ")
        || low.contains("set-traits")
        || low.contains("set-catchphrase")
        || low.contains("commit ");
    if looks_writing && !yes && !std::io::stdout().is_terminal() {
        return Err(AvcError::Arg(
            "非 TTY 下默认要求 --yes（避免脚本意外执行写操作）".into(),
        ));
    }

    // 选定默认 llm provider：取第一个 key（Phase 1 取首个；Phase 1+ 加选定逻辑）
    let provider_name = cfg
        .provider
        .llm
        .keys()
        .next()
        .expect("provider.llm 非空（已校验）")
        .clone();
    let llm = make_llm(&cfg, &provider_name)?;

    let msgs = vec![ChatMessage {
        role: "user".into(),
        content: nl.to_string(),
    }];

    if dry_run {
        println!("[dry-run] would send to {}: {}", provider_name, nl);
        return Ok(());
    }

    let reply = llm.chat(&msgs).await?;

    if json {
        let v = serde_json::json!({
            "input": nl,
            "provider": provider_name,
            "reply": reply,
            "phase": 1,
        });
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!("[ask] provider={}", provider_name);
        println!("{}", reply);
        println!("hint: 本阶段仅 echo LLM 回复；不自动执行写操作。");
    }

    let _ = yes; // suppress
    Ok(())
}
