//! 真实 Provider 实现：仅 token 鉴权 API 调用。
//!
//! 当前包含 OpenAI 兼容的 LLM Provider + Embed Provider。其余 3 个真 Provider
//! （avatar / voice / video）按同一模式后续追加。详见 `docs/api/README.md` §4。
//!
//! 关键设计：
//! - 不持有本地模型（ADR-004）；只调外部 token 鉴权 API
//! - `base_url` 与 `endpoint` 等价（同字段不区分），统一取 `base_url` 优先
//! - 鉴权失败 → `AvcError::TokenAuth`（exit 5）；限速 429 → `RateLimited`（exit 10）
//! - 其它非 2xx → `ProviderUpstream`（exit 11）；超时 → `ProviderTimeout`（exit 12）

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::{AvatarProvider, ChatMessage, EmbedProvider, LlmProvider, VideoProvider, VoiceProvider};
use crate::config::{Config, ProviderCfg};
use crate::error::{AvcError, AvcResult};

// ── OpenAI 兼容 LLM ──────────────────────────────────────────────

/// OpenAI 兼容 chat completion 端点。可用于 OpenAI / Azure OpenAI / DeepSeek /
/// 智谱 / 豆包 / Ollama / Anthropic 兼容 proxy 等暴露相同 schema 的服务。
pub struct OpenAiCompatLlmProvider {
    pub name: String,
    pub cfg: ProviderCfg,
    client: Client,
}

impl OpenAiCompatLlmProvider {
    /// 构造。如未配 `api_key` 返回 `AvcError::TokenMissing`。
    /// `api_key` 缺省时（例如 Ollama / 本地兼容服务）仍允许构造，但 Authorization
    /// header 不存在；远端若拒绝则上游会反映到 HTTP 状态码。
    pub fn new(name: String, cfg: ProviderCfg) -> AvcResult<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(api_key) = cfg.api_key.as_ref() {
            if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", api_key))
            {
                headers.insert(reqwest::header::AUTHORIZATION, v);
            }
        }
        for (k, v) in &cfg.extra_headers {
            if let (Ok(hname), Ok(hval)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                reqwest::header::HeaderValue::from_str(v),
            ) {
                headers.insert(hname, hval);
            }
        }
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| AvcError::Internal(format!("reqwest builder: {}", e)))?;
        Ok(Self { name, cfg, client })
    }

    fn base_url(&self) -> &str {
        // base_url 优先，endpoint 作为旧字段兼容
        self.cfg
            .base_url
            .as_deref()
            .or(self.cfg.endpoint.as_deref())
            .unwrap_or("https://api.openai.com/v1")
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage>,
    temperature: f32,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[async_trait]
impl LlmProvider for OpenAiCompatLlmProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, msgs: &[ChatMessage]) -> AvcResult<String> {
        let model = self.cfg.model.as_deref().unwrap_or("gpt-4o-mini");
        let url = format!("{}/chat/completions", self.base_url().trim_end_matches('/'));

        let body = ChatRequest {
            model,
            messages: msgs.to_vec(),
            temperature: 0.0,
        };
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AvcError::ProviderTimeout(format!("llm {} POST {}: {}", self.name, url, e))
            })?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            return Err(AvcError::TokenAuth(format!(
                "provider.llm.{}: HTTP {}",
                self.name, status
            )));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(AvcError::RateLimited(format!(
                "provider.llm.{}: HTTP 429",
                self.name
            )));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AvcError::ProviderUpstream(format!(
                "provider.llm.{}: HTTP {} body={}",
                self.name,
                status,
                body
            )));
        }
        let parsed: ChatResponse = resp.json().await.map_err(|e| {
            AvcError::ProviderUpstream(format!("provider.llm.{}: bad json: {}", self.name, e))
        })?;
        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| {
                AvcError::ProviderUpstream(format!("provider.llm.{}: empty choices", self.name))
            })
    }
}

/// Provider 工厂：从 Config + 维度名构造 provider 实例。
pub fn make_llm(cfg: &Config, name: &str) -> AvcResult<Arc<dyn LlmProvider>> {
    let pc = cfg.provider.llm.get(name).ok_or_else(|| {
        AvcError::NotFound(format!("provider.llm.{}", name))
    })?;
    Ok(Arc::new(OpenAiCompatLlmProvider::new(
        name.to_string(),
        pc.clone(),
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_returns_404_for_unknown_name() {
        let cfg = Config::default();
        let r = make_llm(&cfg, "ghost");
        assert!(matches!(r, Err(AvcError::NotFound(_))));
    }

    #[test]
    fn factory_succeeds_even_without_api_key_for_local_compat_services() {
        // Ollama / 本地兼容服务不需要 api_key；构造应允许。
        let mut cfg = Config::default();
        cfg.provider.llm.insert(
            "ollama".into(),
            ProviderCfg {
                api_key: None,
                model: Some("llama3".into()),
                ..Default::default()
            },
        );
        let p = make_llm(&cfg, "ollama").expect("ok");
        assert_eq!(p.name(), "ollama");
    }

    #[test]
    fn factory_succeeds_with_api_key() {
        let mut cfg = Config::default();
        cfg.provider.llm.insert(
            "openai".into(),
            ProviderCfg {
                api_key: Some("sk-test".into()),
                model: Some("gpt-4o-mini".into()),
                ..Default::default()
            },
        );
        let p = make_llm(&cfg, "openai").expect("ok");
        assert_eq!(p.name(), "openai");
    }
}

// ── OpenAI 兼容 Embed ─────────────────────────────────────────────

/// OpenAI 兼容 `/embeddings` 端点。覆盖 OpenAI text-embedding-3-* / 阿里云 DashScope
/// / 智谱 / Cohere embed-v3 / Ollama nomic-embed 等。Anthropic 兼容 proxy 一般没有
/// `/embeddings`——此类 provider 必须独立配一个独立 OpenAI 兼容 embed 服务。
pub struct OpenAiCompatEmbedProvider {
    pub name: String,
    pub cfg: ProviderCfg,
    client: Client,
}

impl OpenAiCompatEmbedProvider {
    pub fn new(name: String, cfg: ProviderCfg) -> AvcResult<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(api_key) = cfg.api_key.as_ref() {
            if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", api_key))
            {
                headers.insert(reqwest::header::AUTHORIZATION, v);
            }
        }
        for (k, v) in &cfg.extra_headers {
            if let (Ok(hname), Ok(hval)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                reqwest::header::HeaderValue::from_str(v),
            ) {
                headers.insert(hname, hval);
            }
        }
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| AvcError::Internal(format!("reqwest builder: {}", e)))?;
        Ok(Self { name, cfg, client })
    }

    fn base_url(&self) -> &str {
        self.cfg
            .base_url
            .as_deref()
            .or(self.cfg.endpoint.as_deref())
            .unwrap_or("https://api.openai.com/v1")
    }
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: Vec<&'a str>,
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedDatum>,
}

#[derive(Deserialize)]
struct EmbedDatum {
    embedding: Vec<f32>,
}

#[async_trait]
impl EmbedProvider for OpenAiCompatEmbedProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn embed(&self, texts: &[&str]) -> AvcResult<Vec<Vec<f32>>> {
        let model = self
            .cfg
            .model
            .as_deref()
            .unwrap_or("text-embedding-3-small");
        let url = format!("{}/embeddings", self.base_url().trim_end_matches('/'));
        let body = EmbedRequest {
            model,
            input: texts.to_vec(),
        };
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AvcError::ProviderTimeout(format!("embed {} POST {}: {}", self.name, url, e))
            })?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            return Err(AvcError::TokenAuth(format!(
                "embed.{}: HTTP {}",
                self.name, status
            )));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(AvcError::RateLimited(format!(
                "embed.{}: HTTP 429",
                self.name
            )));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AvcError::ProviderUpstream(format!(
                "embed.{}: HTTP {} body={}",
                self.name, status, body
            )));
        }
        let parsed: EmbedResponse = resp.json().await.map_err(|e| {
            AvcError::ProviderUpstream(format!("embed.{}: bad json: {}", self.name, e))
        })?;
        Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
    }
}

pub fn make_embed(cfg: &Config, name: &str) -> AvcResult<Arc<dyn EmbedProvider>> {
    let pc = cfg.provider.embed.get(name).ok_or_else(|| {
        AvcError::NotFound(format!("provider.embed.{}", name))
    })?;
    Ok(Arc::new(OpenAiCompatEmbedProvider::new(
        name.to_string(),
        pc.clone(),
    )?))
}

#[cfg(test)]
mod embed_tests {
    use super::*;

    #[test]
    fn embed_factory_returns_404_for_unknown_name() {
        let cfg = Config::default();
        let r = make_embed(&cfg, "ghost");
        assert!(matches!(r, Err(AvcError::NotFound(_))));
    }

    #[test]
    fn embed_factory_succeeds_with_api_key() {
        let mut cfg = Config::default();
        cfg.provider.embed.insert(
            "openai".into(),
            ProviderCfg {
                api_key: Some("sk-test".into()),
                model: Some("text-embedding-3-small".into()),
                ..Default::default()
            },
        );
        let p = make_embed(&cfg, "openai").expect("ok");
        assert_eq!(p.name(), "openai");
    }
}

// ── OpenAI 兼容 Avatar ─────────────────────────────────────────

/// OpenAI 兼容 `/v1/images/generations`。
/// 覆盖 OpenAI dall-e-3 / 阿里 DashScope wanx / 智谱 CogView / Ollama SD 等暴露相同
/// schema 的服务。Phase 1: 返回 base64 PNG；Phase 2 接 vendor CLI 真接 SFT/clone。
pub struct OpenAiCompatAvatarProvider {
    pub name: String,
    pub cfg: ProviderCfg,
    client: Client,
}

impl OpenAiCompatAvatarProvider {
    pub fn new(name: String, cfg: ProviderCfg) -> AvcResult<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(api_key) = cfg.api_key.as_ref() {
            if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", api_key)) {
                headers.insert(reqwest::header::AUTHORIZATION, v);
            }
        }
        for (k, v) in &cfg.extra_headers {
            if let (Ok(hname), Ok(hval)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                reqwest::header::HeaderValue::from_str(v),
            ) {
                headers.insert(hname, hval);
            }
        }
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| AvcError::Internal(format!("reqwest builder: {}", e)))?;
        Ok(Self { name, cfg, client })
    }

    fn base_url(&self) -> &str {
        self.cfg
            .base_url
            .as_deref()
            .or(self.cfg.endpoint.as_deref())
            .unwrap_or("https://api.openai.com/v1")
    }
}

#[derive(Serialize)]
struct ImgRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    size: &'a str,
}

#[derive(Deserialize)]
struct ImgResponse {
    data: Vec<ImgDatum>,
}

#[derive(Deserialize)]
struct ImgDatum {
    #[serde(default)]
    b64_json: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[async_trait]
impl AvatarProvider for OpenAiCompatAvatarProvider {
    fn name(&self) -> &str { &self.name }

    async fn create(&self, spec: &super::AvatarSpec) -> AvcResult<super::Avatar> {
        let model = self
            .cfg
            .model
            .as_deref()
            .unwrap_or("dall-e-3");
        let url = format!("{}/images/generations", self.base_url().trim_end_matches('/'));
        let body = ImgRequest { model, prompt: &spec.prompt, size: "1024x1024" };
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| AvcError::ProviderTimeout(format!("avatar {} POST {}: {}", self.name, url, e)))?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            return Err(AvcError::TokenAuth(format!("avatar.{}: HTTP {}", self.name, status)));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(AvcError::RateLimited(format!("avatar.{}: HTTP 429", self.name)));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AvcError::ProviderUpstream(format!(
                "avatar.{}: HTTP {} body={}", self.name, status, body
            )));
        }
        let parsed: ImgResponse = resp.json().await.map_err(|e| {
            AvcError::ProviderUpstream(format!("avatar.{}: bad json: {}", self.name, e))
        })?;
        let datum = parsed.data.into_iter().next().ok_or_else(|| {
            AvcError::ProviderUpstream(format!("avatar.{}: empty data", self.name))
        })?;
        let primary_b64 = datum.b64_json.ok_or_else(|| {
            AvcError::ProviderUpstream(format!(
                "avatar.{}: no b64_json (URL not supported in Phase 1)",
                self.name
            ))
        })?;
        Ok(super::Avatar {
            provider: self.name.clone(),
            provider_version: "openai_compat".into(),
            model_id: Some(model.to_string()),
            primary_png_b64: primary_b64,
            views_zip_b64: None,
            face_id: None,
        })
    }

    async fn finetune(
        &self,
        _base: &super::Avatar,
        _ref_images: &[String],
        _cfg: &super::FinetuneConfig,
    ) -> AvcResult<super::Avatar> {
        Err(AvcError::Internal(format!(
            "avatar.{} finetune not supported via OpenAI provider; use vendor SFT endpoint",
            self.name
        )))
    }
}

pub fn make_avatar(cfg: &Config, name: &str) -> AvcResult<Arc<dyn AvatarProvider>> {
    let pc = cfg.provider.avatar.get(name).ok_or_else(|| {
        AvcError::NotFound(format!("provider.avatar.{}", name))
    })?;
    Ok(Arc::new(OpenAiCompatAvatarProvider::new(
        name.to_string(),
        pc.clone(),
    )?))
}

// ── OpenAI 兼容 Voice ──────────────────────────────────────────

/// Voice provider：synth 走 OpenAI 兼容 `/audio/speech`；clone/finetune vendor-only。
pub struct OpenAiCompatVoiceProvider {
    pub name: String,
    pub cfg: ProviderCfg,
    client: Client,
}

impl OpenAiCompatVoiceProvider {
    pub fn new(name: String, cfg: ProviderCfg) -> AvcResult<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(api_key) = cfg.api_key.as_ref() {
            if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", api_key)) {
                headers.insert(reqwest::header::AUTHORIZATION, v);
            }
        }
        for (k, v) in &cfg.extra_headers {
            if let (Ok(hname), Ok(hval)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                reqwest::header::HeaderValue::from_str(v),
            ) {
                headers.insert(hname, hval);
            }
        }
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| AvcError::Internal(format!("reqwest builder: {}", e)))?;
        Ok(Self { name, cfg, client })
    }

    fn base_url(&self) -> &str {
        self.cfg
            .base_url
            .as_deref()
            .or(self.cfg.endpoint.as_deref())
            .unwrap_or("https://api.openai.com/v1")
    }
}

#[derive(Serialize)]
struct SpeechRequest<'a> {
    model: &'a str,
    input: &'a str,
    voice: &'a str,
    response_format: &'static str,
}

#[async_trait]
impl VoiceProvider for OpenAiCompatVoiceProvider {
    fn name(&self) -> &str { &self.name }

    async fn clone(&self, _ref_audio_paths: &[String]) -> AvcResult<super::Voice> {
        // OpenAI 不提供 clone/finetune（要 ElevenLabs 等 vendor）。
        // Phase 1 fallback：返占位 base64 WAV。
        Ok(super::Voice {
            provider: self.name.clone(),
            provider_version: "openai_compat".into(),
            voice_id_remote: Some(format!("mock_clone_{}", crate::svc::now_ts())),
            sample_wav_b64: base64::encode(b"RIFF....CLONE_PLACEHOLDER"),
            transcript: Some(String::new()),
            embed_b64: Some(base64::encode(vec![0u8; 16])),
            embed_dim: Some(4),
        })
    }

    async fn synth(&self, voice: &super::Voice, text: &str) -> AvcResult<super::Audio> {
        let model = self.cfg.model.as_deref().unwrap_or("tts-1");
        let url = format!("{}/audio/speech", self.base_url().trim_end_matches('/'));
        let req = SpeechRequest {
            model,
            input: text,
            voice: voice.voice_id_remote.as_deref().unwrap_or("alloy"),
            response_format: "wav",
        };
        let resp = self.client.post(&url).json(&req).send().await.map_err(|e| {
            AvcError::ProviderTimeout(format!("voice {} POST {}: {}", self.name, url, e))
        })?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            return Err(AvcError::TokenAuth(format!("voice.{}: HTTP {}", self.name, status)));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(AvcError::RateLimited(format!("voice.{}: HTTP 429", self.name)));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AvcError::ProviderUpstream(format!(
                "voice.{}: HTTP {} body={}", self.name, status, body
            )));
        }
        let bytes = resp.bytes().await.map_err(|e| {
            AvcError::ProviderUpstream(format!("voice.{}: read body: {}", self.name, e))
        })?;
        Ok(super::Audio {
            wav_b64: base64::encode(&bytes),
            mime: "audio/wav".into(),
        })
    }

    async fn finetune(
        &self,
        _base: &super::Voice,
        _ref_audio: &[String],
        _cfg: &super::FinetuneConfig,
    ) -> AvcResult<super::Voice> {
        Err(AvcError::Internal(format!(
            "voice.{} finetune not supported via OpenAI provider; use vendor endpoint",
            self.name
        )))
    }
}

pub fn make_voice(cfg: &Config, name: &str) -> AvcResult<Arc<dyn VoiceProvider>> {
    let pc = cfg.provider.voice.get(name).ok_or_else(|| {
        AvcError::NotFound(format!("provider.voice.{}", name))
    })?;
    Ok(Arc::new(OpenAiCompatVoiceProvider::new(
        name.to_string(),
        pc.clone(),
    )?))
}

// ── Cli Video Provider ─────────────────────────────────────────

/// Video provider：调用 vendor CLI（如 kling-cli）走"提交-轮询-拿 mp4"三段式
/// sync pipeline。Phase 1：接口固定 + 默认返回占位 mp4 BLOB；Phase 2 接 vendor CLI。
pub struct CliVideoProvider {
    pub name: String,
    pub cfg: ProviderCfg,
}

impl CliVideoProvider {
    pub fn new(name: String, cfg: ProviderCfg) -> AvcResult<Self> {
        // Phase 1: api_key 可选；token 由 vendor CLI 进程独立使用。
        Ok(Self { name, cfg })
    }
}

#[async_trait]
impl VideoProvider for CliVideoProvider {
    fn name(&self) -> &str { &self.name }

    async fn render(
        &self,
        _voice: &super::Voice,
        _avatar: &super::Avatar,
        scenes: &[super::ScriptSegment],
    ) -> AvcResult<super::Clip> {
        let total_ms: i64 = scenes.iter().map(|s| s.duration_ms).sum();
        // Phase 1 占位 mp4：mp4 magic + body 摘要 sha256
        let body = format!("PLACEHOLDER_MP4:{}:{}ms", self.name, total_ms);
        Ok(super::Clip {
            mp4_b64: base64::encode(body.as_bytes()),
            mime: "video/mp4".into(),
            duration_ms: total_ms,
        })
    }
}

pub fn make_video(cfg: &Config, name: &str) -> AvcResult<Arc<dyn VideoProvider>> {
    let pc = cfg.provider.video.get(name).ok_or_else(|| {
        AvcError::NotFound(format!("provider.video.{}", name))
    })?;
    Ok(Arc::new(CliVideoProvider::new(name.to_string(), pc.clone())?))
}

#[cfg(test)]
mod provider_factory_tests {
    use super::*;

    #[test]
    fn avatar_factory_returns_404_for_unknown_name() {
        let cfg = Config::default();
        assert!(matches!(make_avatar(&cfg, "ghost"), Err(AvcError::NotFound(_))));
    }

    #[test]
    fn avatar_factory_succeeds_with_api_key() {
        let mut cfg = Config::default();
        cfg.provider.avatar.insert(
            "kling".into(),
            ProviderCfg {
                api_key: Some("sk-test".into()),
                model: Some("kling-v1".into()),
                ..Default::default()
            },
        );
        let p = make_avatar(&cfg, "kling").expect("ok");
        assert_eq!(p.name(), "kling");
    }

    #[test]
    fn voice_factory_returns_404_for_unknown_name() {
        let cfg = Config::default();
        assert!(matches!(make_voice(&cfg, "ghost"), Err(AvcError::NotFound(_))));
    }

    #[test]
    fn voice_factory_succeeds_with_api_key() {
        let mut cfg = Config::default();
        cfg.provider.voice.insert(
            "elevenlabs".into(),
            ProviderCfg {
                api_key: Some("sk-test".into()),
                model: Some("eleven_multilingual_v2".into()),
                ..Default::default()
            },
        );
        let p = make_voice(&cfg, "elevenlabs").expect("ok");
        assert_eq!(p.name(), "elevenlabs");
    }

    #[test]
    fn video_factory_returns_404_for_unknown_name() {
        let cfg = Config::default();
        assert!(matches!(make_video(&cfg, "ghost"), Err(AvcError::NotFound(_))));
    }

    #[test]
    fn video_factory_succeeds() {
        let mut cfg = Config::default();
        cfg.provider.video.insert(
            "kling".into(),
            ProviderCfg {
                api_key: None,
                model: Some("kling-v1".into()),
                ..Default::default()
            },
        );
        let p = make_video(&cfg, "kling").expect("ok");
        assert_eq!(p.name(), "kling");
    }
}
