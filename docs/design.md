# AVCore 设计文档（Design Document）

> 面向开发者的纯后端 AI 视频生成核心框架。本文档回答"做什么、流程如何流转、模块如何协作"。

---

## 1. 项目定位

**AVCore（AI Video Core）** 是一个面向开发者的纯后端框架，让开发者能够以 SDK / API 的方式构建"AI 数字人视频生成"能力，覆盖从"造一个角色"到"让角色讲一段专业内容并产出成片"的完整链路。

- **目标用户**：集成方开发者（接入 AI 视频能力的产品 / 中台 / 业务系统）。
- **目标场景**：营销短视频、企业培训、口播讲解、课程内容生产、数字员工等。
- **设计原则**：
  - **后端优先**：不绑定任何前端 / 客户端形态，仅暴露 API / SDK / 异步任务。
  - **模型无关**：每个能力点（形象、声音、视频、LLM）抽象为 Provider，可插拔。
  - **可编排**：将"人物 + 知识 + 音频 + 画面"作为可组合的 Pipeline。
  - **可资产化**：角色、声音、知识、模板都是可复用资产。

---

## 2. 核心概念与领域模型

| 概念 | 说明 | 关键属性 |
|------|------|----------|
| `Character` 角色 | 视频中的"演员"，是形象 + 声音 + 人设的聚合体 | id, name, persona, avatar_id, voice_id, expert_id |
| `Avatar` 形象 | 角色的视觉外观（图片 / LoRA / 3D） | id, type, ref_images, style, lora_weights |
| `Voice` 声音 | 角色的声纹与 TTS 音色 | id, provider, sample_audio, language, emotion_tags |
| `Persona` 人设 | 角色的性格、口吻、对话风格 | id, traits, tone, taboo, scenario_prompt |
| `Expert` 专家 | 角色所代表的领域知识 | id, domain, knowledge_corpus_id, style_id |
| `KnowledgeCorpus` 知识语料 | 喂给专家的事实 / 文档 / FAQ | id, source, chunks, embeddings |
| `Script` 脚本 | 一次视频任务的"剧本" | id, character_id, scenes[], bgm_id, duration |
| `Scene` 分镜 | 脚本中的单段镜头 | id, narration, visual_prompt, avatar_action, duration |
| `Asset` 资产 | 形象 / 声音 / BGM / 模板等可复用素材 | id, type, uri, meta |
| `Job` 任务 | 一次端到端视频生成的运行实例 | id, status, progress, artifacts, logs |
| `Template` 模板 | 预编排好的脚本 + 风格 | id, name, scene_template, defaults |

### 关系示意

```
Character ──┬─ Avatar  (1..1)
            ├─ Voice   (1..1)
            ├─ Persona (1..1)
            └─ Expert  (1..1) ── KnowledgeCorpus (1..N)

Script ──┬─ Character
         ├─ BGM Asset
         └─ Scene[] ── Avatar Action / Visual Prompt / Narration

Job ──── Script + render_options + outputs
```

---

## 3. 端到端业务流

整个视频生产链路被抽象为 **"造角 → 养成 → 拍戏"** 三阶段：

```
┌─────────────────────────────────────────────────────────────┐
│  阶段 1: 人物形象建模（Character Modeling）                  │
│   角色设定 + 参考图/描述 + 声音样本  →  Avatar + Voice      │
└──────────────────────────┬──────────────────────────────────┘
                           ▼
┌─────────────────────────────────────────────────────────────┐
│  阶段 2: 养成（角色养成 + 专家养成）                         │
│   Persona + 领域知识语料      →  Persona + Expert           │
└──────────────────────────┬──────────────────────────────────┘
                           ▼
┌─────────────────────────────────────────────────────────────┐
│  阶段 3: 视频生成（Video Generation）                        │
│   主题 + Character + Script 模板  →  Script  →  Video       │
└─────────────────────────────────────────────────────────────┘
```

### 3.1 阶段 1：人物形象建模

**目的**：把抽象的"角色设定"变成可在后续步骤中复用的形象资产 + 声音资产。

输入：
- 角色设定（自然语言描述、参考图、风格关键词）
- 声音样本（可选，若使用声音克隆）

输出：
- `Avatar`：可用于 SDXL / Hunyuan / 即梦 / Kling avatar 等的形象资产
- `Voice`：可用于 CosyVoice / GPT-SoVITS / F5-TTS / 商用 TTS 的音色

关键能力：
- 形象生成：文生图 + 参考图微调（LoRA / IP-Adapter）
- 形象一致性：固定 seed / face id 锁定 / 多视角一致性
- 声音克隆：few-shot 样本训练 / 商用 TTS 音色 ID
- 声音控制：情绪、语速、停顿标记（SSML）

### 3.2 阶段 2：养成（角色 + 专家）

**目的**：让"演员"不仅长得像，还要"会说话、会思考、专业"。

#### 角色养成
- 设定性格、口头禅、禁忌、说话风格
- 行为偏好：对不同场景的反应模板
- 长期记忆（可选）：跨任务的角色成长曲线

#### 专家养成
- 灌入垂直领域语料（文档 / 问答 / 术语表）
- 形成 `KnowledgeCorpus`，提供 RAG 检索
- 设定输出风格：术语偏好、句式长度、合规边界
- 形成 `Expert`，绑定到 Character

### 3.3 阶段 3：视频生成

**目的**：把"演员 + 剧本 + 音视频素材"拼装成最终视频。

**Step A：脚本生成**
- 输入：主题 / 大纲 / 关键点 / 模板
- 调 LLM（结合 Persona + Expert RAG）生成分镜 `Scene[]`
- 每段包含：旁白文本、画面 prompt、表情 / 动作提示、时长

**Step B：音频生成**
- 旁白 TTS（使用 Character.Voice）
- 情绪 / 停顿 / 重音控制
- 可选 BGM 匹配（按场景情绪推荐或选择）
- 输出音轨 + 时间戳

**Step C：画面生成**
- 形象驱动：图生视频 / 关键帧 + 动作 / 商用数字人
- 多模态视频模型（AnimateDiff / Kling / CogVideoX）按 Scene 渲染
- 口型同步（wav2lip / video-retalking）
- 输出镜头片段

**Step D：合成与后期**
- 多镜头拼接、转场、字幕烧录
- BGM 合成、音量平衡
- 输出最终视频（mp4）+ 封面 + 缩略图

---

## 4. 用户故事

| 编号 | 故事 | 涉及模块 |
|------|------|----------|
| US-01 | 作为开发者，我用 API 创建一个"AI 讲师 Lily"角色：上传形象描述 + 声音样本，得到可复用的 Character | 形象建模 |
| US-02 | 作为开发者，我给 Lily 灌入"高中物理"知识语料，让她成为物理专家 | 专家养成 |
| US-03 | 作为开发者，我用脚本模板让 Lily 讲解"牛顿第一定律"，得到 60s 视频 | 视频生成 |
| US-04 | 作为开发者，我批量生产 100 条营销口播视频，复用同一角色 | 视频生成（批处理）|
| US-05 | 作为开发者，我用 Webhook 拿到生成进度与最终视频地址 | 任务系统 |
| US-06 | 作为开发者，我把生成好的视频做二次剪辑（替换某段旁白） | 视频生成（局部重渲染）|

---

## 5. 功能性需求

### 5.1 角色与资产管理
- 角色 CRUD、克隆、版本化
- 资产（形象 / 声音 / BGM / 模板）独立管理
- 资产可被多个角色引用

### 5.2 形象与声音
- 支持文生图、图生图、人像一致性
- 支持少样本声音克隆（≤30s 样本）
- 支持商用 TTS 厂商音色（按音色 ID 调用）

### 5.3 知识与专家
- 支持文档 / 网页 / FAQ 导入
- 自动切分、向量化、入库
- 支持热更新：知识可增量追加
- 检索：BM25 + 向量混合检索

### 5.4 脚本与编排
- LLM 生成分镜：支持模板 / 风格 / 时长约束
- 脚本可编辑：开发者拿到 JSON 后可二次修改
- 支持从已有视频反推脚本（可选）

### 5.5 视频生成
- 单镜头渲染：5s ~ 60s
- 多镜头拼接：自动转场 / 字幕烧录
- 音视频同步：口型同步
- 后期：背景音乐、滤镜、水印、片头片尾

### 5.6 任务系统
- 同步 / 异步任务
- 进度推送（Webhook / WebSocket / SSE / 轮询）
- 失败重试、断点续跑
- 资源调度（GPU 池）

---

## 6. 非功能性需求

| 维度 | 目标 |
|------|------|
| 性能 | 单条 60s 视频，端到端 P95 ≤ 8 分钟（带 GPU 集群） |
| 吞吐 | 单实例支持 ≥ 50 并发生成任务 |
| 可用性 | 99.5%（业务不依赖单一模型厂商） |
| 可扩展 | 任意 AI 模型可作为 Provider 接入 |
| 可观测 | 全链路 Trace、日志、指标、计费埋点 |
| 安全 | API Key / OAuth、租户隔离、内容审核、版权水印 |
| 合规 | 数字人形象 / 声音需提供授权证明，平台保留审核流水 |

---

## 7. 关键交互流程

### 7.1 创建角色（一次性）

```
Client                 API                    Modeling Service        Provider
  │                     │                            │                   │
  │  POST /characters   │                            │                   │
  │  (persona, refs)    │                            │                   │
  ├────────────────────▶│  create_avatar()           │                   │
  │                     ├───────────────────────────▶│  text2img/LoRA     │
  │                     │                            ├──────────────────▶│
  │                     │                            │◀──── avatar_id ───│
  │                     │  create_voice()            │                   │
  │                     ├───────────────────────────▶│  TTS clone        │
  │                     │                            ├──────────────────▶│
  │                     │                            │◀──── voice_id ────│
  │                     │  persist Character         │                   │
  │                     │◀─────── character_id ──────│                   │
  │◀──── 201 Created ───│                            │                   │
```

### 7.2 视频生成（每次任务）

```
Client           API            Pipeline            TTS        Image2Video     Composer
  │                │                │                 │              │               │
  │ POST /jobs     │                │                 │              │               │
  │ (char,topic)   │                │                 │              │               │
  ├───────────────▶│  build_script  │                 │              │               │
  │                ├───────────────▶│  LLM + RAG      │              │               │
  │                │◀── script ─────│                 │              │               │
  │                │  tts(scene)    │                 │              │               │
  │                ├────────────────┼────────────────▶│              │               │
  │                │◀── audio+ts ───┼─────────────────┤              │               │
  │                │  render(scene) │                 │              │               │
  │                ├────────────────┼─────────────────┼─────────────▶│               │
  │                │◀── clip ───────┼─────────────────┼──────────────┤               │
  │                │  compose       │                 │              │               │
  │                ├────────────────┼─────────────────┼──────────────┼──────────────▶│
  │                │◀── final.mp4 ──┼─────────────────┼──────────────┼────────────────┤
  │◀── 200 job ────│                │                 │              │               │
```

---

## 8. 扩展性 / 插件化设计

四大模块均设计为 **Provider 抽象**：

```python
class AvatarProvider(Protocol):
    def create(self, spec: AvatarSpec) -> Avatar: ...
    def render(self, avatar: Avatar, prompt: str, motion: Motion) -> Media: ...

class VoiceProvider(Protocol):
    def clone(self, samples: list[Audio]) -> Voice: ...
    def synth(self, voice: Voice, text: str, ssml: SSML) -> Audio: ...

class LLMProvider(Protocol):
    def chat(self, msgs, tools, **kw) -> LLMResponse: ...

class VideoProvider(Protocol):
    def render(self, scene: Scene, avatar: Avatar, audio: Audio) -> Clip: ...
```

新增模型（MiniMax-Hailuo、可灵 V2、即梦、Pika、Sora）只需实现 Provider。

---

## 9. 内容审核与安全

- 文本前置审核：脚本生成后必须通过合规审核（涉政、涉黄、涉暴、涉广）
- 形象授权：上传参考图需提供"已获授权"声明
- 声音授权：声音克隆需提供被克隆人授权
- 输出水印：默认烧录平台水印，可按租户去除
- 内容追溯：每条视频保留生成日志、模型版本、参数快照

---

## 10. 计费模型

- 按资源消耗计费（推荐）：形象、声音、脚本、视频分开计点
- 按时长计费：视频按秒计费
- 按调用计费：API 调用次数
- 支持租户配额、预付费 / 后付费

---

## 11. 后续阅读

- 架构文档：[architecture.md](./architecture.md)
- 子模块设计：
  - [人物形象建模](./modules/character-modeling.md)
  - [角色养成](./modules/character-cultivation.md)
  - [专家养成](./modules/expert-cultivation.md)
  - [视频生成](./modules/video-generation.md)
  - [工作流编排](./modules/pipeline.md)
- API 概览：[api/README.md](./api/README.md)
