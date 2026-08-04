//! AVCore CLI 入口
//!
//! 根据参数与 TTY 选择三种执行入口之一：
//! - CLI 模式：`avc <atom>` 一次性精确命令
//! - Shell 模式：`avc shell` 或 TTY 下裸 `avc`，交互式 Shell
//! - ask 模式：`avc ask "..."`，非交互式 NL
//! - Daemon 内部入口：`avc _run`（T17 hidden verb，由 T18 的 `daemon start` 触发）

use std::io::IsTerminal;
use std::process::ExitCode;

use avc::{ask, cli, config, shell};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    // T17: 隐藏 verb `_run` —— daemon 内部入口。
    // `_run` 非 Send（内部 `LocalSet` + `&Connection` 跨 await），
    // 必须用单线程 runtime + LocalSet 驱动，不能用默认多线程 `#[tokio::main]`。
    if args.len() >= 2 && args[1] == "_run" {
        let cfg_path = match config::Config::default_config_path() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("config path error: {}", e);
                return ExitCode::from(1);
            }
        };
        let cfg = match config::Config::load(&cfg_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("config error: {}", e);
                return ExitCode::from(1);
            }
        };
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("daemon runtime build: {}", e);
                return ExitCode::from(1);
            }
        };
        let local = tokio::task::LocalSet::new();
        // `_run` 内部自建 LocalSet 并 `spawn_local` ping loop + http server；
        // 外层 `local.run_until(...)` 负责驱动其返回的 non-Send future。
        let daemon_fut = local.run_until(async move { avc::svc::daemon::_run(cfg).await });
        if let Err(e) = rt.block_on(daemon_fut) {
            e.print();
            return e.exit_code();
        }
        return ExitCode::SUCCESS;
    }

    // 入口路由（详见 docs/shell.md §1.1）
    // 注意：tracing init 在 _run 路径里跳过（daemon 自己 init_logging），
    // 在其他路径里走这里（shell/ask/cli）。
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_target(false)
        .try_init();

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
