# AVCore 设计文档（Design Document）

> 面向开发者的纯后端 AI 数字人视频生成核心框架。本文档回答"做什么、流程如何流转、模块如何协作"。

---

## 1. 项目定位

**AVCore（AI Video Core）** 是一个面向开发者的纯后端框架，让开发者能够围绕一个**人物角色模型（PersonaModel）** 构建"从塑形到持续演进，再到产出成片"的完整链路。

模型既可以是：
- **技术专家 / 行业讲师**（带领域语料）
- **形象鲜明的虚构人物**（虚拟主播、品牌代言人、游戏 NPC 复刻）
- **真实人物的数字孪生**（本人授权的形象 + 声音复刻）
- **虚拟员工 / 数字助手**（知识型或不带知识型都可以）
- ……任何"具备视觉 + 听觉 + 人设"的稳定身份

最重要的是：

> **模型被持续训练**。它不是造一次定型，而是随业务运营**不断追加样本、微调、出新版本**；并且演进的版本与历史版本身份一致、不漂移。

- **目标用户**：集成方开发者（接入 AI 数字人能力的产品 / 中台 / 业务系统）。
- **目标场景**：营销短视频、企业培训、口播讲解、课程内容生产、数字员工、虚拟主播、品牌虚拟代言人等。
- **设计原则**：
  - **后端优先**：不绑定任何前端 / 客户端形态，仅暴露 API / SDK / 异步任务。
  - **以 PersonaModel 为中心**：形象 / 声音 / 人设 / 知识（可选）都是模型的可装可拆维度。
  - **可演进**：模型可追加样本、微调、产出新版本；旧版本永不被覆盖，可继续被引用。
  - **可编排**：将"训练 + 渲染"统一拆成 DAG 节点。
  - **可资产化**：PersonaModel 及其各版本独立管理，可被复用 / 共享 / 灰度。
  - **模型无关**：每个能力点（形象、声音、视频、LLM）抽象为 Provider，可插拔。

---

## 2. 核心概念与领域模型

### 2.1 顶层抽象

```
PersonaModel     ── 一次创建，跨版本不变，是"被运营的角色"
PersonaModelVersion ── PersonaModel 的具体某次快照，**不可变**
TrainingJob      ── 从一个版本产出下一个版本的训练任务
PersonaSample    ── 训练样本（图 / 音 / 行为文本 / 反馈）
KnowledgeCorpus  ── 可选语料；只在 persona 是领域专家时绑定
VideoJob         ── 用 PersonaModelVersion 出的成片任务
```

### 2.2 核心实体表

| 概念 | 说明 | 关键属性 |
|------|------|----------|
| `PersonaModel` | 一个"被运营的角色"的顶层聚合 | id, name, archetype, current_version, version_ids[], status |
| `PersonaModelVersion` | 一个**不可变快照**（v1/v2/...），含所有资产与锚点 | version, avatar_id, voice_id, persona_descriptor, knowledge?, identity_anchor, parent_version_id, metrics |
| `Avatar` | 角色的视觉外观（图片 / LoRA / 3D） | id, provider, primary_image, ref_images, lora, face_id |
| `Voice` | 角色的声纹与 TTS 音色 | id, provider, voice_id, sample_audio, language, supported_emotions |
| `PersonaDescriptor` | 性格 / 语气 / 口头禅 / 禁忌 / 场景化 prompt | traits, tone, catchphrases, taboos, scenario_prompts, formality, temperature |
| `KnowledgeCorpus` | 领域语料（可选） | id, source_type, chunk_count, index_version |
| `KnowledgeBinding` | 把语料挂到 persona 的方式 | corpus_ids, domain, grounding_mode, retrieval, style |
| `IdentityAnchor` | 跨版本可比对的锚点特征 | face_embedding, voice_embedding, style_embedding |
| `PersonaSample` | 训练样本（任何维度） | kind (image/audio/behavior_text/feedback), uri/text, source, version_id_at_collection |
| `TrainingJob` | 一次训练运行实例 | base_version_id, target_version, scope[], sample_ids, config, status, result_version_id |
| `Script` | 一次视频任务的"剧本" | persona_model_id, persona_version_id, scenes[] |
| `Scene` | 脚本中的单段镜头 | narration, visual_prompt, avatar_action, duration_ms |
| `VideoJob` | 端到端渲染任务 | script_id, persona_version_id, status, artifacts |
| `Asset` | 形象 / 声音 / BGM / 模板 等可复用素材 | id, type, uri, meta |
| `Template` | 预编排好的脚本 + 风格 | id, name, scene_template, defaults |

### 2.3 关系示意

```
PersonaModel ──┬─ PersonaModelVersion[] ──┬─ Avatar        (不可变快照)
               │                        ├─ Voice         (不可变快照)
               │                        ├─ PersonaDescriptor
               │                        ├─ KnowledgeBinding (0..1, 可选)
               │                        └─ IdentityAnchor
               │
               ├─ TrainingJob[]           (演进历史)
               ├─ PersonaSample[]         (训练样本池)
               │
               └─ current_version → PersonaModelVersion

Script ──┬─ PersonaModel
         ├─ PersonaModelVersion (锁定)
         ├─ Scene[]
         └─ BGM Asset

VideoJob ─── Script + render_options
              锁定 persona_version_id（永不漂移）

KnowledgeCorpus ────PersonaModelVersion.knowledge
```

---

## 3. 端到端业务流

整条业务被抽象为：**模型生成 → 模型演进 → 视频消费** 三段，且后两段会**循环发生**。

```
┌─────────────────────────────────────────────────────────────┐
│  阶段 1: 人物角色模型生成（一次创建）                         │
│   设定 + 参考图/声音样本 + 行为样本 + (可选)领域语料          │
│       → PersonaModel + Version v1                          │
└──────────────────────────┬──────────────────────────────────┘
                           ▼
┌─────────────────────────────────────────────────────────────┐
│  阶段 2: 人物角色模型完善演进（循环发生）                     │
│   追加样本(用户上传/用户反馈回灌) → TrainingJob             │
│   → Version vN (含一致性评估 / 漂移检测)                    │
│   → 决定发布或回退                                          │
└──────────────────────────┬──────────────────────────────────┘
                           ▼
┌─────────────────────────────────────────────────────────────┐
│  阶段 3: 视频消费（按需触发）                                │
│   主题 + 锁定 PersonaModelVersion + Script                   │
│       → VideoJob → final.mp4                               │
│   用户反馈 → 回流到阶段 2 的样本池                          │
└─────────────────────────────────────────────────────────────┘
```

### 3.1 阶段 1：人物角色模型生成

**目的**：把抽象的"角色设定"变成 `PersonaModel v1`，含形象 / 声音 / 人设 / 可选知识。

输入：
- 角色设定（自然语言描述、参考图、风格关键词）
- 声音样本（可选；不提供走商用音色）
- 行为样本（可选；让 LLM 抽取 persona 特征更准）
- 领域语料（可选；只有专家型 persona 才需要）

输出：
- `PersonaModel`（顶层标识）
- `PersonaModelVersion v1`（不可变快照）
  - `Avatar`
  - `Voice`
  - `PersonaDescriptor`
  - `KnowledgeBinding?`
  - `IdentityAnchor`

关键能力详见 [modules/persona-modeling.md](./modules/persona-modeling.md)。

### 3.2 阶段 2：人物角色模型完善演进（核心循环）

**目的**：让模型随业务运营越来越好，同时**不让它变得不像自己**。

驱动来源：
- 用户主动上传图 / 音 / 行为样本
- 视频生成反馈（点"不像本人"等）回灌成 `PersonaSample`
- 运营定期追加新场景样本

训练任务：
- 指定 `base_version_id` 与 `scope`（avatar / voice / persona / knowledge 任选）
- 训练 → Identity Anchor 抽取 → 漂移评估
- 达标 → 发布 `v(N+1)`；不达标 → 回退 + 报告

版本管理：
- 每个版本**不可变**：已渲染的视频永远锁定它生成时的版本
- `PersonaModel.current_version` 可改，不影响历史视频
- 支持强制回滚（指针回拨）

关键能力详见 [modules/persona-evolution.md](./modules/persona-evolution.md)。

### 3.3 阶段 3：视频生成

**目的**：用某个 PersonaModelVersion + 主题 + 脚本出成片。

输入：
- `persona_model_id` + `persona_version_id`（不指定 = 默认版本）
- `topic` + `key_points` + `target_duration` + 可选模板
- `JobRenderOptions`（分辨率、字幕、水印等）

输出：完整视频 + 封面 + 字幕 + meta.json

执行链：
1. 脚本生成（LLM + RAG + Persona prompt）
2. 旁白 TTS（用该 version 的 voice_id）
3. 关键帧（用该 version 的 avatar_id）
4. 图生视频（i2v）
5. 口型同步（可选）
6. 后期合成 / 字幕 / BGM / 水印
7. 封装输出，**在 meta 上烙印 version_id**（用于追溯 & 内容审核）

关键能力详见 [modules/video-generation.md](./modules/video-generation.md)。

### 3.4 反馈闭环

```
用户对成片点"不像 / 不喜欢" 
   │
   ▼
POST /v1/jobs/{jid}/feedback (信号 + 可选文本)
   │
   ▼
后端转成 PersonaSample(kind=feedback)，进入样本池
   │
   ▼
下一次 / 下下次 TrainingJob 消费该样本
```

---

## 4. 角色用例

### 4.1 案例 A：技术专家
- 创建 v1：形象、声音、人设（耐心 / 严谨）+ 高中物理语料 → 一键出"高中物理讲解视频"
- 后续 v2：运营补语料（更高质量题目）→ 重建索引
- 后续 v3：根据完播率分析追加 50 段"学生最爱听的开场白"样本 → 人设微调

### 4.2 案例 B：虚拟品牌代言人
- 创建 v1：形象、声音、人设（活力 / 简洁）+ 不挂任何知识
- 后续 v2：根据"今年新品上线"补一段"新品讲解"专用 prompt → 人设微调
- 后续 v3：根据用户投票，把"活泼风"换成"高级感风" → 视觉重训

### 4.3 案例 C：真实人物数字孪生
- 创建 v1：本人的 30s 视频做声音克隆，本人照片做形象建模
- 后续 v2：加表情样本让笑容更像本人 → 视觉微调
- 后续 v3：替换为新一年造型 / 新衣服 → 视觉迭代

### 4.4 案例 D：游戏 NPC 二创
- 创建 v1：手绘画风 LoRA + 自录台词声音 + 写实略夸张人设
- 后续 v2：玩家后台支持 / 投稿大量对白 → 人设 SFT
- 后续 v3：根据玩家社区数据改进 NPC 反应风格

> 知识维度从来不是必须的——只有 A 真正用了。

---

## 5. 不做什么

- 不做前端 / 编辑器
- 不做内容审核的最终判定（只提供检测 + 拦截接口）
- 不做训练框架本身（用的是业界成熟 SFT / 偏好对齐方案，AVCore 只做编排与状态管理）
- 不做实时直播（虽然将来可能扩，但当前只做离线出片）
- 不"自动创作"出新的 PersonaModel（创建必须由人触发）

---

## 6. 与传统"专家系统"的区别

| 维度 | 传统专家系统 | AVCore |
|------|--------------|--------|
| 核心实体 | 知识库 + 推理规则 | PersonaModel（身份聚合）+ 可选知识 |
| 知识定位 | 必备 | 可选维度 |
| 是否多版本 | 通常单一 | 一致的多版本快照 |
| 是否持续训练 | 否 | 是（核心能力） |
| 视觉表现 | 通常无 | 形象 + 声音 + 行为 |
| 主要产物 | 答案文本 | 视频 |

---

## 7. 关键流程图

### 7.1 首次创建 + 首次出片

```
Client              Persona Modeling           Providers
  │                       │                       │
  │ create_persona_model  │                       │
  ├──────────────────────▶│  创建 Avatar          │
  │                       ├──────────────────────▶│
  │                       │◀── avatar_id ─────────┤
  │                       │  创建 Voice           │
  │                       ├──────────────────────▶│
  │                       │◀── voice_id ──────────┤
  │                       │  写库 v1 (snapshot)   │
  │◀── 201 Created ───────│                       │
  │     { persona_model_id, version_id }           │
                                                  │
                                                  ▼
                                                Video Pipeline
                                                  │
                                                  ▼
                                          TTS → img → i2v → compose
                                                  │
                                                  ▼
                                            final.mp4 + meta
                                            (meta 含 persona_version_id)
```

### 7.2 持续训练循环

```
Client             Persona Evolution Svc    Providers         DB
  │                       │                    │                │
  │ upload samples        │                    │                │
  ├──────────────────────▶│  写入 PersonaSample                 │
  │                       ├────────────────────────────────────▶│
  │                       │                    │                │
  │ create training_job   │  scope=[avatar,voice]              │
  ├──────────────────────▶│  入队 TrainingJob                   │
  │                       ├──────────────────▶│  LoRA / TTS    │
  │                       │                    │   微调         │
  │                       │  Identity Anchor                    │
  │                       │  漂移评估（vs base）                 │
  │                       │  分支：达标→发布 v2                 │
  │                       │        不达标→回退 + report         │
  │◀── report ────────────│                    │                │
```

### 7.3 用户反馈回灌

```
Client (观看成片)   Video Job Svc         Persona Evolution
  │                       │                       │
  │ POST /jobs/{jid}/feedback                       │
  ├──────────────────────▶│  转 PersonaSample     │
  │                       ├──────────────────────▶│
  │                       │  下一次 training 消费 │
```

---

## 8. 扩展性 / 插件化设计

五大模块均设计为 **Provider 抽象**：

```python
class AvatarProvider(Protocol):
    def create(self, spec: AvatarSpec) -> Avatar: ...
    def render(self, avatar: Avatar, prompt: str, motion: Motion) -> Media: ...
    def finetune(self, avatar: Avatar, samples: list[Sample]) -> Avatar: ...

class VoiceProvider(Protocol):
    def clone(self, samples: list[Audio]) -> Voice: ...
    def synth(self, voice: Voice, text: str, ssml: SSML) -> Audio: ...
    def finetune(self, voice: Voice, samples: list[Audio]) -> Voice: ...

class LLMProvider(Protocol):
    def chat(self, msgs, tools, **kw) -> LLMResponse: ...
    def sft(self, dataset, **kw) -> Model: ...

class VideoProvider(Protocol):
    def render(self, scene: Scene, avatar: Avatar, audio: Audio) -> Clip: ...

class KnowledgeProvider(Protocol):
    def chunk(self, document) -> list[Chunk]: ...
    def embed(self, chunks) -> list[Embedding]: ...
    def search(self, query, k) -> list[Hit]: ...
```

新增模型（MiniMax-Hailuo、可灵 V2、即梦、Pika、Sora、MiniMax、混元等）只需实现 Provider。

---

## 9. 内容审核与安全

- 文本前置审核：脚本生成后必须通过合规审核（涉政、涉黄、涉暴、涉广）
- **形象 / 声音授权**：上传参考图 / 声音样本必须提供授权存证
- **真实人物复刻**：默认禁止；开启需通过更严格的合规审核
- 输出水印：默认烧录平台水印 + 不可见水印（含 persona_version_id），用于追溯
- **追溯**：每条视频保留 persona_version_id + 所有节点 trace_id + provider 版本 + 参数快照

---

## 10. 计费模型

- 按资源消耗计费（推荐）：训练样本数 / 训练时长 / 渲染时长 三段独立计点
- 按版本周期订阅：面向"持续运营一个模型"的客户
- 按时长计费：视频按秒计费
- 按调用计费：API 调用次数
- 支持租户配额、预付费 / 后付费

---

## 11. 后续阅读

- 架构文档：[architecture.md](./architecture.md)
- 子模块设计：
  - [人物角色模型生成](./modules/persona-modeling.md)
  - [人物角色模型完善演进](./modules/persona-evolution.md)
  - [视频生成](./modules/video-generation.md)
  - [工作流编排](./modules/pipeline.md)
  - [知识能力（可选）](./modules/knowledge-aspect.md)
- API 概览：[api/README.md](./api/README.md)
