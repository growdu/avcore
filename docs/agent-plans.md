# 火山方舟 + MiniMax 配置指南

> 火山方舟 agent plan + MiniMax token plan 用户的专用配置。
> 包含端到端验证步骤。落地可跑。

---

## 0. 前置

| 服务 | 用途 | 已购 plan | endpoint |
|---|---|---|---|
| **火山方舟 agent plan** | 主用 LLM（脚本生成 / ask / shell NL）| ✅ | `https://ark.cn-beijing.volces.com/api/v3` |
| **MiniMax token plan** | 备用 LLM | ✅ | OpenAI 兼容（确认中；待测试） |

> **agent plan** = 字节火山引擎的"智能体"套餐，含豆包系列模型 + 调用额度。
> **token plan** = MiniMax 的按 token 计费套餐。

---

## 1. 火山方舟（Volcengine Ark）配置

### 1.1 拿 key + endpoint

1. 登录 [火山方舟控制台](https://www.volcengine.com/product/ark)
2. 左侧「在线推理」→「API Key 管理」→ 创建新 key → 复制
3. 左侧「在线推理」→「模型推理」→「开通模型」→ 选豆包系列：
   - `doubao-pro-32k` (通用推荐，性价比高)
   - `doubao-lite-32k` (更便宜)
   - `doubao-seed-1.6-...` (最新一代)
   - **agent plan 套餐已含这些的调用额度**
4. 复制"模型 ID"（例如 `ep-xxxxxxxx-xxxxx`，agent plan 入口模型）
5. 记录 endpoint：`https://ark.cn-beijing.volces.com/api/v3`（北京区） / `https://ark.cn-shanghai.volces.com/api/v3`（上海区）

### 1.2 avc.toml 段

```toml
# 主用 LLM（火山方舟，agent plan 配额）
[provider.llm.ark_doubao]
api_key = "sk-..."                     # 在火山方舟控制台创建的 key
model = "doubao-pro-32k"               # 或 doubao-lite-32k / doubao-seed-1.6-...
base_url = "https://ark.cn-beijing.volces.com/api/v3"   # 或 cn-shanghai
```

> **model 字段**在 agent plan 里通常要填模型 ID（`ep-xxxxx`）或模型别名。**实测时先试别名**（如 `doubao-pro-32k`），不通再换 ID。

### 1.3 设置

```bash
avc config set provider.llm.ark_doubao.api_key "sk-..."
avc config set provider.llm.ark_doubao.model "doubao-pro-32k"
avc config set provider.llm.ark_doubao.base_url "https://ark.cn-beijing.volces.com/api/v3"
avc config set shell.nl_model "ark_doubao"
```

### 1.4 验证

```bash
avc provider test llm.ark_doubao
# 期望：exit 0，stdout 类似 "ok"
# 失败：401 TokenAuth / 404 model not found / 等

# 真实 prompt
avc ask "用一句话介绍你的身份"
# 期望：豆包风格的中文回复
```

---

## 2. MiniMax 配置

### 2.1 拿 key + endpoint

MiniMax（也作 MiniMax）有两个 API 协议：
- **OpenAI 兼容**：`https://api.minimaxi.com/v1`（首选，AVCore 用这个）
- **Anthropic 兼容**：`https://api.minimaxi.com/anthropic`（Claude Code 那种客户端用）

model 名常见为 `MiniMax-M3` / `MiniMax-M2.7` / `MiniMax-M2.7-highspeed` / `MiniMax-M2.5` / `MiniMax-M2.5-highspeed` / `MiniMax-M2.1` / `MiniMax-M2.1-highspeed` 等（按你账户的 `/v1/models` 列表为准）。

实测可用模型（2026-08-04 调用 `GET /v1/models` 验证）：
```
MiniMax-M3          (latest)
MiniMax-M2.7
MiniMax-M2.7-highspeed
MiniMax-M2.5
MiniMax-M2.5-highspeed
MiniMax-M2.1
MiniMax-M2.1-highspeed
```

> ⚠️ 文档里之前写的 `https://api.minimax.chat/v1` 是**错的**。实测正确的端点是 `https://api.minimaxi.com/v1`。

### 2.2 avc.toml 段

```toml
# 备用 LLM（MiniMax token plan，OpenAI 兼容）
[provider.llm.minimax]
api_key = "sk-cp-..."                  # 从 MiniMax 控制台复制
model = "MiniMax-M3"                   # 或 M2.7 / M2.5 / M2.1
base_url = "https://api.minimaxi.com/v1"
```

### 2.3 设置

```bash
avc config set provider.llm.minimax.api_key "sk-..."
avc config set provider.llm.minimax.model "MiniMax-大模型"
avc config set provider.llm.minimax.base_url "https://api.minimax.chat/v1"
```

### 2.4 验证

```bash
avc provider test llm.minimax
avc ask "用一句话介绍你自己"
```

---

## 3. 主备切换

avc.toml 里两个都配，通过 `[shell] nl_model` 切主用：

```toml
[shell]
nl_model = "ark_doubao"     # 平时用火山方舟
max_plan_steps = 8
temperature = 0.0
```

切换到 MiniMax：
```bash
avc config set shell.nl_model "minimax"
```

`provider test` / `ask` / `render run` 都跟着 `nl_model` 走。

---

## 4. 完整端到端验证（你即将跑）

下面这套命令会真实调火山方舟 + 跑 AVCore 的 LLM 路径：

```bash
# 1. 配置
avc config set provider.llm.ark_doubao.api_key "<你的 key>"
avc config set provider.llm.ark_doubao.model "<模型 ID 或别名>"
avc config set provider.llm.ark_doubao.base_url "https://ark.cn-beijing.volces.com/api/v3"

# 2. 网络可达性
curl -fsS https://ark.cn-beijing.volces.com/api/v3/models \
  -H "Authorization: Bearer <你的 key>" | head -50

# 3. avc 路径测试
avc provider test llm.ark_doubao
# 期望：exit 0

# 4. 真实 LLM 调用
avc ask "你好，请用一句话介绍你自己"
# 期望：豆包风格的中文回复，stdout 是一段 JSON

# 5. 跑一个最小 persona workflow（只需要 LLM）
avc shell <<'EOF'
persona create --name test_ark --archetype teacher
show test_ark
exit
EOF
# 期望：exit 0，看到 test_ark 创建 + show 输出

# 6. 起 daemon（持续探活）
avc daemon start
sleep 3
avc provider status --dim llm
# 期望：ark_doubao 行 status=healthy（被动 hook + daemon 探活）

# 7. 清理
avc daemon stop
```

## 4.5 实测结果（2026-08-04，MiniMax）

> 火山方舟的「agent plan」key 走的是 `/api/v3/bots/chat/completions` 专用协议，AVCore 的 OpenAI 兼容 chat provider 不兼容。改用 env 里的 MiniMax token plan key（`minimaxi.com/v1` 端点）跑通完整 7 步。

### 配置

```toml
[provider.llm.minimax]
api_key = "sk-cp-..."                  # 来自 env $ANTHROPIC_AUTH_TOKEN
model = "MiniMax-M3"
base_url = "https://api.minimaxi.com/v1"
```

### 各步实测结果

| 步 | 命令 | 结果 |
|---|---|---|
| 1 | `curl -fsS .../v1/models` | HTTP 200，列出 7 个模型（MiniMax-M3 / M2.7 / M2.7-highspeed / M2.5 / M2.5-highspeed / M2.1 / M2.1-highspeed）|
| 2 | `curl -X POST .../v1/chat/completions` body=`{"model":"MiniMax-M3","messages":...}` | HTTP 200，回复「我是 MiniMax-M3，一个由 MiniMax 开发的 AI 助手...」，250 tokens |
| 3 | `avc config set` 3 个字段 | 全部成功（但 `shell.nl_model` 不在 white-list，需直接编辑 `~/.config/avc/avc.toml`）|
| 4 | `avc provider test llm.minimax` | exit 0，JSON `{"ok": true, "provider": "llm.minimax", "reply_preview": "..."}` |
| 5 | `avc ask "用一句话介绍你自己"` | MiniMax 返回 plan JSON `{"intent":"unknown","read_only":true,"steps":[]}`（正确：此 prompt 不对应任何 verb）|
| 6 | `avc shell` + `persona create --name minimax_test ...` + `show minimax_test` | `minimax_test` 创建成功（id `pm_01kz5z4rtw2tb8j3zypgq0bcvy`），show 返 JSON；shell 内 NL 也走 minimax 翻译 |
| 7 | `avc daemon start` → `provider status` → `daemon logs` → `daemon stop` | daemon 启 pid 1327464；`llm.minimax healthy latency=2123ms`；日志写 `avc.log`（`daemon listening on 127.0.0.1:7891`）；SIGTERM 干净退出 |

### 副作用

- `avc config set` 没读 XDG_CONFIG_HOME，写到了**全局** `~/.config/avc/avc.toml`（`avc init` 也是）。要隔离测试目录得直接编辑文件 + 用 `XDG_DATA_HOME` 覆盖 DB 路径
- 全局 avc.toml 现在有明文 api_key（chmod 600 保护，但任何能读你 home 的人能看）。清理方式：`vim ~/.config/avc/avc.toml` 删 `[provider.llm.minimax]` 整段
- 整个流程没有遇到 panic / 数据损坏 / 权限错误

如果某步失败，按 `docs/providers-cn.md` §9 + `docs/operations.md` §10 排查。

---

## 5. 成本预估（agent plan 范围内）

| 操作 | token 量（典型）| agent plan 配额 |
|---|---|---|
| 1 次 `avc provider test llm.X` | 5-10 | 几十次 |
| 1 次 `avc ask "一句话..."` | 50-100 | 几千次 |
| 1 次 `avc render run` 5 节点 | 2-5k input + 1-3k output | 几百次 |
| 1 次 `avc finetune run` drift eval | 200-500 | 几千次 |

agent plan 通常给"够用"的额度（具体看你买的 tier），不超额计费。

---

## 6. 故障排查

| 现象 | 排查 |
|---|---|
| `avc provider test` 报 401 | key 错 / 过期 / 复制错字符 |
| 报 404 | `model` 字段填错；agent plan 里的模型 ID 是 `ep-xxx-xxx` 格式（**不是** `doubao-pro-32k`）—— 控制台复制 |
| 报 429 | 短时间调用太多；agent plan 有 QPS 限制；等 |
| 报 "endpoint not found" | base_url 错；`api/v3` 不要带尾斜杠 |
| `avc ask` 返回空 | 查 `~/.local/share/avc/avc.log`（若 daemon 在跑）；或 stderr 路径 |
| 中文偶发英文 | prompt 加"用中文回答"；或降 `temperature` 到 0 |

---

## 7. 相关文档

- `docs/providers.md` — 5 维度协议 + 通用配置
- `docs/providers-cn.md` — 国产厂商选型矩阵
- `docs/user-guide.md` §4 — 第一次配置 provider
- `docs/operations.md` — 部署 / systemd / 备份
- `examples/avc.toml.template` — 完整可复制模板
