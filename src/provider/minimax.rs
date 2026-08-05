//! MiniMax 专有 API 适配：avatar / voice / video
//!
//! 详见 docs/superpowers/specs/2026-08-04-minimax-provider-design.md

#![allow(unused_imports)] // T2/T3 会用到 voice/video 的导入；T1 暂时不用

use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::config::ProviderCfg;
use crate::error::{AvcError, AvcResult};
use crate::provider::{
    Audio, Avatar, AvatarProvider, AvatarSpec, FinetuneConfig, VideoProvider, VoiceProvider,
};

const DEFAULT_BASE_URL: &str = "https://api.minimaxi.com";

/// Build a HeaderMap with the Authorization: Bearer <api_key> header.
/// If `api_key` is empty, no header is set (some local compat servers allow anonymous).
pub fn auth_header(api_key: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&format!("Bearer {}", api_key)) {
        h.insert(AUTHORIZATION, v);
    }
    h
}

/// Decode a hex-encoded string (MiniMax encodes audio bytes as hex, not base64).
pub fn decode_hex_audio(hex: &str) -> AvcResult<Vec<u8>> {
    hex::decode(hex).map_err(|e| AvcError::Internal(format!("hex decode: {}", e)))
}

/// Inspect a MiniMax response: parse JSON, check `base_resp.status_code` (0 = success, 2013 = invalid params).
/// Translates MiniMax errors to AvcError variants:
///   HTTP 401 → AvcError::TokenAuth
///   HTTP 429 → AvcError::RateLimited
///   HTTP 5xx → AvcError::ProviderUpstream
///   base_resp.status_code = 2013 → AvcError::Arg
///   other base_resp.status_code != 0 → AvcError::ProviderUpstream
pub async fn handle_response<T: DeserializeOwned>(resp: reqwest::Response) -> AvcResult<T> {
    let status = resp.status();
    if status.as_u16() == 401 {
        return Err(AvcError::TokenAuth("minimax HTTP 401".into()));
    }
    if status.as_u16() == 429 {
        return Err(AvcError::RateLimited("minimax HTTP 429".into()));
    }
    if !status.is_success() {
        return Err(AvcError::ProviderUpstream(format!(
            "minimax HTTP {}",
            status
        )));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AvcError::Internal(format!("minimax json decode: {}", e)))?;
    if let Some(b) = body.get("base_resp") {
        let code = b.get("status_code").and_then(|v| v.as_i64()).unwrap_or(0);
        if code != 0 {
            let msg = b
                .get("status_msg")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            return if code == 2013 {
                Err(AvcError::Arg(format!("minimax invalid params: {}", msg)))
            } else {
                Err(AvcError::ProviderUpstream(format!(
                    "minimax code {}: {}",
                    code, msg
                )))
            };
        }
    }
    serde_json::from_value(body).map_err(|e| AvcError::Internal(format!("minimax decode: {}", e)))
}

// ── Avatar ──────────────────────────────────────────

pub struct MiniMaxCompatAvatarProvider {
    pub name: String,
    pub cfg: ProviderCfg,
    client: Client,
    base_url: String,
}

impl MiniMaxCompatAvatarProvider {
    pub fn new(name: String, cfg: ProviderCfg) -> AvcResult<Self> {
        let headers = auth_header(cfg.api_key.as_deref().unwrap_or(""));
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| AvcError::Internal(format!("reqwest builder: {}", e)))?;
        let base_url = cfg
            .base_url
            .clone()
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        Ok(Self {
            name,
            cfg,
            client,
            base_url,
        })
    }
}

#[async_trait]
impl AvatarProvider for MiniMaxCompatAvatarProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn create(&self, spec: &AvatarSpec) -> AvcResult<Avatar> {
        let body = serde_json::json!({
            "model": self.cfg.model.as_deref().unwrap_or("image-01"),
            "prompt": spec.prompt,
            "n": 1,
            "aspect_ratio": "1:1",
            "response_format": "url",
            "prompt_enhancer": true,
        });
        let url = format!("{}/v1/image_generation", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| AvcError::ProviderUpstream(e.to_string()))?;

        #[derive(Deserialize)]
        struct ImageResp {
            data: ImageData,
            #[allow(dead_code)]
            base_resp: serde_json::Value,
        }
        #[derive(Deserialize)]
        struct ImageData {
            image_urls: Vec<String>,
        }
        let parsed: ImageResp = handle_response(resp).await?;
        let image_url = parsed
            .data
            .image_urls
            .into_iter()
            .next()
            .ok_or_else(|| AvcError::ProviderUpstream("no image_urls in response".into()))?;

        // 下载图片到 BLOB
        let bytes = self
            .client
            .get(&image_url)
            .send()
            .await
            .map_err(|e| AvcError::ProviderUpstream(e.to_string()))?
            .bytes()
            .await
            .map_err(|e| AvcError::Internal(e.to_string()))?;

        Ok(Avatar {
            provider: self.name.clone(),
            provider_version: "v1".into(),
            model_id: self.cfg.model.clone(),
            primary_png_b64: base64::engine::general_purpose::STANDARD.encode(&bytes),
            views_zip_b64: None,
            face_id: None,
        })
    }

    async fn finetune(
        &self,
        _base: &Avatar,
        ref_images: &[String],
        _cfg: &FinetuneConfig,
    ) -> AvcResult<Avatar> {
        // minimax avatar finetune not implemented; use vendor CLI
        let _ = ref_images;
        Err(AvcError::Internal(
            "minimax avatar finetune not implemented; use vendor CLI".into(),
        ))
    }
}

// ── Voice ──────────────────────────────────────────

pub struct MiniMaxCompatVoiceProvider {
    pub name: String,
    pub cfg: ProviderCfg,
    client: Client,
    base_url: String,
}

impl MiniMaxCompatVoiceProvider {
    pub fn new(name: String, cfg: ProviderCfg) -> AvcResult<Self> {
        let headers = auth_header(cfg.api_key.as_deref().unwrap_or(""));
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| AvcError::Internal(format!("reqwest builder: {}", e)))?;
        let base_url = cfg
            .base_url
            .clone()
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        Ok(Self {
            name,
            cfg,
            client,
            base_url,
        })
    }
}

#[async_trait]
impl VoiceProvider for MiniMaxCompatVoiceProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn clone(&self, _ref_audio_paths: &[String]) -> AvcResult<crate::provider::Voice> {
        // minimax voice clone not implemented; use vendor CLI
        Err(AvcError::Internal(
            "minimax voice clone not implemented; use vendor CLI".into(),
        ))
    }

    async fn synth(&self, voice: &crate::provider::Voice, text: &str) -> AvcResult<Audio> {
        let body = serde_json::json!({
            "model": self.cfg.model.as_deref().unwrap_or("speech-01-turbo"),
            "text": text,
            "voice_setting": {
                "voice_id": voice.voice_id_remote.as_deref().unwrap_or("male-qn-qingse"),
            },
            "audio_setting": { "format": "mp3" },
        });
        let url = format!("{}/v1/t2a_v2", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| AvcError::ProviderUpstream(e.to_string()))?;

        #[derive(Deserialize)]
        struct TtsResp {
            data: TtsData,
            #[allow(dead_code)]
            base_resp: serde_json::Value,
        }
        #[derive(Deserialize)]
        struct TtsData {
            audio: String, // hex-encoded MP3
        }
        let parsed: TtsResp = handle_response(resp).await?;
        let mp3_bytes = decode_hex_audio(&parsed.data.audio)?;
        Ok(Audio {
            wav_b64: base64::engine::general_purpose::STANDARD.encode(&mp3_bytes),
            mime: "audio/mpeg".into(),
        })
    }

    async fn finetune(
        &self,
        _base: &crate::provider::Voice,
        _ref_audio: &[String],
        _cfg: &crate::provider::FinetuneConfig,
    ) -> AvcResult<crate::provider::Voice> {
        // minimax voice finetune not implemented; use vendor CLI
        Err(AvcError::Internal(
            "minimax voice finetune not implemented; use vendor CLI".into(),
        ))
    }
}
