# AVCore Persona 模型

> AVCore 把"角色"抽象成什么，怎么迭代，怎么持久化。
> 这是"概念层"——和 `docs/user-guide.md`（how-to 教程）、`docs/modules/persona-iteration.md`（refine/finetune 语义）并列。

---

## 1. 核心抽象

**一个 AVCore persona 就是：**

```
┌─────────────────────────────────────┐
│            persona (人 + 设)            │
│                                     │
│   ┌──────────┐  ┌──────────────────┐  │
│   │  人格    │  │  资产             │  │
│   │  prompt  │  │  ┌────────────┐  │  │
│   │  (文本)  │  │  │  视频/形象  │  │  │
│   │          │  │  └────────────┘  │  │
│   │          │  │  ┌────────────┐  │  │
│   │          │  │  │  声音       │  │  │
│   │          │  │  └────────────┘  │  │
│   │          │  │  ┌────────────┐  │  │
│   │          │  │  │  知识库     │  │  │
│   │          │  │  └────────────┘  │  │
│   └──────────┘  └──────────────────┘  │
└─────────────────────────────────────┘
                  ↓
          render → 5 节点 DAG → mp4
```

| 用户视角的"角色一部分" | AVCore 数据库字段 |
|---|---|
| 人格 / 提示词 / 表达风格 | `persona_versions.persona_descriptor_json` |
| 形象 / 脸 | `persona_versions.avatar_primary` (BLOB) |
| 声音 / 音色 | `persona_versions.voice_sample_wav` (BLOB) |
| 知识库 / 引用 | `persona_versions.knowledge_binding_json` |
| 渲染配置 | `persona_versions.manifest_json` |

> 用户的"15 秒自我介绍"视频 = 1 段 3-scene 的 prompt + 调 vendor 出的 3 段 mp4 → compose 拼接成 15s。

---

## 2. 数据模型（SQLite 持久化）

| 表 | 存什么 | 关键字段 |
|---|---|---|
| `persona_models` | 角色身份 | `name`, `archetype`, `current_version`, `status` |
| `persona_versions` | 角色**版本**快照 | `(persona_model_id, version=1)`, `persona_descriptor_json`, `avatar_primary`, `voice_sample_wav`, `manifest_json`, `knowledge_binding_json` |
| `persona_samples` | 训练样本 | `kind` (image/audio/feedback), `text`, BLOB |
| `jobs` | render 任务 | `status` (pending/running/succeeded/failed) |
| `job_steps` | 5 节点 step 详情 | `node_id`, `status`, `outputs_json`, `error_json`, `duration_ms` |
| `artifacts` | 节点产物 | `kind` (script/audio/image/video), BLOB, `byte_size`, `sha256` |
| `iterate_jobs` | refine 任务账本 | `target_version`, `changes_json` |
| `finetune_jobs` | finetune 任务账本 | `scope`, `drift_report_json`, `status` |

**关键**：`persona_versions` 是**版本化**的——同一个 persona 名字（如 `yu`）可以有多行 `(yu, 1)`、`(yu, 2)`、`(yu, 3)`，每行完整 1 套（prompt + 资产 + 配置）。`persona_models.current_version` 是个指针（默认 1，可切到 N）。

---

## 3. 持久化保证

数据库文件：`~/.local/share/avc/avc.db`（SQLite + WAL）

| 操作 | 持久？ | 验证命令 |
|---|---|---|
| 创建 persona | ✅ 立刻落库 | `avc persona list` |
| iterate apply 改 prompt | ✅ 落 persona_versions | `avc iterate list <n>` |
| render run 创建 job | ✅ 落 jobs | `avc job list` |
| render 中间产物 | ✅ 落 job_steps + artifacts | `avc job show <id>` |
| 切换 current version | ✅ 落 persona_models | `avc persona versions <n>` |
| 加样本 | ✅ 落 persona_samples | `avc sample list <n>` |
| finetune 出 v2 | ✅ 落 finetune_jobs + v2 行 | `avc finetune show <id>` |
| **重启机器** | ✅ **全部可读回** | `avc persona show yu` |

> **重启后 prompt + 资产 + 视频** 都在。这是核心：用户在前一次 run 出来的不满意视频，下次开机改 prompt 重渲；上次视频作为参考 / 样本 / 起点，全部在库里。

---

## 4. 4 路径迭代模型

persona 进化的 4 条路径（推荐**按这个顺序**用）：

### 4.1 路径 A：改 prompt 重渲（最常见）

```bash
avc iterate apply yu --version 1 --set-persona '{
  "role": "...",
  "style": "..."
}'
avc render run --persona yu --version 1 --topic "..."
# 同一个 v1 重新生成，prompt 不同 → 视频不同
```

- **不改 version 编号**（还是 v1）
- prompt 是 persona_descriptor_json 字段更新
- 适合：微调风格 / 改领域 / 调结构

### 4.2 路径 B：finetune 出 v2（asset 进化）

```bash
# 1. 加样本
avc sample add yu --kind image --uri /tmp/img_01.png
avc sample add yu --kind image --uri /tmp/img_02.png
avc sample add yu --kind audio --uri /tmp/aud_01.wav --text "在源码里找答案"

# 2. 启 finetune job
avc finetune start yu --base-version 1 --scope voice   # 或 avatar / persona

# 3. finetune run（v1 走 vendor CLI 异步；3 步 submit / poll / retrieve）
avc finetune run <fj_id> --embed openai

# 4. 成功后 v2 自动 ready
avc persona versions yu    # → [1, 2]
```

- **v2 = v1 + 资产进化**（声音克隆 / 形象调优 / 知识库增厚）
- v1 不删
- 适合：训练新声音 / 精调形象 / 加领域知识

### 4.3 路径 C：换当前版本

```bash
avc persona versions yu      # 看所有版本
avc persona current yu --set 2   # 切到 v2
# 之后所有 render/iterate 默认用 v2
```

- 不删旧版本
- 适合：A/B 测试、紧急回滚

### 4.4 路径 D：asset 复用（I2V，v1 暂不支持）

```bash
# v1 出片后用 v1 视频作为 v2 的首帧引导
# 路径 B 的 finetune 内置支持；独立的 I2V 路径 v0.4+
```

- **v1 不直接支持**——需要 finetune 内嵌
- 适合：连续性、风格锁定

---

## 5. "15 秒自我介绍"完整示例

### 5.1 一次性创建 + 渲染

```bash
# 1. 创建角色（v1，初始空）
avc persona create --name yu \
  --archetype db_kernel_expert \
  --description "PostgreSQL 内核开发专家" \
  --catchphrase "show me the source"

# 2. 写 v1 完整 prompt（3 scene，自我介绍风）
avc iterate apply yu --version 1 --set-persona '{
  "role": "PostgreSQL 内核开发工程师，10 年经验",
  "expertise": ["WAL", "MVCC", "查询优化", "buffer manager"],
  "self_intro": "我是 yu，一个 PG 内核老兵",
  "style": "直接给源码 + gdb 断点示例，简洁不废话",
  "audience": "中高级 PG 开发者"
}'

# 3. 渲 v1
# AVCore 的 5 节点 DAG：script_gen → tts × 3 → img_gen × 3 → i2v × 3 → compose
# 每段 video clip 默认 5s；3 段 = 15s
avc render run --persona yu --version 1 \
  --topic "15 秒自我介绍：PostgreSQL 内核专家 yu" \
  --llm-provider minimax \
  --avatar-provider yu_minimax \
  --voice-provider yu_minimax \
  --video-provider yu_minimax
# → job_id

# 4. 看 + 导出
avc job show <job_id>          # 5 节点状态
avc job export <job_id> --out /tmp/yu-intro/
ls /tmp/yu-intro/
# script.json  voice_1.mp3 voice_2.mp3 voice_3.mp3
# image_1.jpg image_2.jpg image_3.jpg
# video_1.mp4 video_2.mp4 video_3.mp4
# final.mp4  (15s)
```

### 5.2 迭代：改 prompt 重渲

```bash
# 觉得 v1 太严肃，活泼一点
avc iterate apply yu --version 1 --set-persona '{
  "role": "...",
  "style": "用类比 + 故事讲 PG 内核",
  ...
}'
avc render run --persona yu --version 1 --topic "..."
# 同一个 v1 重新渲，prompt 不同 → 视频不同
```

### 5.3 迭代：finetune 出 v2

```bash
# 觉得 v1 声音不对，加几段样本训练
avc sample add yu --kind audio --uri /tmp/yu-voice-1.wav --text "我看了下 InnoDB 源码"
avc sample add yu --kind audio --uri /tmp/yu-voice-2.wav --text "WAL 是预写日志"
avc finetune start yu --base-version 1 --scope voice
# 配 video vendor binary（v1 必填）
avc config set provider.video.kling.binary "/path/to/vendor-cli"
avc finetune run <fj_id> --embed openai
# 成功后 v2 自动 ready

# 切到 v2
avc persona versions yu
avc persona current yu --set 2
```

---

## 6. v1 实际能给的 vs 不能给的

### ✅ 能

- **手动迭代 prompt**：改 → 重渲
- **多版本管理**：v1 / v2 / v3 都在；切 current
- **数据持久化**：重启不丢
- **15 秒自然实现**：3 scene × 5s
- **MiniMax 多模态 API 适配**：avatar / voice / video 全通
- **手工 finetune 流**：v1 + vendor CLI

### ❌ 不能（v1 范围）

- **render 自动跑 pipeline**——v1 不实现 `svc::job_worker`，daemon 不会触发；要么等要么手动
- **asset 复用 (I2V)**：v1 暂不支持独立 I2V 路径
- **持续对话**："ask" 是单次 LLM 翻译，不累积上下文
- **模型训练**："finetune" 是 vendor CLI 调用，不在 AVCore 进程内训练

---

## 7. 开发者模型（你的语言 → AVCore 抽象）

| 你说的 | AVCore 实际 |
|---|---|
| "角色 = 人格 + 视频 + 声音" | `persona_versions` row = 1 套 assets + 1 份 prompt |
| "根据人格和提示词生成视频" | `render run` 拿 `version N` 的 prompt + assets 调 vendor |
| "根据人格和视频修改完善" | 改 prompt（路径 A）/ finetune v2（路径 B） |
| "数据持久化" | SQLite + WAL |
| "重启后继续读出" | `avc persona show` / `avc job show` 全部读回 |
| "在基础上不断完善" | 4 路径迭代（A / B / C / D） |

**完全对得上**。本设计的核心是：

> 一个 persona 是 (`prompt`, `assets`) 的多版本快照。  
> 每次"渲染"是拿当前 version + 调用 vendor API 出 assets。  
> 每次"迭代"是改 prompt / finetune 出新 version。  
> 一切持久化在 SQLite，重启可读。

---

## 8. 相关文档

- `docs/user-guide.md` — 怎么用（how-to）
- `docs/modules/persona-iteration.md` — refine / finetune 语义
- `docs/storage.md` — 完整 schema
- `docs/providers.md` — 5 维度 provider 协议
- `docs/providers-cn.md` — 国产厂商选型
- `docs/minimax-api.md` — MiniMax 多模态实测
- `docs/operations.md` — 部署运维
