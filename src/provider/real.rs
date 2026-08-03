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
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::{
    AvatarProvider, ChatMessage, EmbedProvider, LlmProvider, VideoProvider, VoiceProvider,
};
use crate::config::{Config, ProviderCfg};
use crate::error::{AvcError, AvcResult};
use crate::svc::health::{parse_retry_after, rate_limit_upsert, record, Status};

// ── Passive health/rate-limit hooks (best-effort) ────────────────
//
// Provider hook 层在 chat() / embed() / create() 等返回 error 时**被动**写
// `provider_health` / `provider_rate_limit` 表，供 daemon 健康探测和前端查询。
// 设计要点：
// - 第一次调用时懒打开默认 DB；失败 → 用 in-memory DB fallback（best-effort）。
// - 写失败一律 `.ok()` 吞掉，绝不影响主调用方 error 返回与 exit code。
// - 业务错误的原 `Err(...)` 必须原样保留（不改 variant，不改 exit code）。
fn db_conn() -> &'static std::sync::Mutex<rusqlite::Connection> {
    use std::sync::{Mutex, OnceLock};
    static POOL: OnceLock<Mutex<rusqlite::Connection>> = OnceLock::new();
    POOL.get_or_init(|| {
        let conn = crate::db::open_default().unwrap_or_else(|_| {
            // Fall back to in-memory so hooks don't crash the main process.
            rusqlite::Connection::open_in_memory().expect("in-mem")
        });
        Mutex::new(conn)
    })
}

/// Best-effort record wrapper：拿不到锁 / 写失败一律静默吞掉，绝不影响主调用方。
fn hook_record(key: &str, status: Status, latency_ms: Option<i64>, err_msg: &str) {
    if let Ok(conn) = db_conn().lock() {
        let _ = record(&conn, key, status, latency_ms, Some(err_msg), "hook");
    }
}

/// Best-effort rate_limit_upsert wrapper。
fn hook_rate_limit_upsert(key: &str, retry_after_s: Option<i64>, until_ts: Option<&str>) {
    if let Ok(conn) = db_conn().lock() {
        let _ = rate_limit_upsert(&conn, key, retry_after_s, until_ts);
    }
}

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
        let provider_key = format!("llm.{}", self.name);
        let started = Instant::now();
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
                let msg = format!("llm {} POST {}: {}", self.name, url, e);
                hook_record(
                    &provider_key,
                    Status::Timeout,
                    Some(started.elapsed().as_millis() as i64),
                    "timeout",
                );
                AvcError::ProviderTimeout(msg)
            })?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            hook_record(
                &provider_key,
                Status::Auth,
                Some(started.elapsed().as_millis() as i64),
                "auth",
            );
            return Err(AvcError::TokenAuth(format!(
                "provider.llm.{}: HTTP {}",
                self.name, status
            )));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            // 被动 hook：尝试解析 Retry-After 头算 until_ts
            let retry_after_s = resp
                .headers()
                .get("retry-after")
                .and_then(|h| h.to_str().ok())
                .and_then(parse_retry_after);
            let until_ts = retry_after_s.map(|s| {
                use chrono::Utc;
                (Utc::now() + chrono::Duration::seconds(s))
                    .format("%Y-%m-%dT%H:%M:%SZ")
                    .to_string()
            });
            hook_record(
                &provider_key,
                Status::RateLimited,
                Some(started.elapsed().as_millis() as i64),
                "rate",
            );
            hook_rate_limit_upsert(&provider_key, retry_after_s, until_ts.as_deref());
            return Err(AvcError::RateLimited(format!(
                "provider.llm.{}: HTTP 429",
                self.name
            )));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            hook_record(
                &provider_key,
                Status::UpstreamError,
                Some(started.elapsed().as_millis() as i64),
                "upstream",
            );
            return Err(AvcError::ProviderUpstream(format!(
                "provider.llm.{}: HTTP {} body={}",
                self.name, status, body
            )));
        }
        let parsed: ChatResponse = resp.json().await.map_err(|e| {
            let msg = format!("provider.llm.{}: bad json: {}", self.name, e);
            hook_record(
                &provider_key,
                Status::UpstreamError,
                Some(started.elapsed().as_millis() as i64),
                "upstream",
            );
            AvcError::ProviderUpstream(msg)
        })?;
        let content = parsed.choices.into_iter().next().map(|c| c.message.content);
        match content {
            Some(c) => Ok(c),
            None => {
                hook_record(
                    &provider_key,
                    Status::UpstreamError,
                    Some(started.elapsed().as_millis() as i64),
                    "upstream",
                );
                Err(AvcError::ProviderUpstream(format!(
                    "provider.llm.{}: empty choices",
                    self.name
                )))
            }
        }
    }
}

/// Provider 工厂：从 Config + 维度名构造 provider 实例。
pub fn make_llm(cfg: &Config, name: &str) -> AvcResult<Arc<dyn LlmProvider>> {
    let pc = cfg
        .provider
        .llm
        .get(name)
        .ok_or_else(|| AvcError::NotFound(format!("provider.llm.{}", name)))?;
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
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
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
    let pc = cfg
        .provider
        .embed
        .get(name)
        .ok_or_else(|| AvcError::NotFound(format!("provider.embed.{}", name)))?;
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
    // OpenAI 支持 `url` 作为 base64 的替代返回；当前实现只走 b64_json 但保留反序列化
    #[serde(default)]
    #[allow(dead_code)]
    url: Option<String>,
}

#[async_trait]
impl AvatarProvider for OpenAiCompatAvatarProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn create(&self, spec: &super::AvatarSpec) -> AvcResult<super::Avatar> {
        let model = self.cfg.model.as_deref().unwrap_or("dall-e-3");
        let url = format!(
            "{}/images/generations",
            self.base_url().trim_end_matches('/')
        );
        let body = ImgRequest {
            model,
            prompt: &spec.prompt,
            size: "1024x1024",
        };
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AvcError::ProviderTimeout(format!("avatar {} POST {}: {}", self.name, url, e))
            })?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(AvcError::TokenAuth(format!(
                "avatar.{}: HTTP {}",
                self.name, status
            )));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(AvcError::RateLimited(format!(
                "avatar.{}: HTTP 429",
                self.name
            )));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AvcError::ProviderUpstream(format!(
                "avatar.{}: HTTP {} body={}",
                self.name, status, body
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
        ref_images: &[String],
        _cfg: &super::FinetuneConfig,
    ) -> AvcResult<super::Avatar> {
        // Avatar SFT 没有 OpenAI 兼容的远端端点（DALL-E / wanx / CogView 都是 create
        // 而非 finetune）。要 finetune 就走 vendor CLI：把 cfg.binary 配成 kling-cli /
        // doubao-cli / 自家 SFT 工具；二进制读 --ref-image 列表，submit → poll → fetch
        // 出一张 PNG。
        let binary = match self.cfg.binary.as_deref() {
            Some(b) if !b.trim().is_empty() => b,
            _ => {
                return Err(AvcError::Internal(format!(
                    "avatar.{} finetune requires a vendor CLI binary; configure                      [provider.avatar.{}] binary = \"...\" in avc.toml",
                    self.name, self.name
                )));
            }
        };
        let png_bytes = run_finetune_vendor_pipeline(
            &format!("avatar.{}", self.name),
            binary,
            "--ref-image",
            ref_images,
            "png",
        )?;
        Ok(super::Avatar {
            provider: self.name.clone(),
            provider_version: "vendor-sft".into(),
            model_id: Some(format!("sft-{}", self.name)),
            primary_png_b64: base64::engine::general_purpose::STANDARD.encode(&png_bytes),
            views_zip_b64: None,
            face_id: Some(format!("sft-{}", self.name)),
        })
    }
}

pub fn make_avatar(cfg: &Config, name: &str) -> AvcResult<Arc<dyn AvatarProvider>> {
    let pc = cfg
        .provider
        .avatar
        .get(name)
        .ok_or_else(|| AvcError::NotFound(format!("provider.avatar.{}", name)))?;
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
    fn name(&self) -> &str {
        &self.name
    }

    async fn clone(&self, _ref_audio_paths: &[String]) -> AvcResult<super::Voice> {
        // OpenAI 不提供 clone/finetune（要 ElevenLabs 等 vendor）。
        // Phase 1 fallback：返占位 base64 WAV。
        Ok(super::Voice {
            provider: self.name.clone(),
            provider_version: "openai_compat".into(),
            voice_id_remote: Some(format!("mock_clone_{}", crate::svc::now_ts())),
            sample_wav_b64: base64::engine::general_purpose::STANDARD
                .encode(b"RIFF....CLONE_PLACEHOLDER"),
            transcript: Some(String::new()),
            embed_b64: Some(base64::engine::general_purpose::STANDARD.encode(vec![0u8; 16])),
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
        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| {
                AvcError::ProviderTimeout(format!("voice {} POST {}: {}", self.name, url, e))
            })?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(AvcError::TokenAuth(format!(
                "voice.{}: HTTP {}",
                self.name, status
            )));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(AvcError::RateLimited(format!(
                "voice.{}: HTTP 429",
                self.name
            )));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AvcError::ProviderUpstream(format!(
                "voice.{}: HTTP {} body={}",
                self.name, status, body
            )));
        }
        let bytes = resp.bytes().await.map_err(|e| {
            AvcError::ProviderUpstream(format!("voice.{}: read body: {}", self.name, e))
        })?;
        Ok(super::Audio {
            wav_b64: base64::engine::general_purpose::STANDARD.encode(&bytes),
            mime: "audio/wav".into(),
        })
    }

    async fn finetune(
        &self,
        _base: &super::Voice,
        ref_audio: &[String],
        _cfg: &super::FinetuneConfig,
    ) -> AvcResult<super::Voice> {
        // Voice SFT 没有 OpenAI 兼容的远端端点（tts-1 仅 synth，不接 clone）。要
        // finetune 就走 vendor CLI：把 cfg.binary 配成 elevenlabs-clone /
        // doubao-voice-clone / 自家工具；二进制读 --ref-audio 列表，submit → poll
        // → fetch 出一段 WAV。embed 由 svc::finetune::run 后续通过 embed Provider 补算。
        let binary = match self.cfg.binary.as_deref() {
            Some(b) if !b.trim().is_empty() => b,
            _ => {
                return Err(AvcError::Internal(format!(
                    "voice.{} finetune requires a vendor CLI binary; configure                      [provider.voice.{}] binary = \"...\" in avc.toml",
                    self.name, self.name
                )));
            }
        };
        let wav_bytes = run_finetune_vendor_pipeline(
            &format!("voice.{}", self.name),
            binary,
            "--ref-audio",
            ref_audio,
            "wav",
        )?;
        Ok(super::Voice {
            provider: self.name.clone(),
            provider_version: "vendor-sft".into(),
            voice_id_remote: Some(format!("sft-{}", self.name)),
            sample_wav_b64: base64::engine::general_purpose::STANDARD.encode(&wav_bytes),
            transcript: None,
            embed_b64: None,
            embed_dim: None,
        })
    }
}

pub fn make_voice(cfg: &Config, name: &str) -> AvcResult<Arc<dyn VoiceProvider>> {
    let pc = cfg
        .provider
        .voice
        .get(name)
        .ok_or_else(|| AvcError::NotFound(format!("provider.voice.{}", name)))?;
    Ok(Arc::new(OpenAiCompatVoiceProvider::new(
        name.to_string(),
        pc.clone(),
    )?))
}

// ── Cli Video Provider ─────────────────────────────────────────

/// Video provider：调用 vendor CLI（如 kling-cli）走"提交-轮询-拿 mp4"三段式。
///
/// Phase 1 没有真 binary 时 → 直接返占位 mp4 BLOB（保留向后兼容）。
/// Phase 2 用户在 `avc.toml` 的 `[provider.video.<name>]` 段设 `binary = "/usr/local/bin/kling-cli"`
/// → 真跑 submit / poll / fetch 三阶段。
///
/// 三阶段协议（与 vendor CLI 工具无关）：
/// 1. **submit** — `binary submit --prompt @script.txt --ref-image avatar.png --ref-audio voice.wav`
///    stdout 必须含 `task_id=...` 行（容许 `data:{"task_id":"..."}` JSON 也行）；
///    解析为 task_id（带前导 data: 截取）。
/// 2. **poll** — `binary status --task-id <id>`
///    stdout 必须含 `status=done|pending|failed`（容许 JSON `{"status":"done"}`）；
///    未 done 就 sleep retry，每 500ms 重试，timeout 5 分钟（可经 poll_interval_ms / poll_timeout_ms 调）。
/// 3. **fetch** — `binary fetch --task-id <id> --out <path>`
///    exit 0 时 `<path>` 是 mp4；读 bytes → base64 → 返 Clip。
///
/// 任何阶段 non-zero exit / 超时 → ProviderUpstream / ProviderTimeout。
pub struct CliVideoProvider {
    pub name: String,
    pub cfg: ProviderCfg,
}

impl CliVideoProvider {
    pub fn new(name: String, cfg: ProviderCfg) -> AvcResult<Self> {
        Ok(Self { name, cfg })
    }
}

#[async_trait]
impl VideoProvider for CliVideoProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn render(
        &self,
        voice: &super::Voice,
        avatar: &super::Avatar,
        scenes: &[super::ScriptSegment],
    ) -> AvcResult<super::Clip> {
        let total_ms: i64 = scenes.iter().map(|s| s.duration_ms).sum();

        // Phase 1 fallback：无 binary 配 → 占位 mp4 BLOB
        let binary = match self.cfg.binary.as_deref() {
            Some(b) if !b.trim().is_empty() => b,
            _ => {
                let body = format!("PLACEHOLDER_MP4:{}:{}ms", self.name, total_ms);
                return Ok(super::Clip {
                    mp4_b64: base64::engine::general_purpose::STANDARD.encode(body.as_bytes()),
                    mime: "video/mp4".into(),
                    duration_ms: total_ms,
                });
            }
        };

        // ── Phase 2: 把 exact 上游 DAG artifact（scenes text / avatar png / voice wav）
        //    materialize 成 unique 临时文件，再 spawn vendor CLI 走三段式。
        //
        // 设计要点：
        // 1. **校验 base64 在 spawn 之前**：avatar.primary_png_b64 / voice.sample_wav_b64
        //    都必须是合法 base64（容许为空 → 写空文件）。解码失败 → ProviderUpstream，
        //    不浪费 spawn。
        // 2. **unique 路径**：`<temp>/avc-<provider>-<kind>-<pid>-<nanos>.<ext>`；并发
        //    render 不会互相覆盖。fetch 用同名 mp4；fetch 后再读取 + 删除。
        // 3. **RAII 守卫 TempFileGuard**：Drop 时一次性删除所有临时文件，覆盖成功 + 所有
        //    error return + panic unwinds。
        // 4. **submit 用真实文件路径**：vendor 协议约定 `--prompt @<path>` 让它自己读，
        //    `--ref-image <path>` / `--ref-audio <path>` 同理。
        let script_bytes = serialize_scenes(scenes);
        let png_bytes = base64::engine::general_purpose::STANDARD
            .decode(avatar.primary_png_b64.as_bytes())
            .map_err(|e| {
                AvcError::ProviderUpstream(format!(
                    "video.{}: avatar.primary_png_b64 invalid base64: {}",
                    self.name, e
                ))
            })?;
        let wav_bytes = base64::engine::general_purpose::STANDARD
            .decode(voice.sample_wav_b64.as_bytes())
            .map_err(|e| {
                AvcError::ProviderUpstream(format!(
                    "video.{}: voice.sample_wav_b64 invalid base64: {}",
                    self.name, e
                ))
            })?;

        let unique = unique_suffix();
        let script_path =
            std::env::temp_dir().join(format!("avc-{}-script-{}.txt", self.name, unique));
        let image_path =
            std::env::temp_dir().join(format!("avc-{}-avatar-{}.png", self.name, unique));
        let audio_path =
            std::env::temp_dir().join(format!("avc-{}-voice-{}.wav", self.name, unique));
        let fetch_path =
            std::env::temp_dir().join(format!("avc-{}-fetch-{}.mp4", self.name, unique));

        // 先全部写出来。任何一个 IO 失败 → guard 在 drop 时清掉已写的剩余文件。
        write_temp_file(&script_path, &script_bytes).map_err(|e| {
            AvcError::ProviderUpstream(format!(
                "video.{}: write script {}: {}",
                self.name,
                script_path.display(),
                e
            ))
        })?;
        write_temp_file(&image_path, &png_bytes).map_err(|e| {
            AvcError::ProviderUpstream(format!(
                "video.{}: write avatar png {}: {}",
                self.name,
                image_path.display(),
                e
            ))
        })?;
        write_temp_file(&audio_path, &wav_bytes).map_err(|e| {
            AvcError::ProviderUpstream(format!(
                "video.{}: write voice wav {}: {}",
                self.name,
                audio_path.display(),
                e
            ))
        })?;

        let _guard = TempFileGuard::new(vec![
            script_path.clone(),
            image_path.clone(),
            audio_path.clone(),
            fetch_path.clone(),
        ]);

        // 1. submit — 协议：`--prompt @<script-path>`，ref-image/ref-audio 直接传路径。
        let script_arg = format!("@{}", script_path.display());
        let image_arg = image_path.display().to_string();
        let audio_arg = audio_path.display().to_string();
        let submit_argv = [
            "submit".to_string(),
            "--prompt".to_string(),
            script_arg,
            "--ref-image".to_string(),
            image_arg,
            "--ref-audio".to_string(),
            audio_arg,
        ];
        let submit_argv_ref: Vec<&str> = submit_argv.iter().map(String::as_str).collect();
        let submit_out = run_vendor_cmd(binary, &submit_argv_ref)?;
        let task_id = parse_field(&submit_out, "task_id").ok_or_else(|| {
            AvcError::ProviderUpstream(format!(
                "video.{}: cannot parse task_id from submit stdout: {:?}",
                self.name, submit_out
            ))
        })?;

        // 2. poll
        let poll_started = std::time::Instant::now();
        let poll_timeout = std::time::Duration::from_secs(300);
        let poll_interval = std::time::Duration::from_millis(500);
        loop {
            let poll_out = run_vendor_cmd(binary, &["status", "--task-id", &task_id])?;
            let status = parse_field(&poll_out, "status").unwrap_or_default();
            if status == "done" {
                break;
            }
            if status == "failed" {
                return Err(AvcError::ProviderUpstream(format!(
                    "video.{} task {} failed",
                    self.name, task_id
                )));
            }
            if poll_started.elapsed() > poll_timeout {
                return Err(AvcError::ProviderTimeout(format!(
                    "video.{} task {} poll timeout ({}s)",
                    self.name,
                    task_id,
                    poll_timeout.as_secs()
                )));
            }
            std::thread::sleep(poll_interval);
        }

        // 3. fetch — 把 mp4 写到我们提供的 fetch_path
        let fetch_arg = fetch_path.display().to_string();
        let fetch_argv = [
            "fetch".to_string(),
            "--task-id".to_string(),
            task_id.clone(),
            "--out".to_string(),
            fetch_arg,
        ];
        let fetch_argv_ref: Vec<&str> = fetch_argv.iter().map(String::as_str).collect();
        run_vendor_cmd(binary, &fetch_argv_ref)?;

        let bytes = std::fs::read(&fetch_path).map_err(|e| {
            AvcError::ProviderUpstream(format!(
                "video.{}: read fetched mp4 {}: {}",
                self.name,
                fetch_path.display(),
                e
            ))
        })?;
        if bytes.is_empty() {
            return Err(AvcError::ProviderUpstream(format!(
                "video.{}: fetched mp4 {} is empty",
                self.name,
                fetch_path.display()
            )));
        }

        // guard 在此函数末尾 drop → 删 4 个 tmp file（含 fetch_path）。
        Ok(super::Clip {
            mp4_b64: base64::engine::general_purpose::STANDARD.encode(&bytes),
            mime: "video/mp4".into(),
            duration_ms: total_ms,
        })
    }
}

/// 把 `ScriptSegment` 列表序列化成 vendor CLI 可消费的纯文本 prompt：
/// 每行 `<scene_index>: <text>`，末尾空行。空 scenes → 空 bytes（仍写空文件）。
fn serialize_scenes(scenes: &[super::ScriptSegment]) -> Vec<u8> {
    let mut s = String::new();
    for seg in scenes {
        s.push_str(&format!("{}: {}\n", seg.scene_index, seg.text));
    }
    s.into_bytes()
}

/// 生成 PID + nanos 后缀，保证多个并发 render 不会撞名。
fn unique_suffix() -> String {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{}", pid, nanos)
}

fn write_temp_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

/// Drop 时一次性删除一组临时文件。删除失败不报错（best-effort）——文件可能已被外部
/// fetch / 用户手动清理，但仍要把我们写入的剩余文件清掉。
struct TempFileGuard {
    paths: Vec<std::path::PathBuf>,
}

impl TempFileGuard {
    fn new(paths: Vec<std::path::PathBuf>) -> Self {
        Self { paths }
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        for p in &self.paths {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Spawn vendor CLI binary + 一组 args，返 stdout（trim 后）。
/// non-zero exit → ProviderUpstream；启动失败（NotFound / Permission）→ 同上。
fn run_vendor_cmd(binary: &str, args: &[&str]) -> AvcResult<String> {
    let out = std::process::Command::new(binary)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| {
            AvcError::ProviderUpstream(format!("spawn {} {}: {}", binary, args.join(" "), e))
        })?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        return Err(AvcError::ProviderUpstream(format!(
            "{} {} exit {:?}: stdout={} stderr={}",
            binary,
            args.join(" "),
            out.status.code(),
            stdout,
            stderr
        )));
    }
    Ok(stdout)
}

/// stdout 是自由格式的输出。容忍三种解析：
/// 1. `key=value` 行（KV-flavor — 推荐用于 shell 包装）
/// 2. JSON: `{"key":"value"...}` 或 `data:{"key":"value"...}`（vendor 通常 streaming）
/// 3. 单 token（视为 task_id / status）
fn parse_field(stdout: &str, key: &str) -> Option<String> {
    for line in stdout.lines() {
        let line = line.trim();
        // 1. KV
        if let Some(rest) = line.strip_prefix(&format!("{}=", key)) {
            return Some(rest.trim().to_string());
        }
        // 2. JSON
        if line.starts_with("data:") || line.starts_with('{') {
            let l = line.trim_start_matches("data:").trim();
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(l) {
                if let Some(val) = v.get(key) {
                    if let Some(s) = val.as_str() {
                        return Some(s.to_string());
                    }
                    if let Some(n) = val.as_i64() {
                        return Some(n.to_string());
                    }
                }
            }
        }
    }
    None
}

/// 走 vendor CLI SFT 协议（`finetune submit / status / fetch` 三段式）。
///
/// 协议与 CliVideoProvider 的 `submit / status / fetch` 一致，仅多了一层
/// `finetune` 子命令命名空间（避免与 video 渲染 `submit` 撞名）：
///
/// 1. `binary finetune submit --<ref_flag> <ref_path_1> [<ref_path_2> ...]`
///    stdout 必须含 `task_id=...`（KV-flavor）；也容许 `data:{"task_id":"..."}` JSON。
/// 2. `binary finetune status --task-id <id>`
///    stdout 必须含 `status=done|pending|failed`；未 done 就 sleep 500ms 重试，
///    5 min 超时。
/// 3. `binary finetune fetch --task-id <id> --out <output_path>`
///    exit 0 时 `<output_path>` 是结果文件（avatar 走 `png`、voice 走 `wav`）。
///    读 bytes 返 Vec<u8>；调用方负责 base64 编码。
///
/// `ref_flag` 是 submit 那一阶段用于标 ref 素材的开关名：
///   - 调 avatar.finetune 时是 `"--ref-image"`
///   - 调 voice.finetune 时是 `"--ref-audio"`
///
/// 临时文件命名：`<temp>/avc-<provider>-finetune-<kind>-<pid>-<nanos>.<ext>`，
/// 并发跑不会撞；返回的字节来自 `--out` 路径，fetch 后由 guard 在 drop 时清掉。
fn run_finetune_vendor_pipeline(
    provider_name: &str,
    binary: &str,
    ref_flag: &str,
    ref_paths: &[String],
    output_ext: &str,
) -> AvcResult<Vec<u8>> {
    if ref_paths.is_empty() {
        return Err(AvcError::ProviderUpstream(format!(
            "{}.finetune: no ref paths provided ({} must be non-empty)",
            provider_name, ref_flag
        )));
    }
    let unique = unique_suffix();
    let out_path = std::env::temp_dir().join(format!(
        "avc-{}-finetune-{}-{}.{}",
        provider_name,
        ref_flag.trim_start_matches("--"),
        unique,
        output_ext
    ));
    let _guard = TempFileGuard::new(vec![out_path.clone()]);

    // 1. submit
    let mut submit_argv: Vec<String> = vec!["finetune".into(), "submit".into()];
    submit_argv.push(ref_flag.to_string());
    for p in ref_paths {
        submit_argv.push(p.clone());
    }
    let submit_refs: Vec<&str> = submit_argv.iter().map(String::as_str).collect();
    let submit_out = run_vendor_cmd(binary, &submit_refs)?;
    let task_id = parse_field(&submit_out, "task_id").ok_or_else(|| {
        AvcError::ProviderUpstream(format!(
            "{}.finetune: cannot parse task_id from submit stdout: {:?}",
            provider_name, submit_out
        ))
    })?;

    // 2. poll
    let poll_started = std::time::Instant::now();
    let poll_timeout = std::time::Duration::from_secs(300);
    let poll_interval = std::time::Duration::from_millis(500);
    loop {
        let poll_out = run_vendor_cmd(binary, &["finetune", "status", "--task-id", &task_id])?;
        let status = parse_field(&poll_out, "status").unwrap_or_default();
        if status == "done" {
            break;
        }
        if status == "failed" {
            return Err(AvcError::ProviderUpstream(format!(
                "{}.finetune task {} failed",
                provider_name, task_id
            )));
        }
        if poll_started.elapsed() > poll_timeout {
            return Err(AvcError::ProviderTimeout(format!(
                "{}.finetune task {} poll timeout ({}s)",
                provider_name,
                task_id,
                poll_timeout.as_secs()
            )));
        }
        std::thread::sleep(poll_interval);
    }

    // 3. fetch
    let fetch_argv = [
        "finetune".to_string(),
        "fetch".to_string(),
        "--task-id".to_string(),
        task_id.clone(),
        "--out".to_string(),
        out_path.display().to_string(),
    ];
    let fetch_refs: Vec<&str> = fetch_argv.iter().map(String::as_str).collect();
    run_vendor_cmd(binary, &fetch_refs)?;

    let bytes = std::fs::read(&out_path).map_err(|e| {
        AvcError::ProviderUpstream(format!(
            "{}.finetune: read fetched {} {}: {}",
            provider_name,
            output_ext,
            out_path.display(),
            e
        ))
    })?;
    if bytes.is_empty() {
        return Err(AvcError::ProviderUpstream(format!(
            "{}.finetune: fetched {} is empty",
            provider_name, output_ext
        )));
    }
    // guard 在此函数末尾 drop → 删 fetch 出的 output 文件
    Ok(bytes)
}

pub fn make_video(cfg: &Config, name: &str) -> AvcResult<Arc<dyn VideoProvider>> {
    let pc = cfg
        .provider
        .video
        .get(name)
        .ok_or_else(|| AvcError::NotFound(format!("provider.video.{}", name)))?;
    Ok(Arc::new(CliVideoProvider::new(
        name.to_string(),
        pc.clone(),
    )?))
}

#[cfg(test)]
mod provider_factory_tests {
    use super::*;

    #[test]
    fn avatar_factory_returns_404_for_unknown_name() {
        let cfg = Config::default();
        assert!(matches!(
            make_avatar(&cfg, "ghost"),
            Err(AvcError::NotFound(_))
        ));
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
        assert!(matches!(
            make_voice(&cfg, "ghost"),
            Err(AvcError::NotFound(_))
        ));
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
        assert!(matches!(
            make_video(&cfg, "ghost"),
            Err(AvcError::NotFound(_))
        ));
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

    /// Phase 2：spawn vendor CLI 三段式（submit → poll → fetch）。
    /// Mock binary：必须精确读到 `--prompt @<path>` 指向的脚本文件、--ref-image 指向的
    /// avatar png、--ref-audio 指向的 voice wav；并把内容写到 --out。
    /// 缺任何 upstream 文件 / ref 都退出 3，避免 regression 静默通过（failed accounting）。
    #[test]
    fn cli_video_calls_binary_succeeds() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let bin = dir.path().join("mock_video_cli.sh");
        // submit: stdout task_id=xxx (KV-flavor)；status: stdout status=done；
        // fetch: 写真到 --out 路径，exit 0。
        std::fs::write(
            &bin,
            "#!/bin/sh
set -e
case \"$1\" in
  submit)
    # 提取 --prompt 后的文件名（不真读，仅 echo token）
    echo \"task_id=mock-task-1\"
    ;;
  status)
    echo \"status=done\"
    ;;
  fetch)
    # 找 --out 后的值写真
    while [ \"$#\" -gt 0 ]; do
      case \"$1\" in
        --out) OUT=\"$2\"; shift 2;;
        *) shift;;
      esac
    done
    mkdir -p \"$(dirname \"$OUT\")\"
    printf 'MOCK_VIDEO_mp4_magic_ftyp' > \"$OUT\"
    # 写满点字节让 fetch 真读到非空
    head -c 1024 /dev/urandom >> \"$OUT\"
    ;;
  *)
    echo \"unknown subcommand: $1\" >&2
    exit 2
    ;;
esac
",
        )
        .expect("write mock bin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut cfg = Config::default();
        let pc = ProviderCfg {
            binary: Some(bin.to_str().unwrap().to_string()),
            model: Some("mock".into()),
            ..Default::default()
        };
        cfg.provider.video.insert("mock".into(), pc);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let p = make_video(&cfg, "mock").expect("provider");
            let voice = crate::provider::Voice {
                provider: "mock".into(),
                provider_version: "stub".into(),
                voice_id_remote: None,
                sample_wav_b64: String::new(),
                transcript: None,
                embed_b64: None,
                embed_dim: None,
            };
            let avatar = crate::provider::Avatar {
                provider: "mock".into(),
                provider_version: "stub".into(),
                model_id: None,
                primary_png_b64: String::new(),
                views_zip_b64: None,
                face_id: None,
            };
            let scenes = vec![crate::provider::ScriptSegment {
                scene_index: 0,
                text: "hi".into(),
                duration_ms: 1000,
            }];
            let clip = p.render(&voice, &avatar, &scenes).await.expect("render ok");
            assert!(!clip.mp4_b64.is_empty());
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&clip.mp4_b64)
                .expect("b64");
            // 写真 ≈ 1024 bytes + magic header
            assert!(bytes.len() >= 100);
            // 第一段是 mp4 ft_magic
            assert!(bytes.starts_with(b"MOCK_VIDEO_mp4_magic_ftyp") || bytes.starts_with(b"MOCK"));
        });
    }

    #[test]
    fn cli_video_binary_subprocess_failure_returns_provider_upstream() {
        // binary 退出码 != 0 → ProviderUpstream
        let dir = tempfile::tempdir().expect("tmpdir");
        let bin = dir.path().join("fail.sh");
        std::fs::write(&bin, "#!/bin/sh\necho boom >&2\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut cfg = Config::default();
        let pc = ProviderCfg {
            binary: Some(bin.to_str().unwrap().to_string()),
            ..Default::default()
        };
        cfg.provider.video.insert("mock".into(), pc);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let p = make_video(&cfg, "mock").expect("provider");
            let voice = crate::provider::Voice {
                provider: "mock".into(),
                provider_version: "stub".into(),
                voice_id_remote: None,
                sample_wav_b64: String::new(),
                transcript: None,
                embed_b64: None,
                embed_dim: None,
            };
            let avatar = crate::provider::Avatar {
                provider: "mock".into(),
                provider_version: "stub".into(),
                model_id: None,
                primary_png_b64: String::new(),
                views_zip_b64: None,
                face_id: None,
            };
            let scenes = vec![crate::provider::ScriptSegment {
                scene_index: 0,
                text: "hi".into(),
                duration_ms: 1000,
            }];
            let res = p.render(&voice, &avatar, &scenes).await;
            assert!(matches!(
                res,
                Err(crate::error::AvcError::ProviderUpstream(_))
            ));
        });
    }

    #[test]
    fn cli_video_binary_missing_returns_provider_upstream() {
        // binary 路径不存在 → ProviderUpstream (spawn NotFound)
        let mut cfg = Config::default();
        let pc = ProviderCfg {
            binary: Some("/nonexistent/path/to/binary".into()),
            ..Default::default()
        };
        cfg.provider.video.insert("mock".into(), pc);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let p = make_video(&cfg, "mock").expect("provider");
            let voice = crate::provider::Voice {
                provider: "mock".into(),
                provider_version: "stub".into(),
                voice_id_remote: None,
                sample_wav_b64: String::new(),
                transcript: None,
                embed_b64: None,
                embed_dim: None,
            };
            let avatar = crate::provider::Avatar {
                provider: "mock".into(),
                provider_version: "stub".into(),
                model_id: None,
                primary_png_b64: String::new(),
                views_zip_b64: None,
                face_id: None,
            };
            let scenes = vec![crate::provider::ScriptSegment {
                scene_index: 0,
                text: "x".into(),
                duration_ms: 1,
            }];
            assert!(matches!(
                p.render(&voice, &avatar, &scenes).await,
                Err(crate::error::AvcError::ProviderUpstream(_))
            ));
        });
    }

    // ── avatar vendor finetune ─────────────────────────────────────

    /// avatar finetune 走 vendor CLI：mock binary 接受 `--ref-image <path>` 列表，
    /// submit 输出 task_id=...，status 输出 status=done，fetch 把 PNG 写到 --out。
    #[test]
    fn cli_avatar_finetune_calls_binary_succeeds() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let bin = dir.path().join("mock_avatar_finetune.sh");
        std::fs::write(
            &bin,
            "#!/bin/sh
set -e
case \"$1\" in
  finetune)
    case \"$2\" in
      submit)
        # 第三个 token 必为 --ref-image（avatar 协议）
        if [ \"$3\" != \"--ref-image\" ]; then
          echo \"avatar finetune submit: expected --ref-image, got $3\" >&2
          exit 2
        fi
        # 至少要 1 个 ref-image 路径（参 caller 传的非空切片）
        if [ \"$#\" -lt 4 ]; then
          echo \"avatar finetune submit: --ref-image requires at least 1 path\" >&2
          exit 2
        fi
        echo \"task_id=mock-avatar-ft-1\"
        ;;
      status)
        echo \"status=done\"
        ;;
      fetch)
        OUT=\"\"
        while [ \"$#\" -gt 0 ]; do
          case \"$1\" in
            --out) OUT=\"$2\"; shift 2;;
            *) shift;;
          esac
        done
        mkdir -p \"$(dirname \"$OUT\")\"
        printf 'MOCK_AVATAR_SFT_PNG_ftyp' > \"$OUT\"
        head -c 256 /dev/urandom >> \"$OUT\"
        ;;
      *) echo \"unknown finetune sub: $2\" >&2; exit 2 ;;
    esac
    ;;
  *) echo \"unknown sub: $1\" >&2; exit 2 ;;
esac
",
        )
        .expect("write mock bin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut cfg = Config::default();
        let pc = ProviderCfg {
            binary: Some(bin.to_str().unwrap().to_string()),
            ..Default::default()
        };
        cfg.provider.avatar.insert("mock".into(), pc);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let p = make_avatar(&cfg, "mock").expect("provider");
            let base = crate::provider::Avatar {
                provider: "mock".into(),
                provider_version: "stub".into(),
                model_id: None,
                primary_png_b64: String::new(),
                views_zip_b64: None,
                face_id: None,
            };
            // 写 2 个空 ref 文件 mock vendor 看得到的训练素材
            let ref1 = dir.path().join("ref1.png");
            let ref2 = dir.path().join("ref2.png");
            std::fs::write(&ref1, b"ref1").unwrap();
            std::fs::write(&ref2, b"ref2").unwrap();
            let refs = vec![
                ref1.to_str().unwrap().to_string(),
                ref2.to_str().unwrap().to_string(),
            ];
            let out = p
                .finetune(
                    &base,
                    &refs,
                    &super::super::FinetuneConfig {
                        full_retrain: false,
                        epochs: 1,
                        consistency_threshold: 0.85,
                    },
                )
                .await
                .expect("finetune ok");
            assert_eq!(out.provider, "mock");
            assert_eq!(out.provider_version, "vendor-sft");
            assert!(out.primary_png_b64.len() > 50);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&out.primary_png_b64)
                .expect("b64");
            assert!(bytes.starts_with(b"MOCK_AVATAR_SFT_PNG_ftyp"));
            assert!(bytes.len() >= 256);
        });
    }

    /// avatar finetune 但没配 binary → 明确报 "requires a vendor CLI binary"，
    /// 不静默走 OpenAI 端点（OpenAI 没有 SFT 端点）。
    #[test]
    fn cli_avatar_finetune_no_binary_returns_internal() {
        let mut cfg = Config::default();
        cfg.provider.avatar.insert(
            "kling".into(),
            ProviderCfg {
                api_key: Some("sk".into()),
                ..Default::default()
            },
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let p = make_avatar(&cfg, "kling").expect("provider");
            let base = crate::provider::Avatar {
                provider: "kling".into(),
                provider_version: "stub".into(),
                model_id: None,
                primary_png_b64: String::new(),
                views_zip_b64: None,
                face_id: None,
            };
            let refs = vec!["/tmp/r.png".to_string()];
            let res = p
                .finetune(
                    &base,
                    &refs,
                    &super::super::FinetuneConfig {
                        full_retrain: false,
                        epochs: 1,
                        consistency_threshold: 0.85,
                    },
                )
                .await;
            assert!(
                matches!(res, Err(crate::error::AvcError::Internal(_))),
                "expected Internal, got {:?}",
                res.map(|_| "ok")
            );
        });
    }

    /// avatar finetune 但 ref_images 为空 → ProviderUpstream（mock shell 看 --ref-image
    /// 后跟路径数；空切片不应 spawn）。
    #[test]
    fn cli_avatar_finetune_empty_refs_returns_provider_upstream() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let bin = dir.path().join("mock_avatar_finetune.sh");
        std::fs::write(&bin, "#!/bin/sh\nexit 99\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut cfg = Config::default();
        let pc = ProviderCfg {
            binary: Some(bin.to_str().unwrap().to_string()),
            ..Default::default()
        };
        cfg.provider.avatar.insert("mock".into(), pc);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let p = make_avatar(&cfg, "mock").expect("provider");
            let base = crate::provider::Avatar {
                provider: "mock".into(),
                provider_version: "stub".into(),
                model_id: None,
                primary_png_b64: String::new(),
                views_zip_b64: None,
                face_id: None,
            };
            let res = p
                .finetune(
                    &base,
                    &[],
                    &super::super::FinetuneConfig {
                        full_retrain: false,
                        epochs: 1,
                        consistency_threshold: 0.85,
                    },
                )
                .await;
            assert!(
                matches!(res, Err(crate::error::AvcError::ProviderUpstream(_))),
                "expected ProviderUpstream, got {:?}",
                res.map(|_| "ok")
            );
        });
    }

    // ── voice vendor finetune ──────────────────────────────────────

    /// voice finetune 走 vendor CLI：mock binary 接受 `--ref-audio <path>` 列表，
    /// submit 输出 task_id=...，fetch 把 WAV 写到 --out。
    #[test]
    fn cli_voice_finetune_calls_binary_succeeds() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let bin = dir.path().join("mock_voice_finetune.sh");
        std::fs::write(
            &bin,
            "#!/bin/sh
set -e
case \"$1\" in
  finetune)
    case \"$2\" in
      submit)
        if [ \"$3\" != \"--ref-audio\" ]; then
          echo \"voice finetune submit: expected --ref-audio, got $3\" >&2
          exit 2
        fi
        if [ \"$#\" -lt 4 ]; then
          echo \"voice finetune submit: --ref-audio requires at least 1 path\" >&2
          exit 2
        fi
        echo \"task_id=mock-voice-ft-1\"
        ;;
      status)
        echo \"status=done\"
        ;;
      fetch)
        OUT=\"\"
        while [ \"$#\" -gt 0 ]; do
          case \"$1\" in
            --out) OUT=\"$2\"; shift 2;;
            *) shift;;
          esac
        done
        mkdir -p \"$(dirname \"$OUT\")\"
        printf 'MOCK_VOICE_SFT_WAV_RIFF' > \"$OUT\"
        head -c 256 /dev/urandom >> \"$OUT\"
        ;;
      *) echo \"unknown finetune sub: $2\" >&2; exit 2 ;;
    esac
    ;;
  *) echo \"unknown sub: $1\" >&2; exit 2 ;;
esac
",
        )
        .expect("write mock bin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut cfg = Config::default();
        let pc = ProviderCfg {
            binary: Some(bin.to_str().unwrap().to_string()),
            ..Default::default()
        };
        cfg.provider.voice.insert("mock".into(), pc);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let p = make_voice(&cfg, "mock").expect("provider");
            let base = crate::provider::Voice {
                provider: "mock".into(),
                provider_version: "stub".into(),
                voice_id_remote: None,
                sample_wav_b64: String::new(),
                transcript: None,
                embed_b64: None,
                embed_dim: None,
            };
            let ref1 = dir.path().join("ref1.wav");
            let ref2 = dir.path().join("ref2.wav");
            std::fs::write(&ref1, b"ref1").unwrap();
            std::fs::write(&ref2, b"ref2").unwrap();
            let refs = vec![
                ref1.to_str().unwrap().to_string(),
                ref2.to_str().unwrap().to_string(),
            ];
            let out = p
                .finetune(
                    &base,
                    &refs,
                    &super::super::FinetuneConfig {
                        full_retrain: false,
                        epochs: 1,
                        consistency_threshold: 0.85,
                    },
                )
                .await
                .expect("finetune ok");
            assert_eq!(out.provider, "mock");
            assert_eq!(out.provider_version, "vendor-sft");
            assert!(out.voice_id_remote.is_some());
            assert!(out.sample_wav_b64.len() > 50);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&out.sample_wav_b64)
                .expect("b64");
            assert!(bytes.starts_with(b"MOCK_VOICE_SFT_WAV_RIFF"));
        });
    }

    /// voice finetune 没配 binary → Internal（"requires a vendor CLI binary"）。
    #[test]
    fn cli_voice_finetune_no_binary_returns_internal() {
        let mut cfg = Config::default();
        cfg.provider.voice.insert(
            "elevenlabs".into(),
            ProviderCfg {
                api_key: Some("sk".into()),
                ..Default::default()
            },
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let p = make_voice(&cfg, "elevenlabs").expect("provider");
            let base = crate::provider::Voice {
                provider: "elevenlabs".into(),
                provider_version: "stub".into(),
                voice_id_remote: None,
                sample_wav_b64: String::new(),
                transcript: None,
                embed_b64: None,
                embed_dim: None,
            };
            let refs = vec!["/tmp/r.wav".to_string()];
            let res = p
                .finetune(
                    &base,
                    &refs,
                    &super::super::FinetuneConfig {
                        full_retrain: false,
                        epochs: 1,
                        consistency_threshold: 0.85,
                    },
                )
                .await;
            assert!(
                matches!(res, Err(crate::error::AvcError::Internal(_))),
                "expected Internal, got {:?}",
                res.map(|_| "ok")
            );
        });
    }

    /// voice finetune binary exit != 0 → ProviderUpstream。
    #[test]
    fn cli_voice_finetune_binary_failure_returns_provider_upstream() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let bin = dir.path().join("fail.sh");
        std::fs::write(&bin, "#!/bin/sh\necho boom >&2\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut cfg = Config::default();
        let pc = ProviderCfg {
            binary: Some(bin.to_str().unwrap().to_string()),
            ..Default::default()
        };
        cfg.provider.voice.insert("mock".into(), pc);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let p = make_voice(&cfg, "mock").expect("provider");
            let base = crate::provider::Voice {
                provider: "mock".into(),
                provider_version: "stub".into(),
                voice_id_remote: None,
                sample_wav_b64: String::new(),
                transcript: None,
                embed_b64: None,
                embed_dim: None,
            };
            let refs = vec!["/tmp/r.wav".to_string()];
            let res = p
                .finetune(
                    &base,
                    &refs,
                    &super::super::FinetuneConfig {
                        full_retrain: false,
                        epochs: 1,
                        consistency_threshold: 0.85,
                    },
                )
                .await;
            assert!(matches!(
                res,
                Err(crate::error::AvcError::ProviderUpstream(_))
            ));
        });
    }
}
