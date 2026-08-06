# Vidu (生数科技) Video API 集成 Spec for AVCore

> 研究日期：2026-08-05
> 信息汇总 + 待确认项标记

---

## 1. 推荐 Endpoint

**不走** `vidu.studio` 直接提供的接口（部分资料待确认，且与 Aliyun 不兼容，文档分散）。
**推荐走阿里云百炼 (Model Studio / DashScope) 通道**：

- 同一份 DashScope API Key 即可调用 Vidu，与 Wanx / 可灵 共用账号，免费额度、计费、QPS 限制统一在阿里云控制台可见。
- AVCore 的 `provider` 抽象（`Arc<dyn VideoProvider>`）已与具体 HTTP 服务解耦，新增 provider 只需实现 `submit / poll / fetch` 三个阶段。

---

## 2. 端点 (DashScope 异步视频生成)

| 阶段 | 方法 | URL | Body | Response |
|------|------|-----|------|----------|
| 提交任务 | `POST` | `https://dashscope.aliyuncs.com/api/v1/services/aigc/video-generation/video-synthesis` | `{"model", "input":{"prompt","img_url"?}, "parameters":{duration,resolution,aspect_ratio,movement_amplitude,seed?}}` | `{"output":{"task_id","task_status":"PENDING"},"request_id"}` |
| 查询任务 | `GET` | `https://dashscope.aliyuncs.com/api/v1/tasks/{task_id}` | (Header only) | `{"output":{"task_id","task_status","video_url","submit_time","end_time","orig_prompt"},"usage":{video_count,video_duration,video_ratio}}` |
| 获取视频 | `GET` | `video_url` (返回的是 OSS URL) | (无) | 二进制 mp4 |

**Header 通用**：
```
Authorization: Bearer ${DASHSCOPE_API_KEY}
Content-Type: application/json
```

**错误响应格式**（统一 DashScope 协议）：
```json
{ "code": "InvalidParameter", "message": "...", "request_id": "..." }
```

**任务状态机**：`PENDING → RUNNING → SUCCEEDED | FAILED | CANCELED`

---

## 3. 模型 + 参数

| 模型名 | 类型 | 时长 | 分辨率 | 比例 | 说明 |
|--------|------|------|--------|------|------|
| `vidu-q1` | 文生视频 / 图生视频 | 1~8s | 720p / 1080p | 1:1, 16:9, 9:16, 4:3, 3:4, 21:9 | 当前 Q1 系列主力模型 |
| `vidu-q1-turbo` | 文生视频 | 1~8s | 720p | 同上 | 速度优先版本 |
| `vidu2.0` | 文生视频 / 图生视频 | 1~8s | 720p / 1080p | 同上 | Vidu 2.0 系列 |
| `vidu-q2` | 图生视频 | 2~8s | 720p / 1080p | 同上 | 2025-09 新发布；2 种风格（电影大片 / 闪电出片） |
| `vidu1.5` | 文/图生视频 | 1~8s | 720p | 同上 | 上代模型，仍可用 |

**参数细则**：
- `duration`：整数，1~8 秒
- `resolution`：`"720p"` 或 `"1080p"`（1080p 通常双倍积分）
- `aspect_ratio`：`"1:1" / "16:9" / "9:16" / "4:3" / "3:4" / "21:9"`
- `movement_amplitude`：`"auto"` / `"small"` / `"medium"` / `"large"`（vidu-q1 特有）
- `seed`：可选，复现用
- `input.img_url`：图生视频时必填，公网可访问的 JPEG/PNG

⚠️ **完整模型名 / 最新参数** 以 [阿里云百炼模型广场](https://bailian.console.aliyun.com/) 为准。

---

## 4. 价格 + 配额

| 项目 | 数值 |
|------|------|
| 计费单位 | 积分（1 积分 ≈ ¥0.28） |
| vidu-q1 标准价 | 0.5 积分/秒 → 约 ¥0.14/秒 |
| 长视频阶梯折扣 | ≥5s 0.4 积分/秒；≥10s 0.3 积分/秒（待确认是否仍生效） |
| 1080p 倍率 | 通常为 720p 的 2 倍积分 |
| 失败任务 | 不扣积分（按官方说明） |
| 新用户免费额度 | 阿里云百炼新用户每个模型 100 万 Token 等价试用金（视频模型按秒折算，待确认具体秒数） |
| 免费额度有效期 | 90 天（2025-09-08 起调整） |
| 视频 URL 有效期 | OSS 临时 URL，默认 24 小时（必须在 fetch 阶段下载） |

**官方平台（vidu.studio/vidu.com）独立价格**：
- Vidu Q1 官方定价：1080p 约 ¥0.3/秒（早期定价）
- Vidu 2.0：¥0.04/秒（"4 分钱" 营销价）

**档位 QPS 限制**（官方 Vidu 平台的积分等级对应）：
- Tier 0（默认）：5 tasks/min，100 tasks/day
- Tier 1（≥5000 积分）：20 tasks/min，2000 tasks/day，并发 15
- Tier 2（≥50000 积分）：60 tasks/min，20000 tasks/day

⚠️ 以上 Tier 限制来自 Vidu 官方积分文档；阿里云百炼通道的 QPS 限制**待确认**（大概率共用或更宽松）。

---

## 5. 限制

- **地域限制**：Aliyun DashScope 视频生成 API Key 须使用「华北 2（北京）」地域；海外用户需确认是否可注册。
- **异步任务**：必须 submit → poll → fetch 三步，不可同步调用。
- **任务超时**：未明确上限，常规任务 30s~3min。
- **图片输入**：仅支持公网 URL（不支持 base64），需先上传到 OSS / CDN。
- **视频 URL 过期**：24 小时，不下载即失效。
- **moderation**：自动内容审核，违规 prompt 会被直接 reject。

---

## 6. 关键差异 (vs MiniMax)

| 维度 | MiniMax | Vidu (DashScope) |
|------|---------|------------------|
| 通道 | `api.minimax.chat` 直接 | `dashscope.aliyuncs.com` 阿里云百炼 |
| 同步/异步 | 异步（也走 task_id） | 异步 |
| 阶段数 | 3 步（submit/poll/fetch） | 3 步（结构相同） |
| 单日限额 | 3 视频/天（硬限） | 100+ 任务/天（Tier 0） |
| 模型 | MiniMax-Hailuo-02 / 2.3 / T2V-01 | vidu-q1 / vidu-q2 / vidu2.0 |
| 视频时长 | 6s / 10s | 1~8s |
| 计费 | 包月 / 套餐 | 按秒积分（按量） |
| 鉴权 | Bearer JWT (`MiniMax-Api-Key`) | Bearer API Key (`DASHSCOPE_API_KEY`) |
| 控制台 | hailuoai.video | bailian.console.aliyun.com |
| 优势 | 中文 prompt 优化好 | 价格更便宜，所有阿里云模型统一账户 |
| 劣势 | 配额极严 | 中文 prompt 偶尔需精调 |

---

## 7. 实测 curl 模板

```bash
# === 1. 提交任务（图生视频） ===
curl -X POST "https://dashscope.aliyuncs.com/api/v1/services/aigc/video-generation/video-synthesis" \
  -H "Authorization: Bearer ${DASHSCOPE_API_KEY}" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "vidu-q1",
    "input": {
      "prompt": "一只柴犬在樱花树下奔跑，逆光，电影感",
      "img_url": "https://example.com/dog.jpg"
    },
    "parameters": {
      "duration": 5,
      "resolution": "720p",
      "aspect_ratio": "16:9",
      "movement_amplitude": "auto",
      "seed": 12345
    }
  }'

# === 2. 轮询任务状态 ===
TASK_ID="c2cbb950-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
curl -X GET "https://dashscope.aliyuncs.com/api/v1/tasks/${TASK_ID}" \
  -H "Authorization: Bearer ${DASHSCOPE_API_KEY}"

# === 3. 任务完成后下载视频 ===
VIDEO_URL="https://dashscope-result-bj.oss-cn-beijing.aliyuncs.com/..."
curl -L -o output.mp4 "${VIDEO_URL}"
```

**仅文生视频（无 img_url）**：
```bash
curl -X POST "https://dashscope.aliyuncs.com/api/v1/services/aigc/video-generation/video-synthesis" \
  -H "Authorization: Bearer ${DASHSCOPE_API_KEY}" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "vidu-q1",
    "input": { "prompt": "海浪拍岸，夕阳，电影感镜头" },
    "parameters": { "duration": 5, "resolution": "720p", "aspect_ratio": "16:9" }
  }'
```

---

## 8. 推荐 AVCore 集成

### 8.1 新文件结构

```
src/provider/
├── minimax.rs      # 已有
├── vidu.rs         # 新增：ViduDashScopeVideoProvider
├── real.rs         # 改：append `if name.ends_with("_vidu")` 分支
└── mod.rs          # 改：注册 `pub mod vidu;`
```

### 8.2 实现路径

1. **新建 `src/provider/vidu.rs`**：
   - 实现 `VideoProvider` trait（`render` 方法）
   - 内部三步：`submit_task` → `poll_task(每 5s, 超时 600s)` → `download_to_b64`
   - 复用 `crate::provider::Clip` 数据结构 (`mp4_b64` / `mime` / `duration_ms`)
   - 复用 `AvcError` / `AvcResult` 错误体系

2. **关键签名**：
   ```rust
   pub struct ViduDashScopeVideoProvider {
       api_key: String,
       base: String,  // "https://dashscope.aliyuncs.com/api/v1"
       client: reqwest::Client,
       poll_interval: Duration,
       max_wait: Duration,
   }
   impl ViduDashScopeVideoProvider {
       pub fn new(api_key: String) -> Self { ... }
   }
   #[async_trait]
   impl VideoProvider for ViduDashScopeVideoProvider {
       fn name(&self) -> &str { "vidu" }
       async fn render(&self, voice: &Voice, avatar: &Avatar, scenes: &[ScriptSegment]) -> AvcResult<Clip> { ... }
   }
   ```

3. **复用 helper**：
   - `CliVideoProvider` 的 `submit / status / fetch` 逻辑可抽到 `provider/real.rs` 抽出 `pub(crate) async fn submit_task / poll_task / download_video` 三个泛型 helper（用 trait `AsyncSubmitEndpoint`），让 Vidu 与现有 CliVideoProvider 共用。
   - 或者更保守：直接复制 `CliVideoProvider` 的轮询/下载代码到 `vidu.rs`，代价是 ~50 行重复。

4. **注册到 `make_video()`** (`real.rs:1402`)：
   ```rust
   if name.ends_with("_vidu") {
       let api_key = resolve_api_key(&cfg, "vidu")?;
       return Ok(Arc::new(super::vidu::ViduDashScopeVideoProvider::new(api_key)));
   }
   ```

### 8.3 配置格式 (`avc.toml`)

```toml
[provider.video.hailuo_vidu]
kind = "vidu"                   # 触发 ViduDashScopeVideoProvider
binary = ""                     # Vidu 通道不需要
api_key = "${DASHSCOPE_API_KEY}"  # 走 env 或直接 sk-xxx
model = "vidu-q1"               # 可选，默认 vidu-q1
resolution = "720p"             # 可选
aspect_ratio = "16:9"           # 可选
duration_seconds = 5            # 可选，默认 5
poll_interval_ms = 5000         # 可选
max_wait_seconds = 600          # 可选
```

### 8.4 行号参考

- `provider/real.rs:1402` — `make_video()` factory 函数，新分支插这里
- `provider/real.rs:988-1180` — `CliVideoProvider` 现成的 submit/poll/fetch 模板
- `provider/minimax.rs` — 已有纯 HTTP provider 范例（完美对照）
- `provider/mod.rs:118-127` — `VideoProvider` trait 定义

---

## 9. 待确认

| 项 | 状态 | 建议核实方式 |
|----|------|--------------|
| 阿里云百炼 Vidu 的具体秒单价（720p vs 1080p 是否仍为 2x） | 待确认 | bailian.console.aliyun.com → 模型详情页 |
| 新用户视频免费额度具体秒数 | 待确认 | 阿里云百炼免费额度公告页 |
| 长视频阶梯折扣（≥5s / ≥10s）是否仍生效 | 待确认 | 阿里云百炼计费 FAQ |
| Aliyun DashScope 视频 QPS 限速具体值 | 待确认 | 阿里云百炼 API 限流文档 |
| `platform.vidu.studio` 是否提供独立 API（区别于百炼） | 待确认 | 尝试 fetch 该站点（WebFetch 失败，建议浏览器实测） |
| 海外用户是否可注册阿里云账号 | 待确认 | 阿里云国际版 vs 国内版差异 |
| `vidu-q2` 是否已在百炼模型广场上架 | 待确认 | 模型广场搜索 "Vidu" |
| OSS 视频 URL 有效期是否 24h | 待确认 | 实测一次性 |
| 图生视频是否支持 `input.images` 数组（多图参考） | 待确认 | 文档或实测 |
| Vidu Q1 是否支持 `audio` 参数（生成同步音轨） | 待确认 | 模型说明页 |

---

## Source URLs

- [阿里云百炼 Model Studio 主入口](https://bailian.console.aliyun.com/)
- [阿里云百炼 SDK 安装文档](https://help.aliyun.com/zh/model-studio/developer-reference/install-sdk)
- [阿里云百炼计费说明](https://help.aliyun.com/zh/model-studio/billing-for-model-studio)
- [阿里云百炼新用户免费额度](https://help.aliyun.com/zh/model-studio/new-free-quota)
- [阿里云百炼视频 API 汇总](https://www.aliyun.com/sswb/1101833.html)
- [Vidu AI 官方平台 (国内)](http://www.vidu.com/)
- [Vidu 平台文档 (platform.vidu.studio)](https://platform.vidu.studio) — WebFetch 失败，需浏览器直访
- [Vidu Q1 API 全球开放新闻 (Sohu)](https://www.sohu.com/a/893810640_362225)
- [生数科技发布 Vidu 2.0 (新浪财经)](https://finance.sina.com.cn/roll/2025-01-15/doc-inefachv7679948.shtml)
- [Vidu Q1 上线 + 性能超 Runway/Sora (新浪财经)](https://finance.sina.com.cn/tech/2025-04-23/doc-ineucyit9900490.shtml)
- [Vidu Q2 发布 (Tencent News)](https://new.qq.com/rain/a/20250925A042Q000)
- [阿里云百炼 Vidu 文生视频 curl 示例 (CSDN 综合)](https://blog.csdn.net/SmartTony/article/details/157389375)
- [DashScope SDK 用法 (博客园)](https://www.cnblogs.com/javatoai/p/19272309)
