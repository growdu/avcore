//! Provider trait + 内置 Provider
//!
//! 详见 docs/api/README.md。

pub mod minimax;
pub mod mock;
pub mod probe;
pub mod real;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::AvcResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarSpec {
    pub prompt: String,
    pub style: Option<String>,
    pub ref_image_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Avatar {
    pub provider: String,
    pub provider_version: String,
    pub model_id: Option<String>,
    pub primary_png_b64: String,       // base64-encoded PNG BLOB
    pub views_zip_b64: Option<String>, // base64-encoded multi-view zip
    pub face_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Voice {
    pub provider: String,
    pub provider_version: String,
    pub voice_id_remote: Option<String>,
    pub sample_wav_b64: String,
    pub transcript: Option<String>,
    pub embed_b64: Option<String>,
    pub embed_dim: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Audio {
    pub wav_b64: String,
    pub mime: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptSegment {
    pub scene_index: i64,
    pub text: String,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Script_ {
    pub topic: String,
    pub segments: Vec<ScriptSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub mp4_b64: String,
    pub mime: String,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Embedding {
    pub values: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinetuneConfig {
    pub full_retrain: bool,
    pub epochs: u32,
    pub consistency_threshold: f32,
}

#[async_trait]
pub trait AvatarProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn create(&self, spec: &AvatarSpec) -> AvcResult<Avatar>;
    async fn finetune(
        &self,
        base: &Avatar,
        ref_images: &[String],
        cfg: &FinetuneConfig,
    ) -> AvcResult<Avatar>;
}

#[async_trait]
pub trait VoiceProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn clone(&self, ref_audio_paths: &[String]) -> AvcResult<Voice>;
    async fn synth(&self, voice: &Voice, text: &str) -> AvcResult<Audio>;
    async fn finetune(
        &self,
        base: &Voice,
        ref_audio_paths: &[String],
        cfg: &FinetuneConfig,
    ) -> AvcResult<Voice>;
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn chat(&self, msgs: &[ChatMessage]) -> AvcResult<String>;
}

#[async_trait]
pub trait VideoProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn render(
        &self,
        voice: &Voice,
        avatar: &Avatar,
        scenes: &[ScriptSegment],
    ) -> AvcResult<Clip>;
}

#[async_trait]
pub trait EmbedProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn embed(&self, texts: &[&str]) -> AvcResult<Vec<Vec<f32>>>;
}
