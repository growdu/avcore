//! ask 模式：非交互式 NL
//!
//! Phase 1：仅检测 NL 模型是否配置，未配置时明确报错。
//! 完整设计见 docs/shell.md §4.11。


use std::io::IsTerminal;

use crate::AvcError;
use crate::AvcResult;
use crate::config::Config;

pub fn run(args: &[String]) -> AvcResult<()> {
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
    let has_llm = !cfg.provider.llm.is_empty();
    if !has_llm {
        return Err(AvcError::NlModelMissing(
            "未配置 provider.llm.* ，无法做 NL 解析；可直接用原子命令".into(),
        ));
    }

    if dry_run {
        println!("[dry-run] would plan: {}", nl);
        return Ok(());
    }

    if !yes && !std::io::stdout().is_terminal() {
        return Err(AvcError::Arg(
            "非 TTY 下默认要求 --yes（避免脚本意外执行写操作）".into(),
        ));
    }

    // Phase 1 占位：拿到 LLM 也只是 echo
    println!("[ask] phase-1 stub: input={}", nl);
    println!("hint: 配置 provider.llm 后可启用真 NL 解析；当前阶段请用原子命令。");
    if json {
        println!("{{\"input\": {:?}, \"phase\": 1}}", nl);
    }

    let _ = yes; // suppress
    Ok(())
}

