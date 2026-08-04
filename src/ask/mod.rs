//! ask 模式：非交互式 NL → 原子计划 → 执行
//!
//! Phase 1.3+ 形态：把 NL 当 user message 发往配置的 LLM，要求 LLM 返标准 JSON plan，
//! 验证 plan 后执行。只读 plan 自动跑；写 plan 在 TTY 下询问 y/n，缺 `--yes` 时在非 TTY
//! 下拒绝。
//!
//! 支持的原子在 `ATOM_CATALOG` 列出（plan 阶段会传给 LLM 作 system prompt 之外的安全约束）：
//! read_only: persona list / show / versions / iter-assets inspection
//! write:     persona set-traits / set-catchphrase / set-render / commit / promote
//!
//! 不在 catalogue 的 cmd 不执行；plan parser 会把它标 unknown，CLI 直接报错。

use std::io::IsTerminal;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::provider::real::make_llm;
use crate::provider::ChatMessage;
use crate::AvcError;
use crate::AvcResult;

/// Allow list of write-action verbs that ask can dispatch.
const WRITE_VERBS: &[&str] = &[
    "set-traits",
    "set-catchphrase",
    "set-render",
    "commit",
    "promote",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub cmd: String,
    #[serde(default)]
    pub args: serde_json::Value,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    #[serde(default)]
    pub intent: String,
    pub steps: Vec<PlanStep>,
    #[serde(default)]
    pub read_only: bool,
}

pub fn run(args: &[String]) -> AvcResult<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AvcError::Internal(format!("ask tokio: {}", e)))?;
    rt.block_on(run_async(args))
}

/// NL 直管入口：接受原始自然语言，返回同步结果。
/// Shell 模式用：直接构造 argv + 调 `run(&[avc, ask, --yes, nl])`，
/// 这样能复用 plan 流水线的所有副作用（BLOB / 错误码 / exit code）。
/// 本函数保留供将来 inline 用（如 Ncurses 风格 nl-input 或 fzf picker）。
///
/// 在 TTY 下若 plan 含 write step 仍走 stdin read y/n；可传 `auto_yes` 跳过。
pub fn dispatch_nl(nl: &str, auto_yes: bool) -> AvcResult<()> {
    let mut argv: Vec<String> = vec!["avc".into(), "ask".into()];
    if auto_yes {
        argv.push("--yes".into());
    }
    argv.push(nl.to_string());
    run(&argv)
}

const SYSTEM_PROMPT: &str = r#"你是 avc CLI 的命令规划器。把用户自然语言翻译成 plan JSON。

规则：
1. 输出严格 JSON，不要任何额外文本，不要 markdown 围栏。
2. shape: {"intent":"...", "read_only":bool, "steps":[{"cmd":"<noun> <verb>","args":{...},"reason":"..."}]}
3. 优先用 read_only 路径（persona list/show/versions），不要把"看"当"做"。
4. 写操作必须确实必要；非必要不要 emit write steps。
5. 只使用下列原子（不知道就返回 steps:[] + read_only=true + intent="unknown"）：
   read_only:
     - persona list
     - persona show <name>
     - persona versions <name>
   write (需 set-traits / set-catchphrase / set-render / commit / promote):
     - persona set-traits <name> --version <v> --traits <csv>
     - persona set-catchphrase <name> --version <v> --add <s> | --remove <s>
     - persona set-render <name> --version <v> --resolution <p>
6. 不解析 finetune / render run / archive / delete（这些不在 ask 范围）。"#;

async fn run_async(args: &[String]) -> AvcResult<()> {
    let argv: Vec<&str> = args.iter().skip(2).map(|s| s.as_str()).collect();

    if argv.is_empty() {
        return Err(AvcError::Arg("avc ask \"...\"".into()));
    }

    let mut nl: Option<&str> = None;
    let mut json = false;
    let mut dry_run = false;
    let mut yes = false;
    for a in &argv {
        match *a {
            "--json" => json = true,
            "--dry-run" => dry_run = true,
            "--yes" | "-y" => yes = true,
            other if !other.starts_with("--") && nl.is_none() => {
                nl = Some(other);
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

    let provider_name = cfg
        .provider
        .llm
        .keys()
        .next()
        .expect("provider.llm 非空（已校验）")
        .clone();
    let llm = make_llm(&cfg, &provider_name)?;

    let msgs = vec![
        ChatMessage {
            role: "system".into(),
            content: SYSTEM_PROMPT.into(),
        },
        ChatMessage {
            role: "user".into(),
            content: nl.to_string(),
        },
    ];

    let raw_reply = llm.chat(&msgs).await?;

    // 尝试解析 plan
    let plan = match parse_plan(&raw_reply) {
        Ok(p) => p,
        Err(e) => {
            // 解析失败：原样回显 LLM 内容（已具有 read_only 静默安全姿态）
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "input": nl,
                        "provider": provider_name,
                        "parse_error": e.to_string(),
                        "raw_reply": raw_reply,
                    }))
                    .unwrap()
                );
            } else {
                println!("[ask] provider={}", provider_name);
                println!("{}", raw_reply);
                println!("hint: LLM 输出无法解析为 plan JSON；以上为原始回显。");
            }
            return Ok(());
        }
    };

    if dry_run {
        println!(
            "[dry-run] intent={} read_only={} steps={}",
            plan.intent,
            plan.read_only,
            plan.steps.len()
        );
        for (i, s) in plan.steps.iter().enumerate() {
            println!(
                "  {}. {} {}",
                i + 1,
                s.cmd,
                serde_json::to_string(&s.args).unwrap_or_default()
            );
            if let Some(r) = &s.reason {
                println!("     reason: {r}");
            }
        }
        return Ok(());
    }

    if plan.steps.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "input": nl, "provider": provider_name,
                    "intent": plan.intent, "plan": [], "read_only": plan.read_only,
                }))
                .unwrap()
            );
        } else {
            println!("[ask] provider={}", provider_name);
            println!(
                "intent: {} (read_only={}, no steps)",
                plan.intent, plan.read_only
            );
        }
        return Ok(());
    }

    // 验证 plan + 确认
    for s in &plan.steps {
        validate_step(s)?;
    }
    let has_write = plan.steps.iter().any(|s| is_write_cmd(&s.cmd));
    if has_write && !plan.read_only && !yes && !std::io::stdout().is_terminal() {
        return Err(AvcError::Arg(
            "非 TTY 下默认要求 --yes（避免脚本意外执行写操作）".into(),
        ));
    }
    if has_write && !plan.read_only && !yes && std::io::stdout().is_terminal() {
        eprintln!(
            "[plan] intent={} (write, {} steps)",
            plan.intent,
            plan.steps.len()
        );
        for (i, s) in plan.steps.iter().enumerate() {
            eprintln!(
                "  {}. {} {}",
                i + 1,
                s.cmd,
                serde_json::to_string(&s.args).unwrap_or_default()
            );
        }
        eprintln!("run? [y/N]");
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| AvcError::Io(e.to_string()))?;
        if !line.trim().eq_ignore_ascii_case("y") {
            return Err(AvcError::Generic("plan rejected by user".into()));
        }
    }

    // 执行：在主进程调用 cli::run
    let mut results = Vec::new();
    for s in plan.steps.iter() {
        // s.cmd 是 "<noun> <verb>" 形式（LLM 输出）；拆成 argv tokens。
        // 带 --flag 形态 args 仍按 "key value" 追加（与 CLI 兼容）。
        let mut cli_argv: Vec<String> = s.cmd.split_whitespace().map(String::from).collect();
        if let Some(map) = s.args.as_object() {
            for (k, v) in map.iter() {
                cli_argv.push(k.clone());
                cli_argv.push(match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                });
            }
        }
        match crate::cli::run(&cli_argv) {
            Ok(()) => results.push(serde_json::json!({"cmd": s.cmd, "ok": true})),
            Err(e) => {
                results.push(serde_json::json!({"cmd": s.cmd, "ok": false, "error": e.to_string()}))
            }
        }
        // e went out of scope next iteration
    }

    if json {
        let v = serde_json::json!({
            "input": nl,
            "provider": provider_name,
            "intent": plan.intent,
            "read_only": plan.read_only,
            "results": results,
        });
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!("[ask] provider={} intent={}", provider_name, plan.intent);
        for r in &results {
            println!("  {}", r);
        }
    }

    let _ = yes;
    Ok(())
}

fn is_write_cmd(cmd: &str) -> bool {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.len() < 2 {
        return false;
    }
    WRITE_VERBS.contains(&parts[1])
}

fn validate_step(s: &PlanStep) -> AvcResult<()> {
    let parts: Vec<&str> = s.cmd.split_whitespace().collect();
    if parts.len() != 2 {
        return Err(AvcError::Arg(format!(
            "plan step cmd 应为 '<noun> <verb>' 但得到: {}",
            s.cmd
        )));
    }
    let verb = parts[1];
    let ok_read: &[&str] = &["list", "show", "versions"];
    let ok_write: &[&str] = WRITE_VERBS;
    if !(ok_read.contains(&verb) || ok_write.contains(&verb)) {
        return Err(AvcError::Arg(format!(
            "plan step 用到未支持的 verb '{}'；ask 仅允许 read_only('list'/'show'/'versions') 与 write({:?})",
            verb, ok_write
        )));
    }
    Ok(())
}

fn parse_plan(raw: &str) -> AvcResult<Plan> {
    // LLM 偶尔会包 ```json fences；这里试图剥离；失败原样解析
    let s = raw.trim();
    let candidate = if s.starts_with("```") {
        // 剥到第一个 '{' 与最后一个 '}'
        if let (Some(open), Some(close)) = (s.find('{'), s.rfind('}')) {
            &s[open..=close]
        } else {
            s
        }
    } else {
        s
    };
    serde_json::from_str::<Plan>(candidate)
        .map_err(|e| AvcError::Internal(format!("plan json parse failed: {}; raw={}", e, raw)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plan_direct_json() {
        let raw =
            r#"{"intent":"list","read_only":true,"steps":[{"cmd":"persona list","args":{}}]}"#;
        let p = parse_plan(raw).unwrap();
        assert_eq!(p.intent, "list");
        assert_eq!(p.steps.len(), 1);
        assert!(p.read_only);
    }

    #[test]
    fn parse_plan_fenced_json() {
        let raw = "```json\n{\"intent\":\"list\",\"read_only\":true,\"steps\":[]}\n```";
        let p = parse_plan(raw).unwrap();
        assert_eq!(p.stent_count(), 0);
        assert_eq!(p.intent, "list");
    }

    #[test]
    fn parse_plan_bad_json_errors() {
        let raw = "not json at all";
        assert!(parse_plan(raw).is_err());
    }

    #[test]
    fn is_write_cmd_true_false() {
        assert!(is_write_cmd("persona set-traits"));
        assert!(is_write_cmd("persona promote"));
        assert!(!is_write_cmd("persona list"));
        assert!(!is_write_cmd("persona show"));
        assert!(!is_write_cmd("foo bar"));
    }

    // 辅助 trait 让 test 可读
    trait Stent {
        fn stent_count(&self) -> usize;
    }
    impl Stent for Plan {
        fn stent_count(&self) -> usize {
            self.steps.len()
        }
    }
}
