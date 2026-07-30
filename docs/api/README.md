# Provider / API 参考

> AVCore **不暴露 HTTP / gRPC 服务**——它是 CLI + Rust crate。  
> 这一页给出：(a) 各 Provider 的配置字段、(b) Rust crate 集成方式、(c) 程序化调用示例。

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
    name: "Lily".into(),
    archetype: Some("mentor".into()),
    avatar: AvatarSpec {
        description: "30 岁东亚女性，温和笑容".into(),
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
        tone: "温和".into(),
        catchphrases: vec!["来，我们一步步看".into()],
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
    topic: "牛顿第一定律".into(),
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

### 3.1 `sdxl_ip_adapter`（规划）

```json
{
  "name": "sdxl_ip_adapter",
  "kind": "avatar",
  "version": "v1",
  "config_schema": {
    "base_url":      { "type": "string", "required": true },
    "max_refs":      { "type": "int",    "default": 6 },
    "default_size":  { "type": "string", "default": "1024x1024" },
    "steps":         { "type": "int",    "default": 30 },
    "guidance":      { "type": "float",  "default": 7.5 }
  }
}
```

### 3.2 `kling_avatar`

```json
{
  "name": "kling_avatar",
  "kind": "avatar",
  "version": "v1.2",
  "config_schema": {
    "api_key":    { "type": "string", "required": true, "secret": true },
    "endpoint":   { "type": "string", "default": "https://api.kling.ai" },
    "max_refs":   { "type": "int",    "default": 6 },
    "max_lora_mb":{ "type": "int",    "default": 250 }
  }
}
```

### 3.3 `heygen_avatar`、`flux_lora` 类似。

---

## 4. 声音 Provider

### 4.1 `cosyvoice`（自托管友好）

```json
{
  "name": "cosyvoice",
  "kind": "voice",
  "config_schema": {
    "api_url":  { "type": "string", "required": true },
    "language": { "type": "string", "default": "zh" },
    "min_sample_seconds": { "type": "int", "default": 30 }
  }
}
```

### 4.2 `gpt_sovits`、`volc_tts`、`azure_tts` 类似。

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

`avc persona evolve lily --scope persona` 时会调 `llm.sft`（与 `llm.chat` 区分）：
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
  "config_schema": {
    "api_key":     { "type": "string", "required": true, "secret": true },
    "endpoint":    { "type": "string" },
    "max_seconds": { "type": "int", "default": 10 },
    "mode":        { "type": "enum", "options": ["std", "pro"], "default": "std" }
  }
}
```

### 6.2 `cogvideox`、`animatediff`、`hunyuan_video` 类似。

---

## 7. 知识 Provider

### 7.1 `embed_openai`

```json
{
  "name": "embed_openai",
  "kind": "embed",
  "config_schema": {
    "api_key":  { "type": "string", "required": true, "secret": true },
    "model":    { "type": "string", "default": "text-embedding-3-large" },
    "dim":      { "type": "int",    "default": 3072 }
  }
}
```

### 7.2 `embed_bge`、`reranker_bge` 类似。

---

## 8. 存储 Provider（可选 / Phase 2）

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

Phase 0 / 1 默认本地文件系统。

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
  target: lily
  hint: did you mean "Lily" (capital L)?
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
