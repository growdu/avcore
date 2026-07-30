//! 输出约定：默认人类可读 / --json / --quiet / --watch
//!
//! 详见 docs/cli.md §5。

use crate::error::AvcResult;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputMode {
    #[default]
    Text,
    Json,
    Quiet,
}

impl OutputMode {
    pub fn from_flags(json: bool, quiet: bool) -> Self {
        if quiet {
            OutputMode::Quiet
        } else if json {
            OutputMode::Json
        } else {
            OutputMode::Text
        }
    }
}

/// 渲染输出
pub fn print<T: Serialize>(mode: OutputMode, value: &T) -> AvcResult<()> {
    match mode {
        OutputMode::Json => {
            let s = serde_json::to_string_pretty(value)?;
            println!("{}", s);
        }
        OutputMode::Quiet => {
            // Quiet 模式由各命令自己提取关键 ID 输出
        }
        OutputMode::Text => {
            println!("{}", serde_json::to_string_pretty(value)?);
        }
    }
    Ok(())
}
