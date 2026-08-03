//! Provider 健康与限速状态持久化
//!
//! 详见 docs/superpowers/specs/2026-08-03-provider-daemon-design.md §3-4

// 后续 T3-T5 会用到这些 import 与 const；先 allow 保持 clippy -D warnings 干净
#![allow(unused_imports, dead_code)]

use crate::error::AvcResult;
use crate::svc::now_iso;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

const HEALTH_KEEP_N: i64 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Healthy,
    Auth,
    RateLimited,
    Timeout,
    UpstreamError,
    Unconfigured,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Healthy => "healthy",
            Status::Auth => "auth",
            Status::RateLimited => "rate_limited",
            Status::Timeout => "timeout",
            Status::UpstreamError => "upstream_error",
            Status::Unconfigured => "unconfigured",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "healthy" => Some(Status::Healthy),
            "auth" => Some(Status::Auth),
            "rate_limited" => Some(Status::RateLimited),
            "timeout" => Some(Status::Timeout),
            "upstream_error" => Some(Status::UpstreamError),
            "unconfigured" => Some(Status::Unconfigured),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthRow {
    pub id: i64,
    pub provider_key: String,
    pub status: Status,
    pub latency_ms: Option<i64>,
    pub error_msg: Option<String>,
    pub checked_at: String,
    pub source: String, // "probe" or "hook"
}

pub fn provider_key(dim: &str, name: &str) -> String {
    format!("{}.{}", dim, name)
}

// 后续 task 填充 record / latest / rate_limit_upsert / rate_limit_get
