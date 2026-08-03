//! 业务服务层
//!
//! 详见 docs/modules/。

pub mod corpus;
pub mod daemon;
pub mod drift;
pub mod finetune;
pub mod health;
pub mod iterate;
pub mod persona;
pub mod pipeline;
pub mod render;
pub mod sample;

use chrono::Utc;

pub fn now_ts() -> i64 {
    Utc::now().timestamp()
}

pub fn now_iso() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn new_id(prefix: &str) -> String {
    format!(
        "{}_{}",
        prefix,
        ulid::Ulid::new().to_string().to_lowercase()
    )
}
