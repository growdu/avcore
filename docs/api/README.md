# Provider / API 参考

> AVCore 是 CLI + Rust crate，不暴露 HTTP API。

---

## 1. Rust crate 集成

```toml
[dependencies]
avc = "0.1"
```

```rust
use avc::{Avc, RenderSpec, JobRenderOptions};
use std::time::Duration;

let avc = Avc::open_default()?;   // ~/.local/share/avc/

for p in avc.personas().list()? {
    println!("{} (current=v{})", p.name, p.current_version);
}

let job = avc.render().video(RenderSpec {
    persona_id: "yu".into(),
    version: 2,
    topic: "InnoDB Buffer Pool 替换算法".into(),
    duration: Duration::from_secs(60),
    options: JobRenderOptions::default(),
}).await?;

let result = job.wait().await?;
println!("mp4 BLOB: {} bytes", result.video_bytes.len());
```

---

## 2. Provider trait

```rust
#[async_trait]
pub trait AvatarProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn create(&self, spec: &AvatarSpec) -> Result<Avatar>;
    async fn finetune(
        &self,
        base: &Avatar,
        samples: &[Sample],
        cfg: &TrainCfg,
    ) -> Result<Avatar>;
}

pub trait VoiceProvider {
    async fn clone(&self, samples: &[Audio]) -> Result<Voice>;
    async fn synth(&self, voice: &Voice, text: &str) -> Result<Audio>;
    async fn finetune(
        &self,
        base: &Voice,
        samples: &[Sample],
        cfg: &TrainCfg,
    ) -> Result<Voice>;
}

pub trait LlmProvider   { async fn chat(&self, msgs: &[Msg]) -> Result<Msg>; }
pub trait VideoProvider { async fn render(&self, req: RenderReq) -> Result<Clip>; }
pub trait EmbedProvider { async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>; }
```

每个 Provider = 一份 `provider.json` + 上面的 trait 实现。**全部 token 鉴权调用商业 / 开源 API**。

注册：

```rust
avc.providers().avatar().register("kling_avatar", Arc::new(KlingAvatarProvider::new(config)));
```

---

## 3. `provider.json` 字段

```json
{
  "name": "kling_avatar",
  "kind": "avatar",
  "auth": {
    "scheme": "bearer",
    "env": "KLING_API_KEY",
    "config_key": "api_key"
  },
  "endpoint": "https://api.kling.ai/v1/avatars"
}
```

- `auth.scheme`: bearer / header:X-Custom / query
- `auth.env`: 优先从环境变量取
- `auth.config_key`: 也可从 `avc.toml` 取（用 secret 加密存盘）
- 没 token → `avc` 直接拒绝 + `error[E0501] provider_unauthenticated`

---

## 4. 内置 Provider 列表（**全部 token API**）

| 维度 | Provider | 说明 |
|------|----------|------|
| avatar | `kling_avatar` / `heygen_avatar` / `doubao_image` / `seedream` / `replicate_flux_lora` | 商业 / 开源 via Replicate |
| voice | `elevenlabs` / `azure_speech` / `doubao_tts` / `openai_tts` | |
| llm | `openai_compat` ✅ 真实现已落地（Phase 1.1） | 任意 OpenAI 兼容 `/chat/completions` 端点；通过 `base_url` + `extra_headers` 接 OpenAI / DeepSeek / 智谱 / Anthropic 兼容 proxy / Ollama 等；设 `provider.llm.<name>.api_key`+`model`+`base_url` 后 `avc ask` 直接可用，错误按 401/403→`TokenAuth`、429→`RateLimited`、非 2xx→`ProviderUpstream` 映射到 §5 exit 码 |
| video | `kling` / `doubao_seedance` / `pika` / `runway` / `replicate_cogvideox` | |
| embed | `openai_embed` / `volcengine_embed` / `alibaba_embed` / `cohere_embed` | |

> **本框架不包含任何自托管 Provider**（如 `sdxl_ip_adapter` / `cosyvoice` / `gpt_sovits` / 本地 BGE 等被设计为本地推理的不在内）。

---

## 5. 错误约定

```rust
#[derive(thiserror::Error, Debug)]
pub enum ProviderError {
    #[error("rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("upstream: {code} {message}")]
    Upstream { code: String, message: String },
    #[error("auth failed")]
    Unauthorized,
    #[error("timeout")]
    Timeout,
}
```

CLI 层映射退出码：

| code | 含义 |
|------|------|
| 5 | token 鉴权失败 |
| 6 | token 未配置 |
| 10 | Provider 限速 |
| 11 | Provider 上游错 |
| 12 | Provider 超时 |
