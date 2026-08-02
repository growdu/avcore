//! Mock Provider：返回占位 BLOB，不发任何网络请求
//!
//! Phase 0 / 测试用。生产路径全部走真实 Provider 实现。

use async_trait::async_trait;
use base64::Engine;

use super::*;

pub struct MockAvatarProvider {
    pub name: String,
}

#[async_trait]
impl AvatarProvider for MockAvatarProvider {
    fn name(&self) -> &str {
        &self.name
    }
    async fn create(&self, _spec: &AvatarSpec) -> AvcResult<Avatar> {
        Ok(Avatar {
            provider: self.name.clone(),
            provider_version: "mock-0".into(),
            model_id: Some(format!("mock_avatar_{}", crate::svc::now_ts())),
            primary_png_b64: base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\n"), // 占位 PNG 头
            views_zip_b64: None,
            face_id: Some(format!("face_{}", crate::svc::now_ts())),
        })
    }
    async fn finetune(
        &self,
        base: &Avatar,
        _ref_images: &[String],
        _cfg: &FinetuneConfig,
    ) -> AvcResult<Avatar> {
        Ok(Avatar {
            provider: self.name.clone(),
            provider_version: "mock-finetuned".into(),
            model_id: Some(format!("mock_avatar_ft_{}", crate::svc::now_ts())),
            primary_png_b64: base.primary_png_b64.clone(),
            views_zip_b64: None,
            face_id: base.face_id.clone(),
        })
    }
}

pub struct MockVoiceProvider {
    pub name: String,
}

#[async_trait]
impl VoiceProvider for MockVoiceProvider {
    fn name(&self) -> &str {
        &self.name
    }
    async fn clone(&self, _ref_audio_paths: &[String]) -> AvcResult<Voice> {
        Ok(Voice {
            provider: self.name.clone(),
            provider_version: "mock-0".into(),
            voice_id_remote: Some(format!("mock_voice_{}", crate::svc::now_ts())),
            sample_wav_b64: base64::engine::general_purpose::STANDARD.encode(b"RIFF"), // WAV 头
            transcript: Some("".into()),
            embed_b64: Some(base64::engine::general_purpose::STANDARD.encode(vec![0u8; 8])),
            embed_dim: Some(2),
        })
    }
    async fn synth(&self, _voice: &Voice, text: &str) -> AvcResult<Audio> {
        Ok(Audio {
            wav_b64: base64::engine::general_purpose::STANDARD
                .encode(format!("MOCK_TTS:{}", text).as_bytes()),
            mime: "audio/wav".into(),
        })
    }
    async fn finetune(
        &self,
        base: &Voice,
        _ref: &[String],
        _cfg: &FinetuneConfig,
    ) -> AvcResult<Voice> {
        Ok(Voice {
            provider_version: "mock-finetuned".into(),
            ..base.clone()
        })
    }
}

pub struct MockLlmProvider {
    pub name: String,
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    fn name(&self) -> &str {
        &self.name
    }
    async fn chat(&self, msgs: &[ChatMessage]) -> AvcResult<String> {
        // 极简 echo：把最后一条 user message 回显
        let last = msgs
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.clone())
            .unwrap_or_default();
        Ok(format!("[mock echo] {}", last))
    }
}

pub struct MockVideoProvider {
    pub name: String,
}

#[async_trait]
impl VideoProvider for MockVideoProvider {
    fn name(&self) -> &str {
        &self.name
    }
    async fn render(&self, _v: &Voice, _a: &Avatar, scenes: &[ScriptSegment]) -> AvcResult<Clip> {
        let total_ms: i64 = scenes.iter().map(|s| s.duration_ms).sum();
        Ok(Clip {
            mp4_b64: base64::engine::general_purpose::STANDARD.encode(b"\x00\x00\x00\x18ftypmock"), // mp4 magic
            mime: "video/mp4".into(),
            duration_ms: total_ms,
        })
    }
}

pub struct MockEmbedProvider {
    pub name: String,
}

#[async_trait]
impl EmbedProvider for MockEmbedProvider {
    fn name(&self) -> &str {
        &self.name
    }
    async fn embed(&self, texts: &[&str]) -> AvcResult<Vec<Vec<f32>>> {
        // 极简 hash → 4 维向量
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; 4];
                for (i, b) in t.bytes().enumerate() {
                    v[i % 4] += b as f32 / 255.0;
                }
                v
            })
            .collect())
    }
}
