//! 交互式 Shell
//!
//! Phase 1.3：原子命令透传 + 内建 help/exit + NL 入口（复用 ask 的 plan 流水线）。
//! 完整设计见 docs/shell.md。

use std::io::{IsTerminal, Write};

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::AvcResult;

pub fn run(_args: &[String]) -> AvcResult<()> {
    let mut rl = DefaultEditor::new().map_err(|e| crate::error::AvcError::Internal(format!("readline init: {}", e)))?;
    let history_path = history_path()?;
    let _ = rl.load_history(&history_path);

    eprintln!("avc shell — type `help` for commands, `exit` to quit");

    loop {
        let prompt = if std::io::stdout().is_terminal() {
            "avc> "
        } else {
            ""
        };

        let readline = rl.readline(prompt);
        match readline {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() { continue; }

                // 内建
                match line {
                    "exit" | "quit" => break,
                    "help" | "?" => {
                        print_help();
                        continue;
                    }
                    "clear" => {
                        print!("\x1b[2J\x1b[1;1H");
                        continue;
                    }
                    "history" => {
                        for (i, h) in rl.history().iter().enumerate() {
                            println!("{:>4}  {}", i + 1, h);
                        }
                        continue;
                    }
                    _ => {}
                }

                // 加到 history（用于 !N 重跑）
                let _ = rl.add_history_entry(line);

                // 分类：原子 / NL
                // 启发式：首 token 是已知 noun (persona/sample/iterate/finetune/job/render/corpus/provider/config/doctor/version/...)
                // 或显式可识别 verb 时走 CLI。否则当作 NL。
                if looks_atomic(line) {
                    let argv: Vec<String> = std::iter::once("avc".to_string())
                        .chain(line.split_whitespace().map(String::from))
                        .collect();
                    let result = std::panic::catch_unwind(|| crate::cli::run(&argv));
                    match result {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => e.print(),
                        Err(_) => eprintln!("internal panic"),
                    }
                } else {
                    // NL：交给 ask。--yes 跳过 plan-confirm，让 shell 自动化用。
                    // TTY 下若计划含 write step，ask 内部仍会读 stdin y/n，
                    // 但 ask→shell 共用 stdin 行为已被 rusticline 替代；为简单起见强制 --yes。
                    eprintln!("[nl] {}", line);
                    match std::panic::catch_unwind(|| crate::ask::dispatch_nl(line, true)) {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => e.print(),
                        Err(_) => eprintln!("ask internal panic"),
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl-C: 取消当前输入，继续循环
                continue;
            }
            Err(ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("readline error: {}", e);
                break;
            }
        }
    }

    let _ = rl.save_history(&history_path);
    Ok(())
}

/// 粗粒度启发式：首 token 是已注册 noun 时视为原子。
const KNOWN_NOUNS: &[&str] = &[
    "persona", "sample", "iterate", "finetune", "job",
    "render", "corpus", "provider", "config", "doctor",
    "version", "init", "shell", "ask",
];

fn looks_atomic(line: &str) -> bool {
    let first = line.split_whitespace().next().unwrap_or("");
    KNOWN_NOUNS.contains(&first)
}

#[cfg(test)]
mod tests {
    use super::looks_atomic;

    #[test]
    fn atomic_inputs_match_known_nouns() {
        assert!(looks_atomic("persona list"));
        assert!(looks_atomic("finetune start yu --base-version 1"));
        assert!(looks_atomic("render run --persona yu"));
        assert!(looks_atomic("version"));
        assert!(looks_atomic("init"));
    }

    #[test]
    fn natural_language_inputs_do_not_match_nouns() {
        assert!(!looks_atomic("列出所有角色"));
        assert!(!looks_atomic("把 Yu 的 traits 改成严谨务实"));
        assert!(!looks_atomic("什么是 voice drift?"));
        assert!(!looks_atomic("yu 当前是哪个版本"));
    }

    #[test]
    fn empty_input_is_not_atomic() {
        assert!(!looks_atomic(""));
        assert!(!looks_atomic("   "));
    }
}

fn history_path() -> AvcResult<std::path::PathBuf> {
    let dir = crate::config::Config::default_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("shell_history"))
}

fn print_help() {
    println!(
        "Built-ins: help, exit, quit, clear, history
 Atomic:    <noun> <verb> [--flag ...]
 NL:        需先在 avc.toml 配置 provider.llm.*
            直接输自然语言 → LLM 解析 → 自动 dispatch

 Examples:
   avc> persona list
   avc> persona show yu
   avc> version
   avc> 列出所有角色
   avc> 把 Yu 的 traits 改成严谨务实
"
    );
}

#[allow(dead_code)]
fn _suppress_unused2() -> AvcResult<()> { Ok(()) }
#[allow(dead_code)]
fn _suppress_unused3<W: Write>() {}
