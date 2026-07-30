//! pipeline-svc：DAG 引擎
//!
//! Phase 1：仅骨架。Phase 2+ 实现真正调度。
//! 详见 docs/modules/pipeline.md。

use crate::error::AvcResult;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSpec {
    pub id: String,
    pub kind: String, // avatar / voice / llm / video / embed / compose / gate / branch
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default)]
    pub input_from: Vec<String>,
    #[serde(default)]
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagSpec {
    pub nodes: Vec<NodeSpec>,
}

pub fn execute(_dag: &DagSpec) -> AvcResult<()> {
    // Phase 1 stub
    Ok(())
}
