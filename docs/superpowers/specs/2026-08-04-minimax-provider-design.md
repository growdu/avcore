# MiniMax Provider 适配 — 设计 spec

> 给 AVCore v1 加 3 个 MiniMax 专有 API provider（avatar / voice / video），让用户用 minimaxi.com token 一站跑通 5 维度。
> 配套 spec：`docs/minimax-api.md`（实测的 endpoint 规格）。

---

## 1. 目标

- **输入**：用户已购 MiniMax 套餐（含 multimodal）；env `ANTHROPIC_AUTH_TOKEN=sk-cp-...`
- **输出**：`avc render run --avatar-provider minimax --voice-provider minimax --video-provider minimax` 走 MiniMax 端到端出片
- **不做**：voice clone（占位）、embedding（沿 OpenAI）、music（不在 5 维度）、job_worker（独立）

---

## 2. 配置 schema（完全独立段）

```toml
[provider.avatar_minimax.yu]
api_key = "sk-cp-..."
model = "image-01"                    # image-01 / image-01-live
base_url = "https://api.minimaxi.com"  # 默认

[provider.voice_minimax.yu]
api_key = "sk-cp-..."
model = "speech-01-turbo"
base_url = "https://api.minimaxi.com"

[provider.video_minimax.yu]
api_key = "sk-cp-..."
model = "video-01"                    # video-01 / T2V-01 / I2V-01 (需 first_frame_image)
base_url = "https://api.minimaxi.com"
```

**工厂路由规则**：provider 名字以 `_minimax` 结尾 → 走 `MiniMaxCompat*Provider` 实现；其他 → 走现有 `OpenAiCompat*Provider`。

**avc.toml 段名硬约束**：
- `[provider.avatar.<name>]` → OpenAI 兼容 avatar
- `[provider.avatar_minimax.<name>]` → MiniMax avatar
- 段名后缀固定 `_minimax`；用户写啥前缀都行，工厂看 `_minimax` 后缀

**5 维度对应的段名**：
- `provider.avatar_minimax.<n>` → MiniMaxAvatarProvider
- `provider.voice_minimax.<n>` → MiniMaxVoiceProvider
- `provider.video_minimax.<n>` → MiniMaxVideoProvider
- `provider.llm.*` / `provider.embed.*` → 已能用现有 OpenAI 兼容（minimax 已有 minimax 段）

---

## 3. 文件组织

| 文件 | 状态 | 改动 |
|---|---|---|
| `src/provider/minimax.rs` | **新建** | 3 个 provider 实现 + 公共 helper（错误翻译、HEX→bytes）~600 行 |
| `src/provider/mod.rs` | 改 | 加 `pub mod minimax;`；avatar / voice / video factory 加 `_minimax` 路由 ~30 行 |
| `tests/integration.rs` | 改 | 工厂路由测试 + 1 个真实 API 集成测试 ~150 行 |
| `docs/cli.md` | 改 | 增 `[provider.<dim>_minimax.<n>]` 段说明 ~30 行 |
| `CHANGELOG.md` | 改 | `[Unreleased]` 增 MiniMax 适配条 |

**不动**：`src/provider/real.rs`、`src/provider/probe.rs`、现有 OpenAI providers、`src/svc/*`（pipeline / render / finetune 不变）。

---

## 4. 公共 helper（`src/provider/minimax.rs`）

```rust
/// 公共：拼 Authorization header
fn auth_header(key: &str) -> reqwest::header::HeaderMap { ... }

/// 公共：把 MiniMax 错误码翻译成 AvcError
/// - HTTP 401 → AvcError::TokenAuth(msg)
/// - HTTP 429 → AvcError::RateLimited(msg)
/// - 业务码 2013 → AvcError::Arg(msg)
/// - 其他 → AvcError::ProviderUpstream(msg)
async fn handle_response<T: DeserializeOwned>(resp: reqwest::Response) -> AvcResult<T> { ... }

/// 公共：MiniMax 音频字段是 HEX 编码（不是 base64）
fn decode_hex_audio(hex: &str) -> AvcResult<Vec<u8>> { ... }

/// 公共：等 video 任务完成（poll status 直到 Success/Fail/timeout）
async fn wait_video_done(
    client: &reqwest::Client,
    base_url: &str,
    auth: &HeaderMap,
    task_id: &str,
    poll_interval: Duration,    // 默认 5s
    timeout: Duration,          // 默认 5min
) -> AvcResult<String>          // 返 file_id
{ ... }
```

---

## 5. 三 provider 实现

### 5.1 `MiniMaxCompatAvatarProvider::create`

```rust
impl AvatarProvider for MiniMaxCompatAvatarProvider {
    fn name(&self) -> &str { &self.name }
    async fn create(&self, spec: &AvatarSpec) -> AvcResult<Avatar> {
        // 1. POST /v1/image_generation
        //    body: {model, prompt, n=1, aspect_ratio, response_format="url", prompt_enhancer=true}
        // 2. handle_response -> data.image_urls[0]
        // 3. client.get(&url).send().await?.bytes().await?  // 下载
        // 4. Avatar { primary: bytes.to_vec(), mime: "image/jpeg" }
        // 注：现 MiniMax 不支持 base64 response，只 url
    }
}
```

**端点**：`POST {base_url}/v1/image_generation`（spec §3）

**实测模型**：`image-01`（推荐）、`image-01-live`
**实测 aspect_ratio**：`1:1` / `16:9` / `9:16` / `4:3` / `3:4` / `3:2` / `2:3` / `21:9` / `9:21`
**默认 aspect**：`1:1`（spec 没给，固定）

### 5.2 `MiniMaxCompatVoiceProvider::synth`

```rust
impl VoiceProvider for MiniMaxCompatVoiceProvider {
    fn name(&self) -> &str { &self.name }
    async fn synth(&self, voice: &Voice, text: &str) -> AvcResult<Audio> {
        // 1. POST /v1/t2a_v2
        //    body: {model, text, voice_setting: {voice_id: "male-qn-qingse"}, audio_setting: {format: "mp3"}}
        // 2. handle_response -> data.audio (HEX 字符串!)
        // 3. decode_hex_audio(&hex) -> MP3 bytes
        // 4. Audio { wav_b64: base64::encode(&mp3_bytes), mime: "audio/mpeg" }
        // 注：v1 不实现 clone()，返回 placeholder RIFF (与 OpenAI provider 现状对齐)
    }
    async fn clone(&self, _base: &Voice, _samples: &[Sample], _cfg: &TrainCfg) -> AvcResult<Voice> {
        // placeholder, returns RIFF...CLONE_PLACEHOLDER + fake id + zero vector
    }
}
```

**端点**：`POST {base_url}/v1/t2a_v2`（spec §4）

**实测模型**：`speech-01-turbo`（推荐）、`speech-01`
**实测 voice_id**：`male-qn-qingse`（其它用户需自测；API 没 list endpoint）
**关键陷阱**：audio 字段是 **HEX 编码**（spec §4.4），不是 base64
**默认 voice_id**：`male-qn-qingse`（写死；用户暂不能选声线）

### 5.3 `MiniMaxCompatVideoProvider`（最复杂）

```rust
impl VideoProvider for MiniMaxCompatVideoProvider {
    fn name(&self) -> &str { &self.name }
    async fn submit(&self, prompt: &str, _avatar: &[u8], _voice: &[u8]) -> AvcResult<String> {
        // 1. POST /v1/video_generation
        //    body: {model, prompt}
        // 2. handle_response -> task_id
    }
    async fn status(&self, task_id: &str) -> AvcResult<VideoStatus> {
        // 1. GET {base_url}/v1/query/video_generation?task_id=...
        // 2. 映射 status: "Processing" -> Pending, "Success" -> Ready, "Fail" -> Failed
    }
    async fn fetch(&self, task_id: &str, out: &Path) -> AvcResult<()> {
        // 1. wait_video_done(...) 拿 file_id
        // 2. GET /v1/files/retrieve?file_id=... -> download_url
        // 3. client.get(&download_url).send().await?.bytes().await?  // ⚠️ 无 auth
        // 4. fs::write(out, &bytes)
    }
}
```

**3 端点**：
- `POST {base_url}/v1/video_generation`（spec §5.1）
- `GET {base_url}/v1/query/video_generation?task_id=...`（spec §5.1 step 2）
- `GET {base_url}/v1/files/retrieve?file_id=...`（spec §5.1 step 3）+ 下载

**实测模型**：`video-01`（T2V）、`I2V-01`（需 first_frame_image，v1 暂不支持）
**实测耗时**：~80-110s/条
**关键陷阱**：video 每日 3 条配额（撞到返 429 译为 `AvcError::RateLimited`）
**默认 poll**：5s 间隔 / 5min timeout（与现有 `CliVideoProvider` 对齐）

### 5.4 工厂路由

修改 `src/provider/mod.rs`：

```rust
// 现有：
pub fn create_avatar(name: &str, cfg: &ProviderCfg) -> AvcResult<Box<dyn AvatarProvider>> { ... }

// 改为：
pub fn create_avatar(name: &str, cfg: &ProviderCfg) -> AvcResult<Box<dyn AvatarProvider>> {
    if name.ends_with("_minimax") {
        return Ok(Box::new(MiniMaxCompatAvatarProvider::new(name, cfg)?));
    }
    // 现有 OpenAI 路径
    Ok(Box::new(OpenAiCompatAvatarProvider::new(name, cfg)?))
}
```

3 个维度（avatar / voice / video）同样加 `_minimax` 后缀路由。

---

## 6. 错误翻译（统一在 `handle_response`）

| HTTP | 业务码 | 翻译 |
|---|---|---|
| 200 | 0 | 成功 |
| 401 | — | `AvcError::TokenAuth(msg)` |
| 429 | — | `AvcError::RateLimited(msg)` |
| 5xx | — | `AvcError::ProviderUpstream(msg)` |
| 200 | 2013 | `AvcError::Arg(msg)`（参数错：模型不存在、字段缺、长度不够等）|
| 200 | 其他 | `AvcError::ProviderUpstream(msg)` |

**实现**：`async fn handle_response<T>(resp: Response) -> AvcResult<T>`，先读 `base_resp.status_code` + `status_msg`，再决定走 `T::deserialize` 还是返 Err。

---

## 7. 测试策略

### 7.1 单元（mock HTTP server）

沿用 `tests/integration.rs` 已有的 `TcpListener + spawn_mock` 模式（与 T6 同一风格）。新增：

| Provider | mock 行为 | 测试数 |
|---|---|---|
| `MiniMaxCompatAvatarProvider` | mock `/v1/image_generation` 返 200 + fake `image_urls`；mock 图片 GET | 2 |
| `MiniMaxCompatVoiceProvider` | mock `/v1/t2a_v2` 返 200 + fake HEX MP3 | 2 |
| `MiniMaxCompatVideoProvider` | mock 3 步：submit 返 task_id → poll 返 Success + file_id → retrieve 返 fake download_url → mock GET 返 fake mp4 bytes | 3 |

### 7.2 集成（真实 API，1 次验证）

跑 3 个真实 MiniMax 调用（avatar 1 张 + voice 1 段 + video 1 段 ≈ 80-110s）：
- 验证 endpoint 真的能用
- 验证 HEX→bytes 解码正确
- 验证异步 video poll 流程

放独立测试函数 `#[ignore]` 标记（默认不跑；`cargo test -- --ignored` 触发）。CI 不跑（节省 token 配额）。

### 7.3 工厂路由

| 测试 | 验证 |
|---|---|
| `factory_routes_minimax_to_new_providers` | provider 名字 `yu_minimax` 走 MiniMax 实现 |
| `factory_routes_openai_to_existing_providers` | provider 名字 `yu` 走现有 OpenAI 实现 |

---

## 8. 实施 checklist

- [ ] 1. `src/provider/minimax.rs` 加 `MiniMaxCompatAvatarProvider` + `MiniMaxCompatVoiceProvider` + `MiniMaxCompatVideoProvider` 3 个 struct + 公共 helper（auth_header / handle_response / decode_hex_audio / wait_video_done）
- [ ] 2. `src/provider/mod.rs` 加 `pub mod minimax;` + 3 个 factory 函数加 `_minimax` 路由分支
- [ ] 3. `tests/integration.rs` 加 7 个 mock 单元测试（avatar 2 / voice 2 / video 3）+ 2 个工厂路由测试
- [ ] 4. `tests/integration.rs` 加 3 个 `#[ignore]` 真实 API 集成测试（avatar 1 / voice 1 / video 1）
- [ ] 5. `docs/cli.md` 加 `[provider.<dim>_minimax.<n>]` 段说明
- [ ] 6. `CHANGELOG.md` `[Unreleased]` 加 MiniMax 适配条
- [ ] 7. `cargo fmt --all -- --check` 干净
- [ ] 8. `cargo clippy --all-targets -- -D warnings` 干净
- [ ] 9. `cargo test --all-targets` 80+ 个测试全过

---

## 9. Commit 策略

3 commit（按 M2/M5/M6 顺序）：

1. `feat(provider): MiniMax avatar + voice providers`（avatar + voice + factory 路由 + 4 单元 + 2 工厂测试）
2. `feat(provider): MiniMax video provider (3 步异步)`（video + 3 单元测试）
3. `test(provider): MiniMax 真实 API 集成测试`（3 个 `#[ignore]` 真实 API 测试 + doc 同步）

每个 commit 后跑 `cargo test --all-targets` 验证。

---

## 10. 用法示例

```bash
# avc.toml
[provider.avatar_minimax.yu]
api_key = "sk-cp-..."
model = "image-01"

[provider.voice_minimax.yu]
api_key = "sk-cp-..."
model = "speech-01-turbo"

[provider.video_minimax.yu]
api_key = "sk-cp-..."
model = "video-01"

[provider.llm.minimax]
api_key = "sk-cp-..."
model = "MiniMax-M3"
base_url = "https://api.minimaxi.com/v1"
```

```bash
# 端到端
avc render run \
  --persona pg_kernel_expert --version 1 \
  --topic "PostgreSQL 逻辑复制：wal sender 与 logical decoding 的内部机制" \
  --llm-provider minimax \
  --avatar-provider yu_minimax \
  --voice-provider yu_minimax \
  --video-provider yu_minimax
```

期望：3 段脚本 → 3 张 PNG（minimax image_generation）→ 3 段 MP3（minimax t2a_v2）→ 3 段 mp4（minimax video_generation，~80s/条）→ 1 个最终 mp4。

---

## 11. 风险与回滚

| 风险 | 缓解 |
|---|---|
| MiniMax 端点变更 | spec §3-5 实测时用 curl 验证 + 单测覆盖基础 case；CI 跑集成测试 `#[ignore]` 默认不触发 |
| 视频每日 3 条配额 | `AvcError::RateLimited` 翻译让用户能感知；错误提示文本里告知「已用 X/3 条」|
| 用户配错 `protocol` 字段（不存在）| 设计上 `_minimax` 后缀是名字一部分，**没有 `protocol` 字段**——避免字段误用 |
| 现有 OpenAI 路由被破坏 | factory 路由**先看后缀**，不是替换；OpenAI 路径走 else 分支，原有测试都过 |

**回滚**：3 commit 一起 `git revert` 即可。`real.rs` 和 `tests/integration.rs` 不动，回滚影响面小。

---

## 12. 不在本设计范围（明确）

| 项 | 原因 | 何时做 |
|---|---|---|
| Voice clone | 需 file_id upload 复杂 schema | v0.4 |
| Embedding | MiniMax 没暴露 | 等 MiniMax API 开放 |
| Music gen | AVCore 5 维度不含 music | 单独扩展 |
| job_worker 自动跑 pipeline | render run 当前只 INSERT | 独立 v0.4 task |
| `protocol` 字段选项 | 选完全独立段命名（清晰）| — |
| Voice ID 列表 | MiniMax API 无 list endpoint | 用户自测 |
| I2V（图生视频）| 需 first_frame_image 字段 | v0.4 |
| 视频模型 param 扩展（duration / fps）| MiniMax 不暴露 | v0.4 |

---

## 13. 引用

- 实测 spec：`docs/minimax-api.md`
- 通用 5 维度协议：`docs/providers.md`
- 国产厂商选型：`docs/providers-cn.md`
- agent plan 配置 + 实测：`docs/agent-plans.md`
- 项目 plan：`docs/superpowers/plans/2026-08-03-provider-daemon.md`
- 现有 OpenAI 兼容 provider：`src/provider/real.rs`（参考实现风格）
- 现有 VideoProvider trait：`src/provider/real.rs::CliVideoProvider`
