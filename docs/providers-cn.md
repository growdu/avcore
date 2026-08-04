# 国产大模型选型与配置

> 国内用户选 LLM / 视频模型怎么挑、怎么配、数据合规怎么考虑。
> 配合 `docs/providers.md`（5 维度协议层细节）使用——本文是国内厂商篇。

---

## 1. 选型考虑 4 维

| 维度 | 关键问题 |
|---|---|
| **任务适配** | 脚本生成（短）vs 长文 / 代码 / 中文文案 — 不同模型有强弱 |
| **数据合规** | 跨境数据出境？企业备案？私有化部署？ |
| **性能价格** | 1k token 多少钱？响应延迟？并发配额？ |
| **网络可达性** | 国内直连 vs 需要走代理？海外服务经常被墙 |

**AVCore 优势**：5 维度解耦，LLM / embed / avatar / voice / video 各自独立配，可以混搭：脚本生成用一家、形象生成用另一家、视频生成用第三家。

---

## 2. 国产 LLM 选型矩阵

| Provider | 代表模型 | 上下文 | 中文质量 | OpenAI 兼容 | 价格（输入/输出 元/1k token） | 适合 |
|---|---|---|---|---|---|---|
| **minimax** | minimax-大模型 | 8k-128k | ★★★★★ | ✅ `/v1` 端点 | ≈0.001 / 0.008 | 长文本 / 多语言 / 高质量 |
| **DeepSeek** | `deepseek-chat` / `deepseek-reasoner` | 32k-128k | ★★★★ | ✅ `/v1` 端点 | ≈0.0001 / 0.0002（缓存命中） / 0.002 | **极低成本**，脚本生成首选 |
| **通义千问 Qwen** | `qwen-plus` / `qwen-max` / `qwen-coder-plus` | 32k-1M | ★★★★★ | ✅ `compatible-mode/v1` 端点 | ≈0.004 / 0.012 | 代码 / 工具调用 / 长文 |
| **智谱 GLM** | `glm-4-plus` / `glm-4-flash` / `glm-z1-air` | 8k-128k | ★★★★★ | ✅ `/api/paas/v4` 端点 | ≈0.0001（flash） / 0.07（plus） | 中文专家 / 推理 |
| **豆包 Doubao** | `doubao-pro-32k` / `doubao-lite-32k` | 32k-128k | ★★★★★ | ✅ `/api/v3` 端点（火山方舟） | ≈0.0008 / 0.001 | 字节生态 / 极致便宜 |
| **月之暗面 Moonshot** | `moonshot-v1-128k` / `kimi-k2` | 8k-128k | ★★★★ | ✅ `/v1` 端点 | ≈0.012 / 0.012 | 长上下文 / 文档分析 |
| **腾讯混元** | `hunyuan-pro` / `hunyuan-standard` | 32k | ★★★★ | ✅（需走 API 3.0） | ≈0.03 / 0.06 | 腾讯生态 |
| **文心一言** | `ernie-4.0` / `ernie-3.5` | 8k | ★★★★ | ❌（不兼容 OpenAI） | 需走 API 3.0 | 百度生态；不推荐 AVCore |
| **MiniMax** | `abab6.5s-chat` / `abab6.5g-chat` | 8k-256k | ★★★★ | ✅ `/v1` 端点 | ≈0.001 / 0.001 | 长上下文 / 多模态 |

> ⚠️ 价格随时变动，以各家官网为准。上表为 2026-08 截取。

**推荐起步**：
- **预算紧 / 高频脚本**：`deepseek-chat` 或 `doubao-lite-32k`（每千 token 几分钱）
- **质量优先 / 多语言**：`minimax` 或 `qwen-max`
- **代码 / 工具调用**：`qwen-coder-plus` 或 `deepseek-coder`
- **长文档分析**：`moonshot-v1-128k` 或 `qwen-long`

---

## 3. 国产视频模型选型矩阵

视频模型都走 **vendor CLI 三段式**（`submit / status / fetch`），所以价格/能力比 OpenAI 兼容性重要。

| Provider | 代表模型 | 最大时长 | 画质 | 价格（每秒视频） | 适合 |
|---|---|---|---|---|---|
| **可灵 Kling**（快手）| `kling-1.6` / `kling-1.5` | 5-10s | 1080p | ≈0.1-0.5 元 | 通用 / 写实人像 / 动态强 |
| **即梦 Jimeng**（字节）| `jimeng-3.0` / `jimeng-video-3.0-pro` | 5-12s | 1080p | 按次计价（≈0.5-2 元/次） | 中文场景 / 特效 |
| **Vidu**（生数）| `vidu-2.0` / `vidu-q1` | 5-8s | 1080p | 按次（≈1 元/次） | 角色一致性 / 动漫风 |
| **智谱 CogVideoX** | `cogvideox-5b` / `cogvideox-2` | 6-10s | 720p-1080p | 按次 | 开源可本地 |
| **海螺 Hailuo**（MiniMax）| `MiniMax-video-01` | 6s | 720p-1080p | 按次 | 电影感 / 运动自然 |
| **腾讯智影** | 暂未公开视频 API | - | - | - | 微信生态 |
| **可图 Kualitu**（快手）| 静态图生成，不是视频 | - | - | - | 仅 avatar，不算 video |

> **不推荐**：百度文心视频（API 限制严）、商汤（无公开视频 API）。

**推荐起步**：
- **质量优先 + 中文场景**：`即梦-3.0-pro`
- **角色一致性（最接近"演员"）**：`Vidu-2.0`
- **写实人像 / 写实风格**：`Kling-1.6`（默认推荐）
- **可本地部署**：`CogVideoX`（开源）+ 自行部署

---

## 4. Embedding 选型

finetune drift eval 用，单次量小（每个 drift eval 调 2-4 次）：

| Provider | 模型 | 价格（元/1k token） | 备注 |
|---|---|---|---|
| **智谱 GLM Embedding** | `embedding-2` | ≈0.0005 | 中文友好 |
| **通义 DashScope** | `text-embedding-v3` | ≈0.0007 | 8k 维度 |
| **OpenAI** | `text-embedding-3-small` | ≈0.0001（USD） | 海外但便宜 |

**推荐**：`embedding-2`（智谱，中文 finetune 场景更准）。

---

## 5. 完整 avc.toml 配置示例（国产组合）

```toml
# ── LLM：主用 DeepSeek（便宜），备选 minimax（高质量）────────────
[provider.llm.deepseek]
api_key = "sk-..."
model = "deepseek-chat"
base_url = "https://api.deepseek.com/v1"

[provider.llm.minimax]
api_key = "sk-..."
model = "minimax-大模型"
base_url = "https://api.minimax.com/v1"

[provider.llm.qwen]
api_key = "sk-..."
model = "qwen-plus"
base_url = "https://dashscope.aliyuncs.com/compatible-mode/v1"

# ── Embed（中文 finetune drift）────────────────────────────────
[provider.embed.zhipu]
api_key = "sk-..."
model = "embedding-2"
base_url = "https://open.bigmodel.cn/api/paas/v4"

# ── Avatar（形象生成）─────────────────────────────────────────
[provider.avatar.jimeng]
api_key = "sk-..."
model = "jimeng-3.0"
base_url = "https://ark.cn-beijing.volces.com/api/v3"

# 可选：用 Qwen 视觉（OpenAI 兼容 image endpoint）做静态图
[provider.avatar.dalle_openai_compat]
api_key = "sk-..."
model = "qwen-image"
base_url = "https://dashscope.aliyuncs.com/api/v1"

# ── Voice（语音合成 / clone）─────────────────────────────────
# 用字节豆包 TTS（火山方舟）
[provider.voice.doubao_tts]
api_key = "sk-..."
model = "speech-01"
base_url = "https://ark.cn-beijing.volces.com/api/v3"

# 可选：MiniMax TTS（HTTP 兼容 OpenAI /audio/speech）
[provider.voice.minimax_tts]
api_key = "sk-..."
model = "speech-01-turbo"
base_url = "https://api.minimax.chat/v1"

# ── Video：可灵（vendor CLI）─────────────────────────────────
[provider.video.kling]
binary = "/opt/avc/vendor/kling-video.sh"

# 备选：即梦
[provider.video.jimeng]
binary = "/opt/avc/vendor/jimeng-video.sh"

# ── Shell NL 入口（推荐用便宜的 DeepSeek）─────────────────
[shell]
nl_model = "deepseek"
max_plan_steps = 8
temperature = 0.0
```

---

## 6. 多 Provider 备用策略

AVCore 5 维度解耦的好处是**多 Provider 并存**。常见 3 种模式：

### 6.1 模式 A：每个维度主用 + 备选

```toml
# LLM 主用便宜，备选高质量
[provider.llm.main]    # 平时用
[provider.llm.fallback] # main 挂了手动切

# 切换方式（CLI）
avc config get provider.llm.<name>.model   # 看当前
# avc.toml 里把 [shell] nl_model 改 fallback
```

### 6.2 模式 B：每个维度多 Provider + 程序自动 failover

v1 **不内置**自动 failover——需要用户在代码层实现（或在 vendor CLI 包装脚本里加 retry 逻辑）。

vendor CLI 包装脚本（`/opt/avc/vendor/kling-video.sh`）：
```bash
#!/bin/bash
# 先试 Kling 真 API，5xx 时 fallback 到即梦
case "$1 $2" in
  "submit --script")
    RESP=$(curl -fsS -X POST https://api.klingai.com/v1/videos/text2video ...)
    if [ $? -ne 0 ]; then
      RESP=$(curl -fsS -X POST https://ark.cn-beijing.volces.com/api/v3/videos ...)
    fi
    echo "$RESP"
    ;;
  ...
esac
```

### 6.3 模式 C：按任务分 Provider

`persona_versions.manifest_json.render_options` 可指定 persona 级别的 provider：

```toml
# 默认 OpenAI
# 但 Yu 这个角色想用国产 → 在 manifest 里覆盖
[persona_versions.yu.v2.manifest]
render_options = '''
{
  "voice_provider": "voice.doubao_tts",
  "avatar_provider": "avatar.jimeng",
  "video_provider": "video.kling"
}
'''
```

v1 通过 `avc persona set-render` CLI 设：
```bash
avc persona set-render yu --version 2 \
  --render-config '{"voice_provider":"voice.doubao_tts","avatar_provider":"avatar.jimeng","video_provider":"video.kling"}'
```

---

## 7. 常见组合推荐

### 7.1 预算紧 / 起步

```toml
# 4 维度全用 DeepSeek 体系（最便宜）
[provider.llm.deepseek]      # deepseek-chat, 0.14元/1k
[provider.embed.zhipu]       # embedding-2
[provider.avatar.jimeng]     # 即梦 3.0（按次）
[provider.video.kling]        # 可灵（按次）
# voice 用 MiniMax speech-01，0.5元/万字
```

**预计成本**：单个 60s 出片 ≈ 0.5-2 元。

### 7.2 质量优先 / 个人 IP

```toml
# 全 minimax 体系（质量最好）
[provider.llm.minimax]       # minimax-大模型
[provider.embed.zhipu]
[provider.avatar.jimeng]     # 即梦 3.0 pro
[provider.voice.minimax_tts]  # MiniMax speech-01-turbo
[provider.video.kling]        # 可灵 1.6 pro
```

**预计成本**：单个 60s 出片 ≈ 2-5 元。

### 7.3 中文长文本 / 知识库

```toml
[provider.llm.moonshot]      # kimi-k2，长上下文
[provider.embed.zhipu]       # 智谱 embedding-2
[provider.voice.doubao_tts]
[provider.video.jimeng]
```

### 7.4 极简 / 一家搞定

如果想 1 个厂商搞定所有维度：
- **字节火山方舟**（豆包）= LLM + Embed + Voice + Avatar（生成）+ Video（即梦 3.0）
- **阿里百炼**（通义）= LLM + Embed + Voice（CosyVoice）+ Avatar（Qwen-Image）
- **腾讯混元** = LLM + Embed + Voice + 智影

---

## 8. 数据合规与备案

### 8.1 跨境数据出境

按 2024-01 起的《规范和促进数据跨境流动规定》：

| 数据类型 | 是否需要申报 |
|---|---|
| 通过 API 调用境外 LLM（OpenAI/Anthropic）| 需自行评估；多数小批量走"免申报"门槛 |
| 训练数据 / persona_samples | 走"重要数据"目录需申报 |
| 渲染产物（mp4 / 图片 / 音频）| 一般不在申报范围 |
| 用户 prompt / 输出内容 | 看是否含个人信息 / 重要数据 |

**实操建议**：
- 用国产 LLM 跑渲染脚本生成、语料处理 → 不出境
- 写好的 prompt 模板 / persona 描述 → 落本地 DB，不出境
- 真的需要海外模型 → 走"数据出境安全评估"或用国产替代

### 8.2 备案（生成式 AI 服务）

按《生成式人工智能服务管理暂行办法》：
- 对公众提供生成式 AI 服务 → 需要算法备案
- **个人 / 内部使用 / 自用** → 多数情况下不需要
- 商业化部署 → 强烈建议咨询合规

### 8.3 API 调用注意事项

- **不要把 API key commit 到 git**（即使 private repo）
- 推荐用 systemd `EnvironmentFile=` 注入 key，不进 avc.toml：
  ```ini
  # /etc/avc/secrets.env
  AVC_LLM_DEEPSEEK_API_KEY=sk-...
  ```
  ```ini
  # /etc/systemd/system/avc.service
  EnvironmentFile=/etc/avc/secrets.env
  ```
- 生产环境：`avc.toml` 只放 base_url / model，key 走环境变量

### 8.4 私有化部署

| Provider | 私有化支持 |
|---|---|
| Qwen | ✅ Ollama / vLLM / DashScope 企业版 |
| GLM | ❌ 仅 SaaS |
| DeepSeek | ❌ 仅 SaaS（开源权重可自部署）|
| 文心 | ✅ 百度智能云私有化 |
| 智谱 | ✅ 私有化版本（贵）|

**v1 不直接支持本地 LLM**（OpenAI 兼容 endpoint 需配——`vllm` 启 OpenAI 兼容 server 后配 `base_url = "http://127.0.0.1:8000/v1"` 即可）。

---

## 9. 故障排查（国产特有问题）

| 现象 | 排查 |
|---|---|
| DeepSeek `ProviderUpstream` | 看 error_msg；DeepSeek 限速敏感，1 分钟内连续发太多会 429 |
| 通义 DashScope `429` | 阿里 QPS 限制严，每秒 5-10 个；减少 daemon `ping_interval_s` |
| 智谱 `TokenAuth` 401 | API key 区分 `glm-4` 和 `embedding-2`，不要混用 |
| 即梦（火山方舟）`ProviderUpstream` 5xx | 火山方舟偶发 503，等；或开 2 个 key 轮换 |
| 可灵 vendor CLI 报"missing binary" | `binary` 路径不对；用 `which` 验；vendor CLI 必须 `chmod +x` |
| 视频出片"placeholder" | 配了 vendor CLI 但仍返 mock → binary 错或 `task_id` 解析失败，看 `avc daemon logs` |
| 国产 LLM 中文偶尔输出英文 | prompt 加"请用中文回答"；或调低 `temperature` 到 0 |

---

## 10. 相关文档

- `docs/providers.md` — 5 维度协议 + OpenAI 兼容配置（协议层）
- `docs/user-guide.md` §4 — 第一次配置 provider（用户视角）
- `docs/operations.md` — 部署 / 备份 / systemd（运维视角）
- `examples/avc.toml.template` — 完整可复制模板
- `examples/vendor-cli/*.sh` — 4 个 mock 脚本（替换 kling / 即梦为真 vendor 即可）

**速查链接**：
- minimax 平台：https://platform.minimax.com
- DeepSeek 平台：https://platform.deepseek.com
- 智谱 BigModel：https://open.bigmodel.cn
- 阿里百炼 DashScope：https://dashscope.aliyuncs.com
- 字节火山方舟：https://www.volcengine.com/product/ark
- 月之暗面 Moonshot：https://platform.moonshot.cn
- 可灵 Kling：https://klingai.com
- 智谱 CogVideoX 开源：https://github.com/THUDM/CogVideoX
