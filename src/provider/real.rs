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

use super::{ChatMessage, EmbedProvider, LlmProvider};
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
