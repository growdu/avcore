# Kling (快手) Video API 集成 Spec for AVCore

> AVCore 接入可灵（klingai.com），替代 MiniMax 的 3 视频/天硬限。
> 调研日期：2026-08-05。Research-only，无代码改动。

---

## 1. 推荐 Endpoint

| Region | Base URL | 备注 |
|--------|----------|------|
| **China（默认）** | `https://api-beijing.klingai.com` | 国内快手，人民币（Alipay/WeChat） |
| **International** | `https://api.klingai.com` | 海外（美元 credit-card 充值）。文档：`https://app.klingai.com/global/dev` |

> **推荐 `api.klingai.com`**（国内用户用国际 endpoint 也行：跨境计费 + 不需要 ICP）。
>
> 与火山方舟 Seedance **不共享协议**——Kling 是快手独立产品。两者都是 AVCore 通过 HTTP 包装的第三方 provider。

---

## 2. 端点

| 阶段 | Method | URL | Body | Response |
|------|--------|-----|------|----------|
| **Auth** | n/a | n/a | n/a | 本地生成 JWT（无远程 login） |
| **Text-to-Video Submit** | `POST` | `/v1/videos/text2video` | `{model_name, prompt, negative_prompt?, duration?, aspect_ratio, mode?, seed?, cfg_scale?, camera_control?, callback_url?}` | `{code:0, data:{task_id, task_status:"submitted"}, message:"success"}` |
| **Image-to-Video Submit** | `POST` | `/v1/videos/image2video` | adds `image` (URL or base64), `image_tail?` (optional end frame) | same envelope |
| **Video Extend Submit** | `POST` | `/v1/videos/video-extend` | `{task_id, prompt?, ...}` | same envelope |
| **Poll task (t2v)** | `GET` | `/v1/videos/text2video/{task_id}` | – | `{code:0, data:{task_id, task_status, task_status_msg, video_urls:[…], created_at, updated_at}}` |
| **Poll task (i2v)** | `GET` | `/v1/videos/image2video/{task_id}` | – | same shape |

**Common envelope**：
```json
{ "code": 0, "message": "success", "data": { ... } }
```

**错误格式**：
```json
{ "code": 1001, "message": "invalid api key", "data": null }
```
`code != 0` ⇒ fail. Common codes：`1001` invalid auth、`1002` insufficient credits、`1101` invalid params、`1300` upstream busy.

**`task_status` values**：`submitted` → `processing` → terminal（`succeed` / `failed`）。On `succeed`，`data.video_urls[]` is array of CDN-signed MP4 links（**TTL ≈ 30 min** — **download immediately**）。

---

## 3. Models + Parameters

| Model | t2v | i2v | v2v | Duration | Aspect | 备注 |
|-------|-----|-----|-----|----------|--------|------|
| `kling-v1`        | ✅ | ✅ | –  | fixed 5s | 16:9 / 9:16 / 1:1 | legacy |
| `kling-v1-5`      | ✅ | ✅ | –  | **5 or 10s** | 16:9 / 9:16 / 1:1 | duration configurable |
| `kling-v1-6`      | ✅ | ✅ | –  | auto (~5–10s) | 16:9 / 9:16 / 1:1 | 1080p option |
| `kling-v2-master` | ✅ | ✅ | –  | 5/10s | 16:9 / 9:16 / 1:1 | high quality |
| `kling-v2-1-master` | ✅ | ✅ | – | 5/10s | 16:9 / 9:16 / 1:1 | |
| `kling-v2-5-turbo` | ✅ | ✅ | – | 5/10s | 16:9 / 9:16 / 1:1 | faster |
| `kling-v2-6` / `kling-v3` / `kling-v3-omni` | ✅ | ✅ | ✅ | 待确认 | 待确认 | latest, 多模态 |
| `kling-video-o1` | ✅ | ✅ | ✅ | 待确认 | 待确认 | multi-modal reference（text/image/video 任一输入） |

**Common params**（across all models）：
- `mode` = `"std"` | `"pro"`（standard vs professional quality）
- `duration` = `"5"` | `"10"` — **only effective on `kling-v1-5`**；others auto-determined
- `aspect_ratio` = `"16:9"` | `"9:16"` | `"1:1"`
- `cfg_scale`（float, kling-v1-5: 0–1, default 0.5）
- `negative_prompt`（string）
- `seed`（int, optional, deterministic）
- `camera_control`（object: type/horizontal/vertical/zoom/pro/pan/tilt/roll）
- `callback_url`（HTTPS webhook for completion push — avoids polling）

**For i2v**：extra `image`（string URL or `data:image/...;base64,...`）and optional `image_tail` for end-frame。

**`prompt` limit**：≤ 2500 chars。

---

## 4. 价格 + 配额

> Kling charges in **"灵感值" (credits)**. 1 credit ≈ 0.01 USD on global; CNY on domestic。

**Per-video cost (third-party / Segmind reseller, USD)**：

| Mode | 5s | 10s |
|------|-----|-----|
| Std (`std`) | $0.28 | $0.56 |
| Pro (`pro`) | $0.98 | $1.96 |

**Per-video cost（Kling 官方 credit table — partial）**：

| Model | std (per 5s) | pro (per 5s) |
|-------|--------------|--------------|
| `kling-v1` | 5 cr | 10 cr |
| `kling-v1-6` | 10 cr | 20 cr |
| `kling-v2-master` / `kling-v2-5-turbo` | 待确认 (~20 cr) | 待确认 (~35 cr) |

> 10s videos = 2× the 5s cost。Higher tiers（`v3`, `o1`）and 1080p resolution add multipliers — **待确认** on exact rate。

**Daily quota（Free tier）**：
- New accounts: ~66 credits / day（historical）。Sufficient for ~3 standard v1.6 videos/day at 5s — **待确认**, this figure has shifted with each promo wave。
- Paid tiers: $5 / $30 / $66 monthly plans exist; credit packs also available。

**Rate limits**：
- Concurrent task limit per account: 待确认 (typical 3–5 in-flight)
- Submit rate: 待确认 (per-second limit undocumented publicly)
- Practical advice: poll every 5–10s; expected latency 30s – 5min depending on queue。

**Top-up channels**：
- **Global**: `https://app.klingai.com/global/dev` → console → Credits (Stripe / credit card)
- **China**: `https://platform.klingai.com` → 充值 (Alipay / WeChat Pay / 企业网银)

---

## 5. 限制

- **Auth**: **JWT Bearer token**, HS256-signed with AK (Access Key) + SK (Secret Key)。No static API key header。Token payload：
  ```json
  { "iss": "<AK>", "exp": <now+1800>, "nbf": <now> }
  ```
- **Geographic**: China region (Beijing) requires ICP-aligned access; international endpoint serves global。Network reachability varies by user ISP — Chinese users on domestic base URL, overseas users on `api.klingai.com`。
- **AK/SK acquisition**: Register at klingai.com → developer console → "Create Application" → copy AK + SK。
- **Video URL TTL**: signed CDN URLs expire in ~30 min — must download to AVCore storage immediately on `succeed`。
- **Content policy**: enforces Kuaishou's content safety filter; some prompts get rejected without refund — surface `task_status_msg` on `failed`。
- **Concurrent in-flight tasks** per AK/SK: 待确认 (recommend config: max 2 in flight)。

---

## 6. 关键差异 vs MiniMax

| Dimension | MiniMax | Kling |
|-----------|---------|-------|
| **Protocol** | OpenAI-compatible `Bearer <api_key>` header | Custom **JWT HS256** generated per call |
| **Sync vs Async** | t2a/t2v mostly synchronous (single request) | **Fully async** — submit + poll (or webhook) |
| **Poll loop** | 1-step (single request → response) | **2-step** (submit `task_id` → poll until `succeed`) |
| **Pricing unit** | Per-call flat (or character-based) | Per-video credits × `mode` × `duration` |
| **Free daily quota** | 3 videos/day (hardcoded in plan) | ~66 cr/day free (≈ 3–6 videos depending on mode) |
| **Video duration** | typically 6s fixed | 5s or 10s user-selectable |
| **Modes** | n/a | `std` / `pro` |
| **Image-to-video** | limited / specific API | first-class (`image2video` + `image_tail`) |
| **URL TTL** | static CDN | signed, **~30 min TTL** — must download immediately |

> Kling is **strictly async**, so AVCore's `VideoProvider::render` will need an internal poll loop (matches MiniMax video path style)。

---

## 7. Real curl Templates

### 7.1 Generate JWT (one-time per process, cached 25 min)

```bash
AK="your_access_key_here"
SK="your_secret_key_here"
NOW=$(date +%s)
EXP=$((NOW + 1800))

# Header (base64url, no padding)
b64url() { printf '%s' "$1" | base64 | tr '+/' '-_' | tr -d '='; }
HEADER=$(b64url '{"alg":"HS256","typ":"JWT"}')
PAYLOAD=$(b64url "{\"iss\":\"$AK\",\"exp\":$EXP,\"nbf\":$NOW}")
SIG=$(printf '%s.%s' "$HEADER" "$PAYLOAD" \
      | openssl dgst -sha256 -hmac "$SK" -binary \
      | base64 | tr '+/' '-_' | tr -d '=')
TOKEN="$HEADER.$PAYLOAD.$SIG"
echo "Bearer $TOKEN"
```

### 7.2 Text-to-Video submit

```bash
TOKEN="...from step 7.1..."
curl -sS -X POST "https://api.klingai.com/v1/videos/text2video" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "model_name": "kling-v1-6",
    "prompt": "A cat walking in a sunlit garden, cinematic, 1080p",
    "aspect_ratio": "16:9",
    "mode": "std",
    "duration": "5",
    "negative_prompt": "blur, watermark",
    "seed": 12345,
    "callback_url": "https://your.app/avcore/kling-callback"
  }'
# → {"code":0,"message":"success","data":{"task_id":"clxxx...","task_status":"submitted"}}
```

### 7.3 Image-to-Video submit

```bash
curl -sS -X POST "https://api.klingai.com/v1/videos/image2video" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "model_name": "kling-v1-6",
    "prompt": "the cat slowly turns its head and yawns",
    "image": "https://your.cdn/cat.png",
    "aspect_ratio": "16:9",
    "mode": "std",
    "duration": "5"
  }'
```

### 7.4 Poll task

```bash
TASK_ID="clxxx..."
while true; do
  RESP=$(curl -sS "https://api.klingai.com/v1/videos/text2video/$TASK_ID" \
    -H "Authorization: Bearer $TOKEN")
  echo "$RESP"
  STATUS=$(echo "$RESP" | jq -r '.data.task_status')
  [ "$STATUS" = "succeed" ] && break
  [ "$STATUS" = "failed" ] && exit 1
  sleep 8
done
URL=$(echo "$RESP" | jq -r '.data.video_urls[0]')
# Download immediately — signed URL TTL ~30 min
curl -sS -o out.mp4 "$URL"
```

---

## 8. 推荐 AVCore 集成

### 8.1 New files
- `src/provider/kling.rs` — `KlingProvider` implementing the existing `VideoProvider` trait
- `src/provider/kling_jwt.rs` (or inline) — JWT generation helper (`jsonwebtoken` crate or hand-rolled HMAC-SHA256)

### 8.2 Config schema (`avc.toml`)
```toml
[provider.video.kling_yu]
api_key = ""                          # 留空用 env AVCORE_KLING_AK + AVCORE_KLING_SK
ak = "AKLT..."                        # 留空用 env
sk = "sk-..."                         # 留空用 env
base_url = "https://api.klingai.com"   # 国际端
model = "kling-v1-6"                  # 默认
mode = "std"                          # std | pro
duration = "5"                        # "5" | "10" (only v1-5 supports 10s)
poll_secs = 8
max_wait_s = 600
concurrent = 2
```

> Reuse the existing `[providers.*]` block pattern that `minimax.rs` and `real.rs` use — drop a `#[serde(default)] KlingCfg` into `config.rs`。

### 8.3 Helper reuse
- **Auth header helper**: Kling doesn't use a single `Bearer <static>`, so MiniMax's `auth_header()` doesn't apply。New helper `kling_auth_header(ak, sk) -> HeaderMap` that signs a fresh JWT (cached by `(ak, sk)` with 25-min TTL — reuse a `OnceCell<Mutex<...>>`)。
- **Response envelope parsing**: New `handle_kling_response()` similar to `handle_response` in `minimax.rs` — checks HTTP status → checks `code != 0` → maps to `AvcError` variants:
  - `1001` (invalid auth) → `AvcError::TokenAuth`
  - `1002` / `1101` (insufficient credits / bad params) → `AvcError::Arg`
  - `429` → `AvcError::RateLimited`
  - `5xx` / unknown `code` → `AvcError::ProviderUpstream`
- **Poll loop**: New `poll_until_done(...)` similar to MiniMax video path. Backoff 5–10s, hard timeout from config。
- **Download**: New `download_signed_video(url) -> Vec<u8>` (Kling URLs expire fast — must download immediately, then base64-encode into existing `Clip { mp4_b64, mime, duration_ms }`)。

### 8.4 Trait implementation skeleton
```rust
pub struct KlingProvider {
    cfg: KlingCfg,
    http: reqwest::Client,
    token_cache: Arc<Mutex<Option<(String, Instant)>>>, // (jwt, exp)
}

impl KlingProvider {
    fn fresh_jwt(&self) -> AvcResult<String> { /* HMAC-SHA256 sign */ }
}

#[async_trait]
impl VideoProvider for KlingProvider {
    fn name(&self) -> &str { "kling" }

    async fn render(
        &self,
        voice: &Voice,
        avatar: &Avatar,
        scenes: &[ScriptSegment],
    ) -> AvcResult<Clip> {
        // 1. submit t2v for first scene (or aggregate prompt)
        // 2. poll until succeed
        // 3. download video bytes (signed URL TTL ~30min)
        // 4. probe duration via ffprobe (or assume config duration)
        // 5. return Clip { mp4_b64, mime: "video/mp4", duration_ms }
    }
}
```

### 8.5 Register
- Add `pub mod kling;` to `src/provider/mod.rs`
- Wire into `make_video()` in `real.rs` alongside MiniMax dispatch (`name.ends_with("_kling")`)

---

## 9. 待确认 (TBD — needs user verification)

| Item | Why unclear | How to verify |
|------|-------------|---------------|
| Exact free daily credit grant | Promos shift it each quarter | Sign up at `app.klingai.com/global/dev` and read the welcome email / dashboard |
| Exact v2-master / v2-5-turbo credit cost | Only v1 / v1-6 tables confirmed | Hit `klingai.com/dev` pricing page, or run 1 test submit and inspect credits-deducted |
| Concurrent in-flight task limit per AK/SK | Not in public docs | Send 3 parallel submits, observe which gets `code:1300` |
| `kling-v3` / `kling-v3-omni` / `kling-video-o1` API surface | Multi-modal O1 may use different params | Check `klingai.github.io/kling-docs/api-reference/` once accessible |
| Submit rate (req/sec) | Undocumented | Empirical load test |
| 1080p flag (kling-v1-6 supports it — param name?) | Some sources mention `resolution: "1080p"` | Try `"resolution":"1080p"` in test body, see if accepted |
| Video URL exact TTL (30min vs 24h) | Reported inconsistently | Inspect `Cache-Control` / `Expires` headers on returned URL |
| Whether domestic `api-beijing.klingai.com` accepts overseas cards | Often no | Test with user's card |

---

## Sources

- [Kling AI — 官方主页](https://klingai.com/)
- [Kling Developer Portal (国际)](https://app.klingai.com/global/dev)
- [Kling Developer Portal (国内)](https://platform.klingai.com)
- [Kling 官方 API 文档 (GitHub Pages)](https://klingai.github.io/kling-docs/api-reference/api-quick-start.html)
- [Kling 官方 API 文档 — 任务查询](https://klingai.github.io/kling-docs/api-reference/api-videos-get.html)
- [可灵 (Kling) AI API 接入实战指南 (CSDN, u012172506)](https://blog.csdn.net/u012172506/article/details/160342648)
- [Kling 视频生成 API 集成指南 (CSDN, xinxin_0916)](https://blog.csdn.net/xinxin_0916/article/details/160096919)
- [Kling 视频生成 API 集成指南 (CSDN, gao_tjie)](https://blog.csdn.net/gao_tjie/article/details/159690566)
- [Kling 3.0 文生视频 / 图生视频 API 实战 (CSDN)](https://blog.csdn.net/2601_95717211/article/details/159763583)
- [可灵 (Kling) 视频 API 在 111API 平台的对接配置 (CSDN)](https://blog.csdn.net/q1379610856/article/details/159252250)
- [可灵 AI V1.6 模型已开放 API (AI 创业之家)](https://www.cy211.cn/aizixun/5445.html)
- [AceDataCloud/KlingMCP (GitHub)](https://github.com/AceDataCloud/KlingMCP)
- [199-mcp/mcp-kling — kling-api-docs.md (GitHub)](https://github.com/199-mcp/mcp-kling/blob/main/kling-api-docs.md)
- [op7418/NanoBanana-PPT-Skills — kling_api.py (GitHub)](https://github.com/op7418/NanoBanana-PPT-Skills/blob/main/kling_api.py)
- [Kling Pricing (Segmind reseller)](https://www.segmind.com/models/kling-text2video/pricing)
- [Kling vs Seedance vs Sora 对比 (CSDN)](https://blog.csdn.net/weixin_30735745/article/details/95691493)
- [Adobe Firefly — Kling Video Generation](https://helpx.adobe.com/ro/firefly/web/firefly-video-editor/generate-videos/generate-videos-using-kling.html)
