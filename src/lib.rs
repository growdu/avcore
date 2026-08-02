//! AVCore — AI Video Core
//!
//! 极简内核：单进程 tokio + 单一 SQLite + DAG Pipeline + Provider trait。
//! 详见各子模块文档。

pub mod ask;
pub mod cli;
pub mod config;
pub mod db;
pub mod error;
pub mod output;
pub mod provider;
pub mod shell;
pub mod svc;

pub use error::{AvcError, AvcResult};
