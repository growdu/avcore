//! CLI 子命令
//!
//! 三种入口（CLI / Shell / ask）的精确命令路径。
//! 详见 docs/cli.md。

pub mod root;
pub mod persona;
pub mod sample;
pub mod iterate;
pub mod finetune;
pub mod job;
pub mod render;
pub mod corpus;
pub mod provider;

use crate::AvcError;
use crate::AvcResult;

pub fn run(args: &[String]) -> AvcResult<()> {
    // main.rs 已经判定 mode=Cli，args 形如 ["/path/to/avc", "persona", "list", ...]
    // 或者 Shell 转发时是 ["avc", "persona", "list", ...]
    // 跳过首参（程序名 / 占位 "avc"）
    let argv: Vec<String> = if args.len() >= 2 {
        let first = &args[0];
        // basename 匹配或等于 "avc"
        let is_prog = first == "avc"
            || std::path::Path::new(first)
                .file_name()
                .map(|n| n == "avc")
                .unwrap_or(false);
        if is_prog { args[1..].to_vec() } else { args.to_vec() }
    } else {
        args.to_vec()
    };

    if argv.is_empty() {
        return print_help();
    }

    match argv[0].as_str() {
        "init" => root::cmd_init(),
        "version" | "--version" => print_version(),
        "--help" | "-h" | "help" => print_help(),
        "doctor" => root::cmd_doctor(),
        "config" => root::cmd_config(&argv[1..]),
        "persona" => persona::dispatch(&argv[1..]),
        "sample" => sample::dispatch(&argv[1..]),
        "iterate" => iterate::dispatch(&argv[1..]),
        "finetune" => finetune::dispatch(&argv[1..]),
        "job" => job::dispatch(&argv[1..]),
        "render" => render::dispatch(&argv[1..]),
        "corpus" => corpus::dispatch(&argv[1..]),
        "provider" => provider::dispatch(&argv[1..]),
        other => Err(AvcError::Arg(format!("未知子命令: {}", other))),
    }
}

pub fn print_help() -> AvcResult<()> {
    let help = r#"
avc — AI Video Core

USAGE:
    avc <command> [args]
    avc shell                # 交互式 Shell
    avc ask "..."            # 非交互式自然语言

COMMANDS:
    init                     初始化 ~/.local/share/avc/avc.db
    version                  打印版本
    doctor                   集成诊断
    config get|set <k> <v>   读写 avc.toml
    persona create|list|...  角色管理
    sample add|list|...      训练样本
    iterate list|show        refine 任务账本
    finetune start|list|...  finetune 任务账本
    job list|show|export     渲染任务账本
    render script|video|run  出片工作流
    corpus create|search     知识语料
    provider list|test       Provider 诊断

更多信息见 docs/cli.md
"#;
    print!("{}", help);
    Ok(())
}

pub fn print_version() -> AvcResult<()> {
    println!("avc {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
