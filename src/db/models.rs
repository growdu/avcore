//! SQLite row 模型
//!
//! 命名：snake_case 列 ↔ snake_case 字段。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaModel {
    pub id: String,
    pub name: String,
    pub archetype: Option<String>,
    pub description: Option<String>,
    pub current_version: i64,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaVersion {
    pub persona_model_id: String,
    pub version: i64,
    pub parent_version: Option<i64>,
    pub status: String,
    pub avatar_provider: Option<String>,
    pub voice_provider: Option<String>,
    pub persona_descriptor_json: Option<String>,
    pub knowledge_binding_json: Option<String>,
    pub manifest_json: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaSample {
    pub id: String,
    pub persona_model_id: String,
    pub version_id_at_collection: Option<i64>,
    pub kind: String,
    pub text: Option<String>,
    pub source: String,
    pub consent_proof: Option<String>,
    pub quality_score: Option<f64>,
    pub sha256: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterateJob {
    pub id: String,
    pub persona_model_id: String,
    pub target_version: i64,
    pub changes_json: String,
    pub status: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinetuneJob {
    pub id: String,
    pub persona_model_id: String,
    pub base_version: i64,
    pub target_version: Option<i64>,
    pub scope_json: String,
    pub config_json: Option<String>,
    pub status: String,
    pub result_version: Option<i64>,
    pub drift_report_json: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Script {
    pub id: String,
    pub persona_model_id: String,
    pub persona_version: i64,
    pub topic: String,
    pub duration_ms: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub script_id: Option<String>,
    pub persona_model_id: String,
    pub persona_version: i64,
    pub status: String,
    pub progress: Option<f64>,
    pub current_step: Option<String>,
    pub options_json: Option<String>,
    pub error_json: Option<String>,
    pub created_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStep {
    pub id: String,
    pub job_id: String,
    pub node_id: String,
    pub status: String,
    pub attempt: i64,
    pub outputs_json: Option<String>,
    pub error_json: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeCorpus {
    pub id: String,
    pub name: Option<String>,
    pub source_type: Option<String>,
    pub language: Option<String>,
    pub chunk_count: i64,
    pub index_version: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusChunk {
    pub id: String,
    pub corpus_id: String,
    pub ordinal: i64,
    pub content: String,
    pub embed_dim: Option<i64>,
    pub token_count: Option<i64>,
    pub deprecated: i64,
    pub meta_json: Option<String>,
}
