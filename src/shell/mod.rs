//! 交互式 Shell
//!
//! Phase 1：原子命令透传 + 内建 help/exit + 占位 NL 解析（未配置 LLM 时报错）。
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

                // 走 CLI 派发
                let argv: Vec<String> = std::iter::once("avc".to_string())
                    .chain(line.split_whitespace().map(String::from))
                    .collect();

                let result = std::panic::catch_unwind(|| {
                    crate::cli::run(&argv)
                });

                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => e.print(),
                    Err(_) => {
                        eprintln!("internal panic");
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

fn history_path() -> AvcResult<std::path::PathBuf> {
    let dir = crate::config::Config::default_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("shell_history"))
}

fn print_help() {
    println!(
        "Built-ins: help, exit, quit, clear, history
         Atomic:    <noun> <verb> [--flag ...]
         NL:        暂时未启用（需要先在 avc.toml 配置 provider.llm.*）
         
         Examples:
           avc> persona list
           avc> persona show yu
           avc> version"
    );
}

#[allow(dead_code)]
fn _suppress_unused2() -> AvcResult<()> { Ok(()) }
#[allow(dead_code)]
fn _suppress_unused3<W: Write>() {}
