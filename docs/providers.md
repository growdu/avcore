# AI Provider 配置与大模型交互

> AVCore 怎么配置 5 个 provider 维度（llm / embed / avatar / voice / video），与各家大模型怎么对话。
> 配合 `examples/avc.toml.template`（完整可复制模板）使用。

---

## 1. 5 维度模型

AVCore 不直接绑死 OpenAI。配置 5 个独立的"provider 维度"，每个维度可以装任意 OpenAI 兼容端点 / vendor CLI：

| 维度 | 用途 | 调用场景 |
|---|---|---|
| `llm` | 文本生成 | `avc ask` / `avc shell` NL 解析 / render 脚本生成（`script_gen` 节点）/ finetune drift seed 文本生成 |
| `embed` | 文本向量化 | finetune drift eval（face/voice/style 三维 seed → vector → cosine）|
| `avatar` | 形象生成 + SFT | persona `create()`（OpenAI `/v1/images/generations`）/ persona `finetune()` avatar scope（vendor CLI）|
| `voice` | 语音合成 + clone/SFT | render `tts` 节点（OpenAI `/audio/speech`）/ persona `finetune()` voice scope（vendor CLI）|
| `video` | 图生视频 | render `i2v` 节点（vendor CLI 三段式 `submit / status / fetch`）|

5 维度解耦的好处：
- LLM 用 OpenAI、avatar 用 DALL-E、voice 用 ElevenLabs、video 用 kling——**完全独立**
- 换一家厂商只改一个 `[provider.<dim>.<name>]` 段
- API key 隔离（voice 厂商挂了不影响 llm）

---

## 2. 配置方式：4 种来源

按优先级从高到低：

| 优先级 | 来源 | 适用 |
|---|---|---|
| 1（最高）| 环境变量 `AVC_<KEY>`（TODO：v1 未直接实现，可走 systemd `Environment=`）| 容器 / K8s secret |
| 2 | `avc.toml` 字段（`api_key = "..."`）| 本地配置文件 |
| 3 | `avc config set provider.<dim>.<name>.<field> <value>` 改写 avc.toml | 一次性设置 |
| 4（最低）| `avc.toml.template` 里的占位 | 起步模板 |

**`Config::load` 路径**：`~/.config/avc/avc.toml`（Linux/macOS 遵循 XDG；Windows 用 `%APPDATA%`）。

**权限要求**：`avc.toml` 必须 `0600`（含 API key）。`Config::save` 自动设 0600；手动复制后用 `chmod 600 ~/.config/avc/avc.toml`。

---

## 3. 5 维度详解

### 3.1 `llm` — 文本大模型

**协议**：OpenAI 兼容 `POST /chat/completions`

**最小配置**：
```toml
[provider.llm.openai]
api_key = "sk-..."
model = "gpt-4o-mini"                    # 或 gpt-4o / o1 / o3 / qwen-plus / claude-sonnet-4-5 / ...
base_url = "https://api.openai.com/v1"    # 可省略，默认 OpenAI 官方
```

**支持的端点**：任何暴露 OpenAI 兼容 chat completion 的服务：
- OpenAI 官方
- Azure OpenAI
- DeepSeek
- 阿里 DashScope（`base_url = "https://dashscope.aliyuncs.com/compatible-mode/v1"`）
- 智谱 GLM（Zhipu）
- Ollama / vLLM 本地
- Anthropic via OpenAI-compat proxy

**特殊场景**：
- Anthropic 直连（不是 OpenAI 协议）：需要 proxy，或者用 `--model claude-...` 走 proxy 的 `base_url`
- 自定义 header：```toml
  extra_headers = { "anthropic-version" = "2023-06-01" }
  ```

**代码路径**：`src/provider/real.rs::OpenAiCompatLlmProvider::chat` → `POST {base_url}/chat/completions` body `{model, messages, temperature}`。

**失败模式**：
- 401 → `AvcError::TokenAuth`（exit 5）
- 429 → `AvcError::RateLimited`（exit 10），会被 passive hook 记录到 `provider_rate_limit` 表
- 5xx / 网络 → `AvcError::ProviderUpstream`（exit 11）

### 3.2 `embed` — 文本向量化

**协议**：OpenAI 兼容 `POST /embeddings`

**最小配置**：
```toml
[provider.embed.openai]
api_key = "sk-..."
model = "text-embedding-3-small"          # 或 text-embedding-3-large / bge-...
base_url = "https://api.openai.com/v1"
```

**当前用途**：`finetune run` 时对 base / target 各自生成 drift seed text 的 embedding，算 cosine。v1 用同一文本模型 + 不同 seed 切片（persona/face/voice/style 各一个），**不是真 CLIP / Resemblyzer**——升级需要独立项目（spec §8）。

**未配置的降级**：`finetune run --embed <name>` 必填；不传会报错。CLI 显式要求。

### 3.3 `avatar` — 形象生成 / SFT

**协议**：OpenAI 兼容 `POST /v1/images/generations`（HTTP）+ 可选 vendor CLI（avatar SFT/clone）

**`create()`（render 中用 + persona create 中用）**：
```toml
[provider.avatar.openai]
api_key = "sk-..."
model = "dall-e-3"
base_url = "https://api.openai.com/v1"    # 或 wanx / cogview 等的 OpenAI 兼容端点
```

返回 base64 PNG BLOB，落 `persona_versions.avatar_primary`。

**`finetune()`（仅在 `avc finetune start --scope avatar` 走）**：
```toml
[provider.avatar.kling]
binary = "/usr/local/bin/kling-cli"        # vendor CLI；submit/status/fetch 三段式
```

`avc.toml` 走 **vendor CLI 协议**（spec §4.1）：
```text
binary finetune submit --ref-image <path...>  →  stdout: task_id=...
binary finetune status --task-id <id>         →  stdout: status=done|pending|failed
binary finetune fetch --task-id <id> --out <p> → 写真 PNG
```

未配 `binary` → `AvcError::Internal("requires a vendor CLI binary")`。**Phase 1 fallback 行为已废弃**（不会再返回占位 PNG）；必须配真 vendor。

详见 `examples/vendor-cli/kling-avatar-fin.sh` 模板（KV-flavor stdout 协议）。

### 3.4 `voice` — 语音合成 / clone / SFT

**`synth()`（render `tts` 节点）**：
```toml
[provider.voice.openai]
api_key = "sk-..."
model = "tts-1"                          # OpenAI 官方 tts-1 / tts-1-hd
base_url = "https://api.openai.com/v1"
```

返回 WAV BLOB，落 `job_steps.outputs_json` + artifacts 表。

**`finetune()` / `clone()`（仅 `avc finetune start --scope voice` 走）**：
```toml
[provider.voice.elevenlabs]
binary = "/usr/local/bin/elevenlabs-cli"  # vendor CLI
```

`voice clone()` 在 `avc sample add --kind audio` 后由 finetune run 触发；调用 vendor CLI 三段式，写真 WAV。

**当前 v1 状态**：根据 T9 实现审计，`OpenAiCompatVoiceProvider::clone()` 仍是 **placeholder 路径**——返回 `RIFF....CLONE_PLACEHOLDER` + 假 ID + 零向量。要真 voice clone 必须配 vendor CLI binary。

### 3.5 `video` — 图生视频

**协议**：vendor CLI 三段式 `submit / status / fetch`（**非 HTTP**）

**最小配置**：
```toml
[provider.video.kling]
binary = "/usr/local/bin/kling-cli"
```

未配 `binary` → `AvcError::Internal("requires a vendor CLI binary")`（Phase 1 的占位 mp4 行为已废弃）。

**协议**（spec §2.3）：
```text
binary submit  --script <text> --avatar <png> --voice <wav>  →  stdout: task_id=...
binary status  --task-id <id>                              →  stdout: status=done|pending|failed (500ms poll, 5min timeout)
binary fetch   --task-id <id> --out <mp4>                   → 写真 mp4
```

详见 `examples/vendor-cli/kling-video.sh` 模板。

---

## 4. 一次性配置命令

```bash
# 查当前配置
avc config get provider.llm.openai.api_key
avc config get provider.video.kling.binary

# 改单个字段
avc config set provider.llm.openai.api_key "sk-..."
avc config set provider.video.kling.binary "/usr/local/bin/kling-cli"

# 查所有 providers
avc provider list
```

`avc config get` 路径格式：`provider.<dim>.<name>.<field>`，仅支持 `api_key / model / endpoint` 三个字段；改 `[export.s3]` 等其他段需直接编辑 `avc.toml`。

---

## 5. 测试与诊断

```bash
# 测单个 provider 是否能通
avc provider test llm.openai          # exit 0 = 通；非 0 = 错（含 stderr 详情）
avc provider test embed.openai
avc provider test avatar.openai
avc provider test voice.openai
avc provider test video.kling

# 综合诊断
avc doctor                            # 检查 db 路径 / config 路径 / 路径权限
```

**passive hooks**（T10/T11）：每次 provider 调用出错时，daemon 都会把 health/rate-limit 状态写库（`provider_health` / `provider_rate_limit` 表），下次用 `avc provider status` 查。

---

## 6. 端到端数据流：一次 `avc render run` 怎么走

```
avc render run --persona yu --version 1 --topic "InnoDB Buffer Pool"
    │
    ├─ 1. script_gen 节点（llm）
    │     → provider.llm.<configured> → POST /chat/completions
    │     → 输出：script (3 个 scene + 时长)
    │
    ├─ 2. tts 节点（voice）
    │     → provider.voice.<configured> → POST /audio/speech
    │     → 输出：3 个 WAV BLOB → 落 job_steps / artifacts
    │
    ├─ 3. img_gen 节点（avatar）
    │     → provider.avatar.<configured> → POST /v1/images/generations
    │     → 输出：3 个 PNG BLOB → 落 job_steps / artifacts
    │
    ├─ 4. i2v 节点（video）
    │     → provider.video.<configured> → vendor CLI submit/status/fetch
    │     → 输出：3 个 mp4 BLOB → 落 artifacts
    │
    └─ 5. compose 节点（本地 FFmpeg）
          → 把 3 个 mp4 拼接为最终片
          → 落 artifacts 名为 <job_id>.mp4
```

每个节点的：
- 输入：上一步 outputs（`job_steps.outputs_json`）
- 输出：写 `artifacts` 表（kind=script/audio/image/video）
- 状态：写 `job_steps` 表（status, duration_ms, error_json）
- 错误：节点失败不阻断 job（除非 `gate` 节点）；error 落 `jobs.error_json`

详见 `docs/modules/pipeline.md`。

---

## 7. 完整 avc.toml 示例

```toml
# ── LLM（OpenAI 兼容）────────────────────────────────────
[provider.llm.openai]
api_key = "sk-..."
model = "gpt-4o-mini"
base_url = "https://api.openai.com/v1"

# 备选：阿里 DashScope
# [provider.llm.dashscope]
# api_key = "sk-..."
# model = "qwen-plus"
# base_url = "https://dashscope.aliyuncs.com/compatible-mode/v1"

# 备选：Ollama 本地
# [provider.llm.ollama]
# api_key = "ollama"        # Ollama 不校验；填占位
# model = "llama3.1:8b"
# base_url = "http://127.0.0.1:11434/v1"

# ── Embed ────────────────────────────────────────────────
[provider.embed.openai]
api_key = "sk-..."
model = "text-embedding-3-small"
base_url = "https://api.openai.com/v1"

# ── Avatar ──────────────────────────────────────────────
[provider.avatar.openai]
api_key = "sk-..."
model = "dall-e-3"
base_url = "https://api.openai.com/v1"

# 配 kling 头像 SFT（finetune start --scope avatar）
[provider.avatar.kling]
binary = "/path/to/kling-avatar-cli"

# ── Voice ───────────────────────────────────────────────
[provider.voice.openai]
api_key = "sk-..."
model = "tts-1"
base_url = "https://api.openai.com/v1"

# 配 ElevenLabs voice clone
[provider.voice.elevenlabs]
binary = "/path/to/elevenlabs-cli"

# ── Video ───────────────────────────────────────────────
[provider.video.kling]
binary = "/path/to/kling-video-cli"
```

---

## 8. 故障排查

| 现象 | 排查 |
|---|---|
| `avc provider test llm.openai` 报 `TokenAuth` | API key 错 / 过期 / 复制错字符 / 用了别环境的 key |
| 报 `ProviderUpstream` 5xx | vendor 服务挂了；看 `avc provider status` 拿 error_msg |
| 报 `ProviderTimeout` | 5s 探测超时；vendor 服务慢 / 网络问题；增大 `avc.toml [daemon] ping_interval_s` 减少探测频率 |
| 报 `rate_limited` | vendor 限速；`avc provider rate-limit` 看 `until_ts` 何时过期 |
| `avc doctor` 报路径错 | 看 stderr：data_dir / config_dir 路径权限 |
| `cargo install` 后 `avc` not found | `~/.cargo/bin` 不在 PATH：`export PATH=$HOME/.cargo/bin:$PATH` |
| vendor CLI 不工作 | `avc daemon logs` 看 tracing；vendor CLI 必须 stdout `key=value` 格式 |

---

## 9. 进阶：vendor CLI 替换

`avc.toml` 字段 `binary` 指定的可执行程序可以是：
- 编译好的二进制
- 包装脚本（`#!/bin/bash` + curl + jq + 真 vendor API）
- Mock 模板（`examples/vendor-cli/*.sh` —— 开发/CI 无 key 时用）

最简 mock 模板（kling-video.sh 风格）：
```bash
#!/bin/bash
case "$1 $2" in
  "submit --script")
    echo "task_id=mock-$(date +%s)"
    ;;
  "status --task-id")
    echo "status=done"
    ;;
  "fetch --task-id")
    # 写真 placeholder mp4（10 字节 dummy）
    printf 'MP4_PLACEHOLDER' > "$4"
    ;;
esac
```

把这种脚本放到 `binary = "/path/to/mock-video.sh"` 就能在 CI 跑通 `render run` 端到端，无需真 vendor key。生产换真 vendor 时只改 `binary` 指向新路径。

---

## 10. 相关文档

- `docs/cli.md` — 完整 CLI 命令参考（含 `avc provider` 所有子命令）
- `docs/storage.md` — schema（`provider_health` / `provider_rate_limit` 表结构）
- `docs/modules/pipeline.md` — 5 节点 DAG 详解
- `docs/modules/persona-iteration.md` — persona 怎么迭代（refine / finetune）
- `docs/operations.md` — 部署 / 备份 / 升级 / systemd
- `examples/avc.toml.template` — 完整可复制配置
- `examples/vendor-cli/*.sh` — 4 个 vendor CLI mock 模板
- `docs/superpowers/specs/2026-08-03-provider-daemon-design.md` §3 — 5 维度的原始设计
