# Provider / API 参考

> AVCore **不暴露 HTTP / gRPC 服务**——它是 CLI + Rust crate。  
> 这一页给出：(a) 各 Provider 的配置字段（含 `api_key`）、(b) Rust crate 集成方式、(c) 程序化调用示例。

> **强约束**：每个 Provider 都通过 token 鉴权调用商业 / 开源模型的 HTTP API；本框架**不加载、不推理任何本地模型**。
>
> CLI / REPL 用法见 [`cli.md`](../cli.md)；统一架构见 [`architecture.md`](../architecture.md)。

---

## 1. 内部模块 API（Rust crate）

集成方在自己的 Rust 应用中引用：

```toml
[dependencies]
avc = "0.1"
```

```rust
use avc::{Avc, PersonaId, VersionId};

let avc = Avc::open_default()?;   // 默认 ~/.local/share/avc

// 列出所有 persona
for p in avc.personas().list()? {
    println!("{} (current=v{})", p.name, p.current_version);
}

// 创建 persona
let task = avc.personas().create(CreateSpec {
    name: "Yu".into(),
    archetype: Some("db_kernel_expert".into()),
    avatar: AvatarSpec {
        description: "数据库内核领域讲师".into(),
        style_tags: vec!["写实".into(), "教学".into()],
        ref_images: vec!["./samples/ref_1.png".into()],
        ..Default::default()
    },
    voice: VoiceSpec {
        language: "zh".into(),
        samples: vec![VoiceSample {
            uri: "./samples/voice_1.wav".into(),
            duration_ms: 42000,
            text: "...".into(),
        }],
        ..Default::default()
    },
    persona_descriptor: PersonaDescriptor {
        traits: vec!["耐心".into(), "严谨".into(), "幽默".into()],
        tone: "严谨".into(),
        catchphrases: vec!["我们直接看源码".into()],
        taboos: vec!["绝对化表述".into()],
        formality: 0.6,
        temperature: 0.7,
        ..Default::default()
    },
    ..Default::default()
}).await?;

let persona = task.wait().await?;     // -> PersonaModel

// 持续训练
let train = avc.evolution().evolve(EvovleSpec {
    persona_id: persona.id.clone(),
    base_version: persona.current_version,
    scope: vec![Scope::Voice, Scope::Persona],
    sample_ids: vec![/* ... */],
    consistency_threshold: 0.85,
    fallback_to_base: true,
    ..Default::default()
}).await?;
let result = train.wait().await?;     // -> TrainingOutcome::Published | RolledBack

// 出片
let job = avc.render().video(RenderSpec {
    persona_id: persona.id.clone(),
    version: 2,
    topic: "InnoDB Buffer Pool 替换算法".into(),
    key_points: vec!["定义".into(), "示例".into(), "应用".into()],
    duration: Duration::from_secs(60),
    options: JobRenderOptions::default(),
}).await?;
let artifact = job.wait().await?;
println!("mp4: {}", artifact.video_path.display());
```

---

## 2. Provider trait 扩展

```rust
use avc::provider::{AvatarProvider, ProviderError};

pub struct MyAvatarProvider;

#[async_trait]
impl AvatarProvider for MyAvatarProvider {
    fn name(&self) -> &str { "my_avatar" }

    async fn create(&self, spec: &AvatarSpec) -> Result<Avatar, ProviderError> {
        // 调你的服务
    }

    async fn finetune(
        &self,
        base: &Avatar,
        samples: &[Sample],
        cfg: &TrainCfg,
    ) -> Result<Avatar, ProviderError> { ... }
}

// 注册
avc.providers().avatar().register("my_avatar", Arc::new(MyAvatarProvider));
```

---

## 3. 形象 Provider

> 所有 Provider 必须有 `api_key`（`secret: true`）；调用前 `avc` 会做 preflight 校验，没配就拒绝。

### 3.1 `kling_avatar`

```json
{
  "name": "kling_avatar",
  "kind": "avatar",
  "auth": { "scheme": "bearer", "env": "KLING_API_KEY", "config_key": "api_key" },
  "endpoint": "https://api.kling.ai/v1/avatars",
  "limits": { "max_refs": 6 },
  "default_size": "1024x1024"
}
```

### 3.2 `heygen_avatar`

```json
{
  "name": "heygen_avatar",
  "kind": "avatar",
  "auth": { "scheme": "bearer", "config_key": "api_key" },
  "endpoint": "https://api.heygen.ai/v1/avatars",
  "limits": { "max_refs": 8 }
}
```

### 3.3 `doubao_image`（字节豆包）

```json
{
  "name": "doubao_image",
  "kind": "avatar",
  "auth": { "scheme": "bearer", "env": "ARK_API_KEY", "config_key": "api_key" },
  "endpoint": "https://ark.cn-beijing.volces.com/api/v3/images"
}
```

### 3.4 `seedream`（阿里即梦 / 通义）

```json
{
  "name": "seedream",
  "kind": "avatar",
  "auth": { "scheme": "bearer", "config_key": "api_key" },
  "endpoint": "https://dashscope.aliyuncs.com/api/v1/services/aigc/image-generation"
}
```

### 3.5 `replicate_flux_lora`

```json
{
  "name": "replicate_flux_lora",
  "kind": "avatar",
  "auth": { "scheme": "bearer", "env": "REPLICATE_API_TOKEN", "config_key": "api_key" },
  "endpoint": "https://api.replicate.com/v1/predictions"
}
```

> 本框架**不包含**自托管形态的 avatar Provider——所有都是 token API。

---


## 4. 声音 Provider

### 4.1 `elevenlabs_voice_clone`

```json
{
  "name": "elevenlabs_voice_clone",
  "kind": "voice",
  "auth": { "scheme": "bearer", "config_key": "api_key" },
  "endpoint": "https://api.elevenlabs.io/v1/voices/add",
  "limits": { "min_sample_seconds": 30, "max_sample_seconds": 300 }
}
```

### 4.2 `azure_speech_personal_voice`

```json
{
  "name": "azure_speech_personal_voice",
  "kind": "voice",
  "auth": { "scheme": "bearer", "config_key": "api_key" },
  "endpoint": "https://<region>.api.cognitive.microsoft.com/",
  "options": { "region": { "type": "string", "required": true } }
}
```

### 4.3 `doubao_tts`

```json
{ "name": "doubao_tts", "kind": "voice",
  "auth": { "scheme": "bearer", "env": "ARK_API_KEY", "config_key": "api_key" },
  "endpoint": "https://openspeech.bytedance.com/api/v1/tts" }
```

### 4.4 `openai_tts`（gpt-4o audio 等）

```json
{ "name": "openai_tts", "kind": "voice",
  "auth": { "scheme": "bearer", "config_key": "api_key" },
  "endpoint": "https://api.openai.com/v1/audio/speech" }
```

### 4.5 `volc_tts`、`azure_tts` 等类似结构。

> 不包含自托管声音 Provider（`cosyvoice` / `gpt-sovits` / `f5-tts` 等本地推理方案）。

---


## 5. LLM Provider

### 5.1 `openai_compat`（兼容 OpenAI / 豆包 / DeepSeek / 智谱）

```json
{
  "name": "openai_compat",
  "kind": "llm",
  "config_schema": {
    "base_url":      { "type": "string", "required": true },
    "api_key":       { "type": "string", "required": true, "secret": true },
    "default_model": { "type": "string", "default": "gpt-4o-mini" },
    "timeout_s":     { "type": "int",    "default": 60 }
  }
}
```

`avc persona evolve yu --scope persona` 时会调 `llm.sft`（与 `llm.chat` 区分）：
```json
{
  "name": "openai_compat",
  "kind": "llm_sft",
  "config_schema": {
    "base_url":      { "type": "string", "required": true },
    "api_key":       { "type": "string", "required": true, "secret": true },
    "base_model":    { "type": "string", "required": true },
    "finetune_endpoint": { "type": "string" }
  }
}
```

---

## 6. 视频 Provider（i2v）

### 6.1 `kling`

```json
{
  "name": "kling",
  "kind": "video",
  "auth": { "scheme": "bearer", "env": "KLING_API_KEY", "config_key": "api_key" },
  "endpoint": "https://api.kling.ai/v1/videos",
  "limits": { "max_seconds": 10 },
  "options": { "mode": { "type": "enum", "values": ["std", "pro"], "default": "std" } }
}
```

### 6.2 `doubao_seedance`

```json
{ "name": "doubao_seedance", "kind": "video",
  "auth": { "scheme": "bearer", "env": "ARK_API_KEY", "config_key": "api_key" },
  "endpoint": "https://ark.cn-beijing.volces.com/api/v3/video/generations" }
```

### 6.3 `pika`、`runway`、`replicate_cogvideox` 类似结构。

> 不包含 `cogvideox` / `animatediff` / `hunyuan_video` 的本地推理版本。

---


## 7. 知识 Provider

### 7.1 `openai_embed`

```json
{
  "name": "openai_embed",
  "kind": "embed",
  "auth": { "scheme": "bearer", "config_key": "api_key" },
  "endpoint": "https://api.openai.com/v1/embeddings",
  "options": { "model": { "default": "text-embedding-3-large" }, "dim": { "default": 3072 } }
}
```

### 7.2 `volcengine_embed`、`alibaba_embed`、`cohere_embed`、`cohere_rerank`、`voyage_rerank` 结构类似。

> 不包含 `embed_bge` / `bge-reranker` 的本地推理版本；若必须使用 BGE，需通过 Hugging Face Inference API 等远端端点。

---


## 8. 存储 Provider（默认不开 / Phase 2+）

```json
{
  "name": "s3",
  "kind": "storage",
  "config_schema": {
    "bucket":     { "type": "string", "required": true },
    "region":     { "type": "string" },
    "prefix":     { "type": "string", "default": "avc/" },
    "access_key": { "type": "string", "secret": true },
    "secret_key": { "type": "string", "secret": true }
  }
}
```

**默认推荐：本地文件系统**（参见 [`../storage.md`](../storage.md)）。本节描述的是当本地空间不足 / 团队多机时的可选方案。

---

## 9. 错误约定

```rust
#[derive(thiserror::Error, Debug)]
pub enum ProviderError {
    #[error("rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("upstream error: {code} {message}")]
    Upstream { code: String, message: String },

    #[error("auth failed")]
    Unauthorized,

    #[error("timeout")]
    Timeout,

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
```

CLI 错误格式：

```
error[E0403]: persona_not_found
  target: yu
  hint: did you mean "Yu" (capital L)?
  doc: https://avc.dev/docs/cli/errors#E0403
```

退出码：
| code | 含义 |
|------|------|
| 0 | ok |
| 1 | 通用失败 |
| 2 | 参数错 |
| 3 | 资源不存在 |
| 4 | 状态冲突 |
| 5 | 鉴权失败 |
| 10 | Provider 限速 |
| 11 | Provider 上游错 |
| 12 | Provider 超时 |

---

## 10. 命名与 ID

- persona_model_id: `pm_<ULID>`
- version_id: 整数（在 persona_model_id 内自增）
- sample_id: `smp_<ULID>`
- training_job_id: `tj_<ULID>`
- video_job_id: `job_<ULID>`
- corpus_id: `crp_<ULID>`
- task_id（异步操作短 ID）: `tsk_<ULID>`

ULID 26 字符、字典序即时间序、URL safe。
