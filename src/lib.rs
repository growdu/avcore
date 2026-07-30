//! AVCore — AI Video Core
//!
//! 极简内核：单进程 tokio + 单一 SQLite + DAG Pipeline + Provider trait。
//! 详见各子模块文档。

pub mod error;
pub mod output;
pub mod config;
pub mod db;
pub mod provider;
pub mod svc;
pub mod cli;
pub mod shell;
pub mod ask;

pub use error::{AvcError, AvcResult};
