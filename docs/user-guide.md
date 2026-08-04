# AVCore 用户手册

> 你是一个普通用户，想用 AVCore 跑通一个 AI 角色 + 出片 workflow。本手册从"准备材料"开始，到"完整出片"结束。
> 适用：任何 Linux / macOS 终端用户。

---

## 1. AVCore 是什么

AVCore 是个 Rust 单二进制 CLI，它让你：

1. **创建一个 AI 角色**（persona）—— 例如"Yu / 数据库内核专家"
2. **多版本迭代**（v1 → v2 → v3...）—— refine 改 prompt，finetune 走 vendor SFT
3. **让某个版本出片**—— render 5 节点 DAG 走 LLM 脚本 → TTS → 图生成 → 图生视频 → 拼接

**一图流**：

```
        persona v1 ──▶ refine ──▶ v2 ──▶ finetune ──▶ v3 ──▶ 出片
            │                            │                   │
            ▼                            ▼                   ▼
      SQLite 1 行                  SQLite 1 行         artifacts (mp4 + 5 节点中间产物)
```

全部状态在一个 SQLite 文件 `~/.local/share/avc/avc.db` 里。

**适合的人**：跑 AI 角色但懒得写 pipeline 的人；想用一个命令串起 5 步出片的人；要管理多版本 persona 的人。

**不适合的人**：要本地跑 LLM 推理的人（AVCore **只调 API，不加载模型**）；要 Web UI 的人（v1 没有）；要多用户 SaaS 的人（v1 单租户）。

---

## 2. 准备什么

### 2.1 系统

- Linux / macOS（Windows 在 v1 范围外）
- Rust 1.78+（[rustup.rs](https://rustup.rs)）
- 4 GB 磁盘（数据 + 5 节点中间产物）
- **不需要 GPU**（全部推理走云端 API）

### 2.2 账号（你至少需要 1 个）

| 用途 | 推荐服务 | 价格举例 |
|---|---|---|
| 文本大模型（**必填**）| OpenAI `gpt-4o-mini` / 阿里 DashScope `qwen-plus` / Ollama 本地 | $0.15/M token |
| Embedding（finetune 时用，可选）| OpenAI `text-embedding-3-small` | $0.02/M token |
| 形象生成（avatar）| OpenAI `dall-e-3` / 阿里 wanx / kling | $0.04/图 |
| 语音合成（voice）| OpenAI `tts-1` / ElevenLabs | $15/M 字符 |
| 视频生成（video）| kling / Sora / Runway（vendor CLI）| 各家不同 |

**省成本起步**：只用 OpenAI 一家 = `llm.openai` + `embed.openai` + `avatar.openai` + `voice.openai` + `video.kling`（vendor CLI 走 mock 模板）。一个 OpenAI key 就能跑通 4/5 维度。

### 2.3 时间

- 安装：5 分钟
- 第一个 render：30 分钟（含配置）
- 完整 finetune 流程：1-2 小时（含 vendor SFT 调用）

---

## 3. 安装（5 分钟）

```bash
git clone https://github.com/avcore/avc.git
cd avc
cargo install --path . --locked          # → ~/.cargo/bin/avc
```

验证：
```bash
avc version                              # 应输出 "avc 0.3.x"
avc doctor                               # 检查 db/config 路径
```

> macOS：`cargo install` 出来的二进制在 `~/.cargo/bin/`，确保该路径在 `$PATH` 里。
> 装完没看到 `avc` 命令？`echo $PATH` 检查；`export PATH=$HOME/.cargo/bin:$PATH` 或加到 `~/.bashrc` / `~/.zshrc`。

---

## 4. 第一次配置（10 分钟）

### 4.1 初始化

```bash
avc init
```

这会建 SQLite 库 + 配置目录。

### 4.2 配置 Provider

**最快路径**（用 OpenAI 一个 key 跑通全 4 维度）：

```bash
avc config set provider.llm.openai.api_key "sk-..."
avc config set provider.llm.openai.model "gpt-4o-mini"
avc config set provider.llm.openai.base_url "https://api.openai.com/v1"

avc config set provider.embed.openai.api_key "sk-..."
avc config set provider.embed.openai.model "text-embedding-3-small"
avc config set provider.embed.openai.base_url "https://api.openai.com/v1"

avc config set provider.avatar.openai.api_key "sk-..."
avc config set provider.avatar.openai.model "dall-e-3"
avc config set provider.avatar.openai.base_url "https://api.openai.com/v1"

avc config set provider.voice.openai.api_key "sk-..."
avc config set provider.voice.openai.model "tts-1"
avc config set provider.voice.openai.base_url "https://api.openai.com/v1"
```

**视频维度**必需 vendor CLI（kling 等）；CI/无 key 时用 mock 模板：

```bash
# 拷 mock 模板
mkdir -p ~/.config/avc/bin
cp examples/vendor-cli/kling-video.sh ~/.config/avc/bin/
chmod +x ~/.config/avc/bin/kling-video.sh

avc config set provider.video.kling.binary "$HOME/.config/avc/bin/kling-video.sh"
```

**测试连通**：

```bash
avc provider test llm.openai            # 应输出 success
avc provider test embed.openai
avc provider test avatar.openai
avc provider test voice.openai
avc provider test video.kling
```

任一失败会有具体报错（401 / 429 / timeout）——查 `docs/providers.md` §8 排查。

### 4.3 改动查询

```bash
avc config get provider.llm.openai.api_key
avc provider list                        # 列出所有已配置的 provider
```

`avc config` 命令只能改 `provider.<dim>.<name>.{api_key,model,endpoint}` 三个字段；其他段（`[shell]` / `[safety]` / `[export.s3]` 等）需要直接编辑 `~/.config/avc/avc.toml`。

---

## 5. 核心概念（30 秒读懂）

| 概念 | 含义 | 类比 |
|---|---|---|
| **PersonaModel** | 角色的稳定身份（一行记录）| "数据库内核专家" |
| **PersonaVersion** | 同一角色的某个版本快照 | v1 的 prompt / v2 的 prompt |
| **Provider** | 5 维度（llm/embed/avatar/voice/video）之一的某个实例 | "openai" / "kling" |
| **Refine (iterate)** | 改 prompt / 知识绑定 / render config，**不建新版本** | 改 v1 的描述 |
| **Finetune** | 调 vendor SFT/clone API，**建新版本** | v1 → v2 走 avatar SFT |
| **Render** | 一次性 5 节点 DAG 出片 | 拿 v1 出 mp4 |
| **Job** | render 跑一次 = 1 个 job | 包含 5 个 job_steps + artifacts |
| **Artifact** | 节点产物（script/audio/image/video） | 落 avc.db BLOB |

记忆口诀：**Model = 身份，Version = 快照，Refine = 同版本改 prompt，Finetune = 同 id 多版本，Render = 一次性出片**。

---

## 6. 完整工作流（30 分钟跑通）

### 6.1 创建角色（一行命令）

```bash
avc persona create \
  --name yu \
  --archetype db_kernel_expert \
  --descriptor "你是一个 MySQL / PostgreSQL 内核专家，喜欢用代码示例回答问题" \
  --catchphrase "show me the source"
```

> 注：v1 的 `create` 只写 persona_descriptor / archetype 等元数据，**形象和声音资产**完全靠 finetune 阶段的 vendor SFT 来创建（或手动 `attach-avatar` / `attach-voice`）。详见 `docs/modules/persona-modeling.md`。

### 6.2 看一眼状态

```bash
avc persona list                # 列出所有 persona
avc persona show yu             # 详细 JSON
avc persona versions yu         # 列出所有版本（v1 = 初版）
avc persona current yu          # 当前指针版本
```

### 6.3 Refine：在同版本内改 prompt

不需要建新版本。直接改 v1 的字段：

```bash
avc persona set-traits yu --version 1 --traits '["严谨", "用 Rust 写示例"]'
avc persona set-catchphrase yu --version 1 --catchphrase "Always include the source code"
avc persona set-render yu --version 1 --render-config '{"voice_provider":"voice.openai","avatar_provider":"avatar.openai"}'
```

refine 都跑在事务里（`svc/iterate.rs::apply`），改 3 张表（`persona_versions.persona_descriptor_json` / `knowledge_binding_json` / `manifest_json`）——不写 metrics_json（那是 v2 的事）。

### 6.4 出片（第 1 次 render）

```bash
avc render run \
  --persona yu \
  --version 1 \
  --topic "InnoDB Buffer Pool 替换算法" \
  --duration 60 \
  --video-provider kling
```

这会跑 5 节点 DAG：
1. `script_gen`（llm）→ 一段脚本
2. `tts`（voice）→ 3 段 WAV
3. `img_gen`（avatar）→ 3 张 PNG
4. `i2v`（video）→ 3 段 mp4
5. `compose`（本地 FFmpeg）→ 1 个最终 mp4

**看进度**：

```bash
avc job list                    # 列出所有 job
avc job show <id>               # 看某个 job 的 5 个 step 状态
avc job wait <id>               # 阻塞，直到 done / failed
```

`job show` 的输出是 JSON，包括 5 个 job_steps 的 status / duration_ms / error_json。

**导出**：

```bash
avc job export <id> --out /tmp/yu-video/      # 拷所有 artifact 到目录
# 或 S3（需 avc.toml [export.s3].upload_cmd）
avc job export <id> --target s3://my-bucket/videos/2026/
```

### 6.5 Finetune：建新版本

`render` 已经能跑通 v1 了，下面加样本、做 finetune、得到 v2。

**加样本**：

```bash
# 加音频样本（用于 voice finetune）
avc sample add yu --kind audio --uri ./yu_voice_01.wav --text "我看了下 InnoDB 的源码"

# 加图片样本（用于 avatar finetune）
avc sample add yu --kind image --uri ./yu_face_01.png

# 加反馈样本（finetune 阶段会喂给 LLM 提炼）
avc sample add yu --feedback --kind text --text "上次出片声音不太对，应该更慢一点"
```

**启 finetune job**：

```bash
avc finetune start yu --base-version 1 --scope voice
# 输出: fj_xxx
```

**跑 finetune**（vendor SFT）：

```bash
avc finetune run fj_xxx --embed openai
# transcript:
#   → 调 voice vendor CLI 跑 voice clone
#   → 算 drift（与 base v1 比 voice cosine）
#   → 达标 → publish 到 v2；不达标 → rollback
```

**进度与降级**：

```bash
avc finetune list yu           # 列出所有 finetune job
avc finetune show fj_xxx       # drift 报告 + 状态
avc finetune report fj_xxx     # JSON drift 详情
```

**v2 就绪后**：

```bash
avc persona versions yu        # 现在 [1, 2]
avc persona current yu         # 可能仍指 1
avc persona current yu --set 2  # 切到 v2，旧版不删
```

下次 `render run --version 2` 用新的声纹 + 形象。

---

## 7. 交互模式

### 7.1 Shell 模式（交互式）

```bash
avc shell
```

```
avc> help
avc> persona list
avc> persona show yu
avc> render run --persona yu --version 1 --topic "..."
avc> exit
```

适合：探索式使用；不知道完整命令时。

Shell 内部用 `rustyline` 支持历史和补全。`avc shell --help` 看更多选项。

### 7.2 Ask 模式（自然语言）

```bash
avc ask "用 yu 角色出片，主题是 InnoDB Buffer Pool"
avc ask "列出所有 persona"
avc ask "把 yu 的 v2 切到当前"
```

工作原理：
1. `ask` 把自然语言发给 `provider.llm.<configured>`
2. LLM 返回一个 JSON `plan`（白名单原子的有限序列）
3. AVCore 执行 plan

**适合**：CI 脚本、cron 任务、自动化。

详细信息：`docs/shell.md` / `docs/api/README.md`。

---

## 8. 进阶技巧

### 8.1 多人协作 / 备份

```bash
# 单 SQLite 文件好备份
sqlite3 ~/.local/share/avc/avc.db ".backup '$(date +%F).db'"

# 还原
avc init                          # 重新建空库
sqlite3 ~/.local/share/avc/avc.db ".restore '2026-08-04.db'"
```

详细：`docs/operations.md` §5 / §6。

### 8.2 后台 daemon：观察 provider 状态

```bash
avc daemon start                  # fork 后台
sleep 5
avc provider status               # 看 5 维度的最新探活结果
avc provider rate-limit           # 看限速冷却状态
avc daemon logs                    # 看日志
avc daemon stop
```

daemon 每 60s 探测一次所有 provider（avc.toml `[daemon] ping_interval_s`）。配置错误 / token 失效能及时发现，不用等下次 render 才报错。

### 8.3 知识库（精调领域知识）

```bash
avc corpus create --name "innodb-docs" --source-path ./docs/
avc corpus search --name "innodb-docs" --query "Buffer Pool LRU"
```

`persona attach-knowledge` 把 corpus 接到 persona 上。详细：`docs/storage.md`。

### 8.4 切换 LLM 厂商

不用改代码，只改 avc.toml：

```toml
[provider.llm.dashscope]        # 阿里 DashScope（OpenAI 兼容）
api_key = "sk-..."
model = "qwen-plus"
base_url = "https://dashscope.aliyuncs.com/compatible-mode/v1"
```

然后 `avc shell.nl_model = "dashscope"` 即可用阿里模型做 ask / script_gen。

### 8.5 不换 vendor SFT 也能新建版本

不调 vendor SFT 的 finetune 也能建新版本：`avc finetune start yu --base-version 1 --scope persona`（不指定 --scope voice/avatar）。这相当于"复制 v1 改个 prompt 得到 v2"，文档化在 `docs/modules/persona-iteration.md`。

---

## 9. 常见错误速查

| 现象 | 含义 | 修法 |
|---|---|---|
| `TokenAuth` exit 5 | API key 错 / 过期 | 重新设 `avc config set provider.<dim>.<name>.api_key` |
| `RateLimited` exit 10 | 厂商限速 | 等到 `avc provider rate-limit` 显示 `until_ts` 过期；或换 key |
| `ProviderTimeout` | 5s 探活超时 | 看 vendor 服务是否正常；增大 `avc.toml [daemon] ping_interval_s` |
| `ProviderUpstream` 5xx | vendor 服务挂了 | 等；查 vendor status page |
| `persona not found` exit 3 | persona 名拼错 | `avc persona list` 看真实名字 |
| `persona version N not in pending/ready` | 用 finetune 还在 building 的版本去 render | 等 finetune 完成，或用 v1 |
| `provider.llm.openai not configured` | llm 没设 | 回到 §4.2 |
| `database is locked` | CLI 跟 daemon 同时密集写 | 错开跑；或检查僵尸进程 |
| `avc: command not found` | `~/.cargo/bin` 不在 PATH | `export PATH=$HOME/.cargo/bin:$PATH` |

详细：`docs/operations.md` §10。

---

## 10. 速查 — 常用命令

```bash
# 查
avc persona list
avc persona show <name>
avc persona versions <name>
avc persona current <name>
avc sample list <name>
avc finetune list <name>
avc job list
avc job show <id>
avc corpus list
avc provider list

# 改
avc config set <key> <value>
avc persona set-traits <name> --version <v> --traits <json>
avc persona set-catchphrase <name> --version <v> --catchphrase "..."
avc persona current <name> --set <v>
avc sample add <name> --kind audio|image|text --uri <path>
avc corpus create --name <n> --source-path <dir>

# 跑
avc render run --persona <name> --version <v> --topic "..." --duration 60
avc finetune start <name> --base-version <v> --scope voice
avc finetune run <fj_id> --embed openai
avc job export <id> --out <dir> | --target s3://bucket/

# 交互
avc shell
avc ask "用 <name> 出片，主题是 ..."
```

---

## 11. 上哪儿继续

| 你想了解 | 文档 |
|---|---|
| 完整 CLI 命令参考 | `docs/cli.md` |
| 数据 schema / 表 | `docs/storage.md` |
| 5 节点 pipeline 详解 | `docs/modules/pipeline.md` |
| Refine / Finetune 语义 | `docs/modules/persona-iteration.md` |
| 5 维度 provider 配置 | `docs/providers.md` |
| 部署 / systemd / 备份 | `docs/operations.md` |
| 整体架构 | `docs/architecture.md` |
| Rust crate API | `docs/api/README.md` |
| 设计原文 | `docs/persona-lifecycle.md` |
| 项目状态 | `docs/status.md` |
| Vendor CLI 协议模板 | `examples/vendor-cli/*.sh` |
| 完整配置模板 | `examples/avc.toml.template` |
