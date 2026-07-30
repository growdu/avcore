//! AVCore CLI 入口
//!
//! 根据参数与 TTY 选择三种执行入口之一：
//! - CLI 模式：`avc <atom>` 一次性精确命令
//! - Shell 模式：`avc shell` 或 TTY 下裸 `avc`，交互式 Shell
//! - ask 模式：`avc ask "..."`，非交互式 NL

use std::io::IsTerminal;
use std::process::ExitCode;

use avc::{cli, shell, ask};

fn main() -> ExitCode {
    // 初始化 tracing
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")))
        .with_target(false)
        .try_init();

    let args: Vec<String> = std::env::args().collect();

    // 入口路由（详见 docs/shell.md §1.1）
    let mode = if args.len() <= 1 {
        if std::io::stdout().is_terminal() {
            Mode::Shell
        } else {
            Mode::Help
        }
    } else {
        match args[1].as_str() {
            "shell" => Mode::Shell,
            "ask" => Mode::Ask,
            "--help" | "-h" => Mode::Help,
            "--version" | "version" => Mode::Version,
            _ => Mode::Cli,
        }
    };

    let result = match mode {
        Mode::Shell => shell::run(&args),
        Mode::Ask => ask::run(&args),
        Mode::Cli => cli::run(&args),
        Mode::Help => cli::print_help(),
        Mode::Version => cli::print_version(),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            e.print();
            e.exit_code()
        }
    }
}

enum Mode {
    Cli,
    Shell,
    Ask,
    Help,
    Version,
}
