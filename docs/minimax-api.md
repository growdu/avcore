# MiniMax API 适配 spec（实测版）

> AVCore 接入 MiniMax 专有多模态 API 的实测技术规格。
> 2026-08-04 实测（base url `https://api.minimaxi.com`，auth 走 `ANTHROPIC_AUTH_TOKEN=sk-cp-...`）。
> 配套实施：`MiniMaxCompatAvatar/Voice/VideoProvider` 写到 `src/provider/real.rs`。

---

## 0. AVCore 现状 vs MiniMax

| 维度 | AVCore 现状（v1.0） | MiniMax 端点 |
|---|---|---|
| `llm` | OpenAI 兼容 `/v1/chat/completions` | ✅ **同协议**，已用 |
| `embed` | OpenAI 兼容 `/v1/embeddings` | ❌ 没暴露（OpenAI 端点 404）|
| `avatar` | OpenAI 兼容 `/v1/images/generations` | ❌ **需新写** `MiniMaxCompatAvatarProvider`（调 `/v1/image_generation`）|
| `voice` | OpenAI 兼容 `/v1/audio/speech` | ❌ **需新写** `MiniMaxCompatVoiceProvider`（调 `/v1/t2a_v2`）|
| `video` | vendor CLI 三段式 | ❌ **需新写** `MiniMaxCompatVideoProvider`（调 `/v1/video_generation` + `/v1/query/video_generation` + `/v1/files/retrieve`）|

**总改动量**：~600 行 Rust：
- `MiniMaxCompatAvatarProvider` (~150 行)
- `MiniMaxCompatVoiceProvider` (~150 行)
- `MiniMaxCompatVideoProvider` (~300 行，因为 video 异步 + file retrieve + 状态机)
- 改 `ProviderCfg` 加 `protocol = "minimax" | "openai"` 字段，或加 `MiniMaxCompatProvider` 单独 Provider 名字
- 单测 + 集成测试

---

## 1. 公共：base URL 与鉴权

| 项 | 值 |
|---|---|
| Base URL | `https://api.minimaxi.com` |
| Auth | `Authorization: Bearer <ANTHROPIC_AUTH_TOKEN>` (key 格式 `sk-cp-...`) |
| Content-Type | `application/json` (binary upload 用 `multipart/form-data`) |

> 注意：key 在火山方舟/OpenAI 是 `sk-...`，MiniMax 是 `sk-cp-...`（带 `cp` 中缀）。AVCore 现 `Config::load` 解析 `api_key` 字段时不区分，可直接复用。

---

## 2. LLM（已经能用，不需要新写）

实测可用（2026-08-04）：
```
POST /v1/chat/completions
Authorization: Bearer $ANTHROPIC_AUTH_TOKEN
Content-Type: application/json
{
  "model": "MiniMax-M3",          // 或 MiniMax-M2.7 / M2.5 / M2.1 + -highspeed
  "messages": [{"role": "user", "content": "..."}],
  "max_tokens": 200,
  "temperature": 0
}
```

返回 OpenAI 格式 chat completion。AVCore `OpenAiCompatLlmProvider` 已能直接用。

---

## 3. Image Generation（待写 avatar adapter）

### 3.1 实测 endpoint

```
POST https://api.minimaxi.com/v1/image_generation
Authorization: Bearer $ANTHROPIC_AUTH_TOKEN
Content-Type: application/json
{
  "model": "image-01",                       // 必须小写，不要加前缀
  "prompt": "a red apple",
  "n": 1,                                    // 1-4
  "aspect_ratio": "1:1",                     // 1:1 / 16:9 / 9:16 / 4:3 / 3:4 / 3:2 / 2:3 / 21:9 / 9:21
  "response_format": "url",                  // 目前只支持 "url"（v1.0 实测）
  "seed": 42,                                 // 可选；同 seed 出同图
  "prompt_enhancer": true                    // 可选；让模型润色 prompt
}
```

成功响应（HTTP 200）：
```json
{
  "id": "06c0e83e28937276b8db9d88fe0b7a4a",
  "data": {
    "image_urls": [
      "https://hailuo-image-algeng-data.oss-cn-wulanchabu.aliyuncs.com/image_inference_output%2Ftalkie%2Fprod%2Fimg%2F2026-08-04%2F...aigc.jpeg?...&Expires=...&Signature=..."
    ]
  },
  "metadata": {
    "failed_count": "0",
    "success_count": "1"
  },
  "base_resp": {
    "status_code": 0,
    "status_msg": "success"
  }
}
```

### 3.2 失败响应

```json
{
  "data": null,
  "base_resp": {
    "status_code": 2013,           // ≠ 0 = 错误
    "status_msg": "invalid params, unsupported model: image-02"
  }
}
```

- HTTP 状态码固定 200；业务状态看 `base_resp.status_code`
- 错误码 `2013` 是参数错；具体看 `status_msg`

### 3.3 AVCore `MiniMaxCompatAvatarProvider::create` 实现要点

```rust
async fn create(&self, spec: &AvatarSpec) -> Result<Avatar> {
    // 1. POST /v1/image_generation with spec.prompt + n=1 + aspect_ratio (derive from spec)
    let body = json!({
        "model": "image-01",
        "prompt": spec.prompt,
        "n": 1,
        "aspect_ratio": "1:1",  // or from spec
    });
    let resp: MiniMaxImageResponse = self.client.post("/v1/image_generation").json(&body).send().await?.json().await?;
    if resp.base_resp.status_code != 0 {
        return Err(AvcError::Upstream(resp.base_resp.status_msg));
    }
    let url = resp.data.image_urls.into_iter().next()
        .ok_or(AvcError::Upstream("no image url"))?;
    
    // 2. 下载图片到 BLOB
    let bytes = self.client.get(&url).send().await?.bytes().await?;
    Ok(Avatar { primary: bytes.to_vec(), mime: "image/jpeg".into(), ... })
}
```

---

## 4. TTS（待写 voice adapter）

### 4.1 实测 endpoint

```
POST https://api.minimaxi.com/v1/t2a_v2
Authorization: Bearer $ANTHROPIC_AUTH_TOKEN
Content-Type: application/json
{
  "model": "speech-01-turbo",              // 或 speech-01
  "text": "你好世界",
  "voice_setting": {
    "voice_id": "male-qn-qingse"         // 男声青涩；其它见 minimax 控制台
  },
  "audio_setting": {
    "sample_rate": 24000,                // 可选
    "bitrate": 128000,                    // 可选
    "format": "mp3"                       // 可选；默认 mp3
  },
  "stream": false                          // false = 一次返；true = 流式
}
```

成功响应（HTTP 200）：
```json
{
  "data": {
    "audio": "4944330400000000..."   // ⚠️ **HEX 编码**的 MP3 字节（不是 base64）
  },
  "base_resp": {
    "status_code": 0,
    "status_msg": "success"
  }
}
```

**重要**：audio 字段是 **HEX 字符串**（不是 base64）。bytes.fromhex 后才是真实 MP3 字节。

### 4.2 错误响应

```json
{
  "base_resp": {
    "status_code": 2013,
    "status_msg": "invalid params, method t2a-v2 not have model: speech-01"
  }
}
```

### 4.3 音色列表（`voice_id`）

实测有效：`male-qn-qingse`（男声青涩）。其它需要从控制台/文档查（API 没暴露 `list voices` endpoint 在 v1.0）。

### 4.4 AVCore `MiniMaxCompatVoiceProvider::synth` 实现要点

```rust
async fn synth(&self, voice: &Voice, text: &str) -> Result<Audio> {
    let body = json!({
        "model": "speech-01-turbo",
        "text": text,
        "voice_setting": { "voice_id": "male-qn-qingse" },
        "audio_setting": { "format": "mp3" }
    });
    let resp: MiniMaxTtsResponse = self.client.post("/v1/t2a_v2").json(&body).send().await?.json().await?;
    if resp.base_resp.status_code != 0 {
        return Err(AvcError::Upstream(resp.base_resp.status_msg));
    }
    let hex_audio = resp.data.audio;
    let bytes = hex::decode(&hex_audio).map_err(|e| AvcError::Internal(format!("hex decode: {}", e)))?;
    // bytes 是 MP3 二进制
    Ok(Audio { wav_b64: base64::encode(&bytes), mime: "audio/mpeg".into(), ... })
}
```

⚠️ `MiniMaxCompatVoiceProvider::clone` 不实现（MiniMax 需 `file_id`/`audio_url` 走的 `/v1/voice_clone`，schema 复杂；v1.0 留 placeholder 同 v1 现状）。

---

## 5. Video Generation（待写 video adapter，复杂）

### 5.1 三步式：submit → poll → retrieve

#### Step 1: Submit

```
POST https://api.minimaxi.com/v1/video_generation
Authorization: Bearer $ANTHROPIC_AUTH_TOKEN
Content-Type: application/json
{
  "model": "video-01",                // 或 T2V-01 / T2V-01-Director / I2V-01 (image-to-video，需 first_frame_image)
  "prompt": "a cat walking"
}
```

成功响应（HTTP 200）：
```json
{
  "task_id": "427144058097923",
  "base_resp": {
    "status_code": 0,
    "status_msg": "success"
  }
}
```

`task_id` 用于 polling。

#### Step 2: Poll

```
GET https://api.minimaxi.com/v1/query/video_generation?task_id=427144058097923
Authorization: Bearer $ANTHROPIC_AUTH_TOKEN
```

响应（处理中）：
```json
{
  "task_id": "427144058097923",
  "status": "Processing",      // Processing | Success | Fail
  "file_id": "",                 // 完成后才有
  "video_width": 0,
  "video_height": 0,
  "base_resp": { "status_code": 0, "status_msg": "success" }
}
```

完成：
```json
{
  "task_id": "427144058097923",
  "status": "Success",
  "file_id": "427071051673913",
  "video_width": 1280,
  "video_height": 720,
  "base_resp": { "status_code": 0, "status_msg": "success" }
}
```

实测：单条 video 约 80-110 秒完成。

#### Step 3: Get Download URL

```
GET https://api.minimaxi.com/v1/files/retrieve?file_id=427071051673913
Authorization: Bearer $ANTHROPIC_AUTH_TOKEN
```

响应：
```json
{
  "file": {
    "file_id": 427071051673913,
    "bytes": 0,
    "created_at": 1785835820,
    "filename": "output_aigc.mp4",
    "purpose": "video_generation",
    "download_url": "https://public-cdn-video-data-algeng.oss-cn-wulanchabu.aliyuncs.com/...?Expires=...&Signature=..."
  },
  "base_resp": { "status_code": 0, "status_msg": "success" }
}
```

⚠️ `download_url` 有过期时间（Expires 参数，~1h）；**立刻下载**或**用 refresh token 重新 get**。

#### Step 4: 下载 mp4

```
GET <download_url>     # 无 auth（公开 URL）
```

返回 mp4 binary。

### 5.2 Video 模型变体

| 模型 | 用途 | 备注 |
|---|---|---|
| `video-01` | 通用文生视频 | 实测 ✅ |
| `T2V-01` | 文本生视频 | ✅ |
| `T2V-01-Director` | 导演模式文生视频 | ✅ |
| `I2V-01` | 图生视频 | 需 `first_frame_image` 字段 |
| `I2V-01-Director` | 图生视频导演模式 | 需 `first_frame_image` |
| `I2V-01-live` | 图生视频 live | 需 `first_frame_image` |

### 5.3 AVCore `MiniMaxCompatVideoProvider` 实现要点

AVCore `VideoProvider` trait 需要 `submit` / `status` / `fetch` 三段式（见 `src/provider/real.rs::CliVideoProvider`）。MiniMax 是**远程 HTTP**（不是 vendor CLI），所以 trait 需要新加一个 `HttpVideoProvider` 抽象，或者 `MiniMaxCompatVideoProvider` 直接 `impl VideoProvider` 用自己的状态机。

```rust
pub struct MiniMaxCompatVideoProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    poll_interval: Duration,  // default 5s
    timeout: Duration,         // default 5min
}

impl VideoProvider for MiniMaxCompatVideoProvider {
    async fn submit(&self, prompt: &str, _avatar: &[u8], _voice: &[u8]) -> Result<String> {
        // POST /v1/video_generation → returns task_id
    }
    async fn status(&self, task_id: &str) -> Result<VideoStatus> {
        // GET /v1/query/video_generation?task_id=...
    }
    async fn fetch(&self, task_id: &str, out: &Path) -> Result<()> {
        // poll status until Success → get file_id → GET /v1/files/retrieve?file_id=... → download mp4 → write to out
    }
}
```

> 难点：trait 现在只有 `submit` / `status` / `fetch` 3 段，没"轮询直到完成"的内置逻辑。`fetch` 内部需要循环 status（5s 一次，5min timeout）然后调 retrieve。可行但要小心：5min 内视频任务失败的话 fetch 也返回 Err。

---

## 6. Embedding（MiniMax 没暴露）

实测 `/v1/embeddings` 返回 404。MiniMax 套餐里没独立的 embedding API。

**绕路**：
- v1 沿用 OpenAI 的 `text-embedding-3-small`（用户需另备 OpenAI key）
- 或 finetune drift eval 用文本对文本（已有 fallback）

---

## 7. Voice Clone（schema 复杂，v1 留 placeholder）

`POST /v1/voice_clone` 至少需要：
- `voice_id`（用户自定义 ID）
- `file_id` 或 `audio_url`（先用 `/v1/files/upload` 上传 mp3，得到 file_id；或传外网 URL）

实测错误：
```
file_id or audio_url is required
voice_id length  // 限制具体长度没测
```

**v1.0 实现建议**：
- 写 `MiniMaxCompatVoiceProvider::clone` 时先调 `/v1/files/upload` 上传本地 mp3 → 拿到 file_id → 再调 `/v1/voice_clone`
- 不实现的话保持 `clone` 返回 placeholder（v1 现状），doc 标注"v1 不支持 MiniMax voice clone"

---

## 8. Music Generation（不实现，AVCore 没这个维度）

AVCore 5 维度是 llm/embed/avatar/voice/video。music 不是维度，跳过。

如果用户要 music：自建 vendor CLI 包 MiniMax `/v1/music_generation`。

---

## 9. 总实施清单（按优先级）

| 顺序 | 模块 | 改动量 | 依赖 |
|---|---|---|---|
| 1 | `MiniMaxCompatVideoProvider` | ~300 行 | 复杂（3 步 + 轮询） |
| 2 | `MiniMaxCompatAvatarProvider` | ~150 行 | 简单（1 POST + 1 GET 下图片） |
| 3 | `MiniMaxCompatVoiceProvider::synth` | ~150 行 | 简单（1 POST，注意 hex→bytes） |
| 4 | 改 `ProviderCfg` 加 `protocol = "minimax" | "openai"` 字段 | ~30 行 | — |
| 5 | 在 `OpenAiCompat*Provider` factory 里加路由 | ~50 行 | — |
| 6 | 加 `shell` 的 `nl_model` 路由（如果 `provider.llm.minimax.protocol = "minimax"` 则用 MiniMax 专有 chat）| — | 已经有 |

**总代码改动**：~600 行 Rust + 5-8 个新单测 + 1-2 个集成测试。

---

## 10. avc.toml 适配 MiniMax 的预期配置

```toml
# ── LLM（MiniMax 兼容 chat，已能用）──
[provider.llm.minimax]
api_key = "sk-cp-..."
model = "MiniMax-M3"
base_url = "https://api.minimaxi.com/v1"

# ── Avatar（待加 MiniMaxCompatAvatarProvider）──
[provider.avatar.minimax]
api_key = "sk-cp-..."        # 跟 LLM 共用
model = "image-01"
protocol = "minimax"         # 触发 MiniMaxCompatAvatarProvider

# ── Voice（待加 MiniMaxCompatVoiceProvider）──
[provider.voice.minimax]
api_key = "sk-cp-..."
model = "speech-01-turbo"
protocol = "minimax"

# ── Video（待加 MiniMaxCompatVideoProvider）──
[provider.video.minimax]
api_key = "sk-cp-..."
model = "video-01"
protocol = "minimax"
```

`provider_test` 和 daemon 探活应能走通：
- `avc provider test avatar.minimax` → POST /v1/image_generation "a red apple"，1x1 aspect，验证下载
- `avc provider test voice.minimax` → POST /v1/t2a_v2 "hi"，验证 hex decode 出有效 MP3
- `avc provider test video.minimax` → POST /v1/video_generation "a cat walking"，验证返回 task_id（不实际等完成）

---

## 11. 端到端 render 计划（待加代码后）

有了 `MiniMaxCompat*Provider`：

```bash
avc render run --persona pg_kernel_expert --version 1 \
  --topic "PostgreSQL 逻辑复制：wal sender 与 logical decoding 的内部机制" \
  --llm-provider minimax \
  --voice-provider minimax \
  --avatar-provider minimax \
  --video-provider minimax
```

期望（要 pipeline worker 实际跑——这是 v1 另一未实现的限制）：
1. `script_gen` 节点 → LLM（minimax）→ 3 段脚本
2. `tts` 节点 → 3 段 MP3（minimax t2a_v2）
3. `img_gen` 节点 → 3 张 PNG（minimax image_generation）
4. `i2v` 节点 → 3 段 mp4（minimax video_generation，~80s）
5. `compose` 节点 → 1 个最终 mp4

总耗时约 5-10 分钟（video 异步最久）。

---

## 12. 相关文档

- `docs/providers.md` — 5 维度协议通用
- `docs/providers-cn.md` — 国产厂商选型
- `docs/agent-plans.md` — 火山方舟 + MiniMax 配置指南（已实测过 LLM）
- `docs/storage.md` — schema
- `docs/cli.md` — CLI 参考

## 13. 测试时复现的 cURL

```bash
source <(env | grep "^ANTHROPIC_" | sed 's/^/export /')

# 1. List models
curl -fsS https://api.minimaxi.com/v1/models -H "Authorization: Bearer $ANTHROPIC_AUTH_TOKEN"

# 2. Image
curl -fsS -X POST https://api.minimaxi.com/v1/image_generation \
  -H "Authorization: Bearer $ANTHROPIC_AUTH_TOKEN" -H "Content-Type: application/json" \
  -d '{"model":"image-01","prompt":"a red apple","n":1,"aspect_ratio":"1:1","response_format":"url"}'

# 3. TTS
curl -fsS -X POST https://api.minimaxi.com/v1/t2a_v2 \
  -H "Authorization: Bearer $ANTHROPIC_AUTH_TOKEN" -H "Content-Type: application/json" \
  -d '{"model":"speech-01-turbo","text":"hi","voice_setting":{"voice_id":"male-qn-qingse"},"audio_setting":{"format":"mp3"}}'

# 4. Video submit
curl -fsS -X POST https://api.minimaxi.com/v1/video_generation \
  -H "Authorization: Bearer $ANTHROPIC_AUTH_TOKEN" -H "Content-Type: application/json" \
  -d '{"model":"video-01","prompt":"a cat walking"}'

# 5. Video poll
curl -fsS "https://api.minimaxi.com/v1/query/video_generation?task_id=<TASK_ID>" \
  -H "Authorization: Bearer $ANTHROPIC_AUTH_TOKEN"

# 6. Get download URL
curl -fsS "https://api.minimaxi.com/v1/files/retrieve?file_id=<FILE_ID>" \
  -H "Authorization: Bearer $ANTHROPIC_AUTH_TOKEN"
```
