# 字节 Seedance API 适配 spec

> AVCore 接入字节 Seedance 视频生成模型，替代 MiniMax 的 3 视频/天硬限。
> 调研日期：2026-08-05。

---

## 1. 推荐 endpoint

**火山方舟 (Volcengine Ark)** — `https://ark.cn-beijing.volces.com/api/v3`

理由：
- 是字节官方对外开放的统一推理网关（OpenAI 兼容 auth 风格）
- Seedance 全系列 (1.0 lite / 1.0 pro / 1.5 pro) 都通过 `/contents/generations/tasks` 异步端点暴露
- 字节没有对外公开"直连"开放平台的 endpoint（只能走方舟）
- 与 MiniMax 用"先 task_id → 再 file_id → 再 download_url"不同，Ark 在轮询响应里直接给 `content.video_url`，少一段

---

## 2. 端点

| 阶段 | 方法 | URL | body | response |
|---|---|---|---|---|
| submit | POST | `{base}/contents/generations/tasks` | 见下表 | `{ "id": "cgt-..." }` |
| poll | GET | `{base}/contents/generations/tasks/{id}` | — | 见下表 |
| fetch | (无需) | — | — | `succeeded` 响应里直接含 `content.video_url`，下载即可 |

**Submit body（Seedance 1.0 pro / 1.5 pro）**：
```json
{
  "model": "doubao-seedance-1-0-pro-250528",
  "content": [
    { "type": "text", "text": "<prompt>" },
    { "type": "image_url", "image_url": { "url": "https://..." } }
  ],
  "ratio": "16:9",
  "duration": 5,
  "resolution": "720p",
  "seed": 12345,
  "camera_fixed": false,
  "watermark": false
}
```

**Poll response (queued / running)**：
```json
{
  "id": "cgt-xxx",
  "model": "doubao-seedance-1-0-pro-250528",
  "status": "running",
  "created_at": "...",
  "updated_at": "..."
}
```

**Poll response (succeeded)**：
```json
{
  "id": "cgt-xxx",
  "model": "doubao-seedance-1-0-pro-250528",
  "status": "succeeded",
  "content": {
    "video_url": "https://ark-content-generation-cn-xxx.volces.com/...mp4"
  },
  "usage": { "generated_tokens": N, "total_tokens": N },
  "created_at": "...",
  "updated_at": "..."
}
```

**Poll response (failed)**：
```json
{
  "id": "cgt-xxx",
  "status": "failed",
  "error": { "code": "...", "message": "..." }
}
```

**Auth header**：`Authorization: Bearer <ARK_API_KEY>`（与 MiniMax 同款 Bearer）。

---

## 3. 模型 + 参数

| model id | 能力 | duration | ratio | resolution | 价格参考 |
|---|---|---|---|---|---|
| `doubao-seedance-1-0-pro-250528` | t2v + i2v（首帧/尾帧），无音 | 5 / 10s | 16:9, 9:16, 1:1, 4:3, 3:4, 21:9 | 480p / 720p | 5s 480p ≈ 1.03 元；5s 720p ≈ 1.73 元 |
| `doubao-seedance-1-0-lite-t2v-250428` | 纯文生视频 | 5 / 10s | 同上 | 480p / 720p | lite 比 pro 便宜 ~50%（待确认） |
| `doubao-seedance-1-0-lite-i2v-250428` | 图生视频 + 首/尾帧 | 同上 | 同上 | 同上 | lite |
| `doubao-seedance-1-5-pro-251215` | t2v + i2v，**有声/无声可选**，支持高自由度相机 | 5 / 10s | 同上 | 720p 为主 | 有声 ≈ 0.35 元/秒；无声 ≈ 0.17 元/秒（720p 24fps） |
| `doubao-seedance-2-0-pro-...` | 工业级多模态 | 5 / 10s | 同上 | 480p / 720p | ≈ 1 元/秒 |

**计费方式**：按输出视频 token 计费（元/百万 token），仅成功生成收费。token ≈ `宽 × 高 × 帧率 × 时长 / 1024`。

---

## 4. 限制

- **异步**：所有 Seedance 任务都是异步，必须 submit → poll。5s 视频通常 ~1 分钟
- **图片要求**：URL 形式（不支持本地文件直传），JPEG/PNG，128×128 ~ 2048×2048
- **首/尾帧**：1.0 pro 支持，lite 只支持 first_frame；同一请求只接受 1 张参考图
- **配额**：方舟按账户配额（默认 5~50 并发），不像 MiniMax 有"3 视频/天"硬限（**这是核心替换动机**）
- **状态枚举**：`queued` / `running` / `succeeded` / `failed` / `cancelled`

---

## 5. 关键差异（vs MiniMax）

| 维度 | MiniMax | 火山方舟 Seedance |
|---|---|---|
| 流程段数 | **3 段**：submit → poll → retrieve(file_id) → download | **2 段**：submit → poll（video_url 直接在 poll 响应里） |
| 状态枚举 | `Success` / `Fail`（首字母大写） | `succeeded` / `failed`（小写） |
| 响应包装 | `{ "task_id":..., "base_resp": {...} }` | 扁平 `{ "id":..., "status":..., "content":... }` |
| 时长字段 | 不可控，由 prompt 决定 | `duration: 5 \| 10`（秒） |
| 比例字段 | 无（视频自由） | `ratio: "16:9" \| ...` |
| 图片参考 | 不支持 | 支持（首/尾帧） |
| 鉴权 | `Bearer <key>` | `Bearer <key>`（一致） |
| 错误格式 | `base_resp.status_code` (0/2013/...) | 顶层 `error: {code, message}` 或 HTTP 状态码 |
| **每日配额硬限** | 3 视频/天 | 无（按账户配额） |

---

## 6. 实测 curl 示例（smoke test 模板 — 等用户提供 ARK_API_KEY 再跑）

```bash
ARK_BASE="https://ark.cn-beijing.volces.com/api/v3"
ARK_KEY="<from user>"
MODEL="doubao-seedance-1-0-pro-250528"

# 1) submit
TASK=$(curl -sS -X POST "$ARK_BASE/contents/generations/tasks" \
  -H "Authorization: Bearer $ARK_KEY" \
  -H "Content-Type: application/json" \
  -d "{
    \"model\": \"$MODEL\",
    \"content\": [{\"type\":\"text\",\"text\":\"a cat walking in the garden\"}],
    \"ratio\": \"16:9\",
    \"duration\": 5,
    \"resolution\": \"720p\"
  }" | jq -r .id)

echo "task_id=$TASK"

# 2) poll
while true; do
  R=$(curl -sS "$ARK_BASE/contents/generations/tasks/$TASK" \
    -H "Authorization: Bearer $ARK_KEY")
  S=$(echo "$R" | jq -r .status)
  echo "status=$S"
  [ "$S" = "succeeded" ] && break
  [ "$S" = "failed" ] && { echo "$R"; exit 1; }
  sleep 5
done

URL=$(echo "$R" | jq -r .content.video_url)
echo "video_url=$URL"

# 3) download
curl -sS "$URL" -o /tmp/seedance-test.mp4
ls -lh /tmp/seedance-test.mp4
```

---

## 7. 推荐 AVCore 集成方式

### a. 新文件 `src/provider/seedance.rs`（推荐）
不并入 `minimax.rs`，理由：
- 异步状态机不同（3 段 vs 2 段，没有 file_id）
- 错误包装不同（base_resp vs 扁平）
- 鉴权前缀虽然一样，但 Ark 的 key 来源是 Volcengine 控制台，与 MiniMax key 是不同的产品
- 单文件独立便于后续单独加 lite / 1.5-pro / 2.0 变体

### b. `src/provider/mod.rs` 改造
```rust
pub mod minimax;
pub mod mock;
pub mod probe;
pub mod real;
pub mod seedance;   // 新增
```

### c. 配置段
复用现有 `Providers.video: HashMap<String, ProviderCfg>`：
```toml
[provider.video.seedance]
api_key = "<ARK_API_KEY>"          # ARK 控制台 -> API Key
model = "doubao-seedance-1-0-pro-250528"
base_url = "https://ark.cn-beijing.volces.com/api/v3"
# extra_headers 可留空；如有 custom region/endpoint 再加
```

**配置 key 后缀**：`seedance`（不是 `_seedance`，与 minimax 平级）。后续要换 lite / 1.5-pro 时配 `provider.video.seedance_lite` / `provider.video.seedance15` 或直接 `model = "..."`。

### d. helper 复用
| helper | 复用？ | 备注 |
|---|---|---|
| `auth_header` | ✅ 复用 | 直接 `use crate::provider::minimax::auth_header;` |
| `handle_response` | ❌ 不直接复用 | Ark 无 `base_resp` 包装，写一个 `seedance::handle_response`（更简单，只判 HTTP status + 顶层 `error`） |
| `wait_video_done` | ❌ 不直接复用 | Ark 状态枚举不同（`succeeded`/`failed` vs `Success`/`Fail`），且成功响应里直接给 video_url，不需要 file_id 中间步。写 `seedance::wait_video_done` 返回 `video_url: String` 即可 |

### e. `real.rs` 工厂
新增（沿用 minimax 的形态）：
```rust
pub fn make_video(cfg: &Config, name: &str) -> AvcResult<Arc<dyn VideoProvider>> {
    let pc = cfg.provider.video.get(name)
        .ok_or_else(|| AvcError::NotFound(format!("provider.video.{}", name)))?;
    match name {
        "seedance" | s if s.starts_with("seedance") =>
            Ok(Arc::new(SeedanceVideoProvider::new(name.to_string(), pc.clone())?)),
        "minimax" | s if s.starts_with("minimax") | s == "minimax_video" =>
            Ok(Arc::new(MiniMaxCompatVideoProvider::new(name.to_string(), pc.clone())?)),
        _ => Ok(Arc::new(CliVideoProvider::new(name.to_string(), pc.clone())?)), // 兜底：vendor CLI
    }
}
```

### f. `VideoProvider::render` 实现要点
- `scenes` 拼成一段 text prompt 即可（Ark 一次只接受 1 个 text）
- `avatar` 暂不用（M2.5 才决定是否支持 image_url 首帧）；可预留：把 avatar primary_png_b64 临时上传拿 URL 再喂 image_url
- `voice` 不用
- `duration` 从 scenes 总时长推：5 / 10 向上取整；如果超 10s 截断或 warn（待确认业务策略）
- `ratio` 从 `AvatarSpec.style` 或新增配置读，默认 `16:9`
- `resolution` 默认 `720p`

---

## 8. 待确认（不要猜测，标 TODO）

- [ ] **Seedance 1.0 pro** 的准确官方 token 计费（火山方舟官方定价页 `volcengine.com/docs/82379/1099320` 未抓到）
- [ ] **Seedance 1.5 pro** 的 image_url 首尾帧是否同时支持（lite 只支持首帧）
- [ ] 方舟 API key 是否区分**预付费 / 后付费**对配额/限速的影响
- [ ] video_url 的有效期（推测 24h，待验证）
- [ ] 是否需要走方舟的 `X-Client-Request-Id` 等幂等 header
- [ ] `scenes` 总时长 > 10s 时的策略：截断 vs 拆段 vs 拒绝

---

## 来源

- [火山方舟 Seedance 技术文档](https://www.volcengine.com/docs/82356/1666946)（官方，WebFetch 被网络策略拦截，未抓到正文 — **待确认**）
- [火山方舟费用参考（ArcReel 维护的对照表）](https://github.com/ArcReel/ArcReel/blob/main/docs/ark-docs/%E7%81%AB%E5%B1%B1%E6%96%B9%E8%88%9F%E8%B4%B9%E7%94%A8%E5%8F%82%E8%80%83.md)
- [火山引擎 Seedance 2.0 API 服务全面开放](https://so.html5qq.com/page/real/search_news?docid=70000021_83569e0abd260452)
- [火山引擎 Seedance 1.0 lite 多图参考+首尾帧](https://www.163.com/dy/article/JVEUC5J5053179F1.html)
- [Java 接入 doubao-seedance-1.5-pro](https://blog.csdn.net/weixin_36332085/article/details/159283145)
- [Seedance 2.0 API 完整教程](https://blog.csdn.net/programmerjob/article/details/160150513)
- [Seedance 1.0 Pro 模型介绍与定价](https://so.html5.qq.com/page/real/search_news?docid=70000021_24269d1e75433752)
- [豆包 Seedance 2.0 1秒1元定价](https://www.ithome.com/0/925/937.htm)

## AVCore 关键文件路径（参考，未修改）

- `/home/ubuntu/avcore/src/provider/minimax.rs` — MiniMax 现有 video provider（`MiniMaxCompatVideoProvider`、`wait_video_done`、`auth_header`、`handle_response`）
- `/home/ubuntu/avcore/src/provider/mod.rs` — 需要新增 `pub mod seedance;` 并可能新增 `make_video` 工厂
- `/home/ubuntu/avcore/src/provider/real.rs` — `make_video` 工厂在文件后面
- `/home/ubuntu/avcore/src/config.rs` — `Providers.video: HashMap<String, ProviderCfg>` 已就位，无需改结构
- `/home/ubuntu/avcore/docs/providers.md` — 第 149-181 行 video 章节，后续补 `seedance` 段落
- `/home/ubuntu/avcore/docs/superpowers/specs/2026-08-04-minimax-provider-design.md` — 现有 spec 模板，新 spec 可参考其格式

## 一句话结论

方舟 Seedance 与 MiniMax 都是异步任务，但 Ark 更短：submit → poll → 直接拿 video_url 下载；建议新建 `src/provider/seedance.rs`，复用 `auth_header`，自写一份 2 段式的 `wait_video_done`（返回 `video_url` 而非 `file_id`），配置 key 用 `provider.video.seedance`。
