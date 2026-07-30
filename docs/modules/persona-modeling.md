# 模块设计：人物角色模型生成（Persona Modeling）

> 创建一个**人物角色模型**（PersonaModel）的初始版本 v1。它既可能是一个技术专家，也可能是一个形象鲜明的虚构人物 / 真实人物复刻 / 虚拟员工——核心是"一个可被识别、可被驱动的角色"。

---

## 1. 模块目标

**输入**：人物设定（自然语言）+ 可选参考素材
- 视觉参考：1~N 张参考图（已获授权）
- 声音样本：≥ 30s 干净人声（已获授权）
- 行为样本：该人物的语气、文风、典型对话片段
- 领域资料（可选）：技术专家 / 行业讲师 才需要；非必要不灌
- 参考人物（可选）：链接到外部知名人物的描述，用于风格借鉴（非克隆）

**输出**：`PersonaModel` 的首个版本（v1），包含
- `avatar`：视觉形象资产
- `voice`：声音资产
- `persona`：人设 / 风格 / 语气 / 禁忌
- `knowledge`（可选）：领域语料与检索配置
- `identity_anchor`：跨版本可比的"锚点特征"

**边界**：本模块只负责"**创建**"，不负责"**持续训练**"。后者见 [persona-evolution.md](./persona-evolution.md)。

---

## 2. 为什么以"PersonaModel"为中心

旧设计把"角色"和"专家"切成两条平行线，这强迫用户在做第一个角色时就要选择"它是普通人还是专家"。事实上：

- 同一个模型可以**先塑形，再灌知识**——先生成 v1 视觉+声音，后期 v2 才接入领域资料
- 同一个模型可以**完全不灌知识**——一个虚拟主播、一个播音主持、一个品牌虚拟代言人，根本不需要"专家"概念
- 知识只是 persona 的一个**可选维度**，不是必须属性

因此 AVCore 以 `PersonaModel` 为顶层聚合，把"形象、声音、人设、知识"作为可装可拆的能力维度。

---

## 3. 领域模型

```python
@dataclass
class PersonaModel:
    id: str                          # 顶层 ID，跨版本不变
    name: str                        # 角色名（"Lily" / "钱教授" / "Vlogger-A"）
    archetype: str                   # mentor / vlogger / anchor / mascot / instructor ...
    description: str                 # 一句话设定
    current_version: str            # 当前默认 version_id，默认指 v1
    version_ids: list[str]           # 历史版本列表（不可删，可停用）
    status: str                      # active / archived
    created_at: datetime
    updated_at: datetime
    meta: dict

@dataclass
class PersonaModelVersion:
    id: str                          # pmod_{uuid}_v{N}
    persona_model_id: str
    version: int                     # 1, 2, 3, ...
    parent_version_id: str | None    # 增量训练时记录父版本
    avatar_id: str                   # 形象资产 ID
    voice_id: str                    # 声音资产 ID
    persona: PersonaDescriptor       # 人设
    knowledge: KnowledgeBinding | None  # 可选
    identity_anchor: IdentityAnchor  # 跨版本一致性锚点
    training_job_id: str | None      # 产生此版本的训练任务
    metrics: VersionMetrics          # 一致性 / 风格 / 知识得分
    created_at: datetime
    status: str                      # building / ready / deprecated
    meta: dict

@dataclass
class PersonaDescriptor:
    traits: list[str]                # 性格词：耐心 / 幽默 / 严谨 / 犀利
    tone: str                        # 整体语气
    catchphrases: list[str]          # 口头禅
    taboos: list[str]                # 禁忌话题 / 措辞
    scenario_prompts: dict[str, str] # 场景化 prompt：教学 / 营销 / 客服
    response_length: str = "medium"
    formality: float = 0.5
    temperature: float = 0.7
    meta: dict

@dataclass
class KnowledgeBinding:
    corpus_ids: list[str]
    domain: str | None               # "高中物理" / "保险产品" / 可空
    grounding_mode: str = "loose"    # strict / loose / hybrid
    retrieval: RetrievalConfig       # top_k / threshold / 混合检索
    style: ExpertStyle               # 术语偏好 / 必含 / 禁用
    meta: dict

@dataclass
class IdentityAnchor:
    # 跨版本可比对的特征向量。视频生成时用作一致性兜底
    face_embedding: list[float] | None
    voice_embedding: list[float] | None
    style_embedding: list[float] | None
    created_at: datetime

@dataclass
class VersionMetrics:
    identity_consistency: float      # 与上一版本的相似度
    style_consistency: float         # 人设稳定性
    quality_score: float             # 人工 / LLM-as-Judge 总分
    notes: str
```

### 关系示意

```
PersonaModel ──┬─ PersonaModelVersion[] ──┬─ Avatar     (1..1, 不可变快照)
               │                        ├─ Voice      (1..1, 不可变快照)
               │                        ├─ PersonaDescriptor (1..1)
               │                        ├─ KnowledgeBinding  (0..1, 可选)
               │                        └─ IdentityAnchor    (1..1)
               │
               ├─ TrainingJob[]          (持续演进历史)
               ├─ Sample[]              (训练样本：图 / 音 / 行为文本 / 反馈)
               └─ VideoJob[]            (下游使用记录)

VideoJob ──── PersonaModelVersion(指定版本)
```

---

## 4. 视觉形象生成

### 4.1 输入
- `description`：自然语言（外貌、年龄、气质、风格关键词）
- `ref_images`：可选参考图，触发 IP-Adapter / InstantID / 风格参考
- 风格词：`写实 / 二次元 / 国风 / 3D 卡通 / 皮克斯风`

### 4.2 能力路径
| 路径 | 输入 | 输出 | 适用场景 |
|------|------|------|----------|
| 文生图 | description | 1~N 张候选形象 | 没有参考图 |
| 图生图 | 1 张参考图 + 风格 prompt | 多角度统一形象 | 有参考人像 |
| 多视角 | 关键 seed | 4~8 视角图 | 后续 3D / 头部驱动 |
| LoRA 微调 | ≥ 5 张高质量参考图 | LoRA 权重 + 一致性主形象 | 强一致性要求 |

### 4.3 数据契约

```python
@dataclass
class AvatarSpec:
    name: str
    description: str
    style_tags: list[str]
    ref_images: list[URI] = []
    age_range: tuple[int, int] | None = None
    gender: str | None = None
    ethnicity_hint: str | None = None

@dataclass
class Avatar:
    id: str
    provider: str                   # sdxl / kling-avatar / heygen
    primary_image: URI
    ref_images: list[URI]
    lora: URI | None
    face_id: str | None
    meta: dict
```

### 4.4 Provider 适配

| 任务 | Provider | 备注 |
|------|----------|------|
| 形象 | `sdxl_ip_adapter` | 自托管，性价比高 |
| 形象 | `kling_avatar` | 商用稳定 |
| 形象 | `heygen_avatar` | 商用 |
| 形象 | `flux_lora` | 高质量微调 |

---

## 5. 声音生成

### 5.1 输入
- 声音样本（`samples`）：每条 `{uri, duration_ms, text, language}`
- 样本要求：≥ 30s 干净人声、单说话人、无 BGM
- 不提供样本 → 走商用音色，匹配声纹近似度排序推荐

### 5.2 数据契约

```python
@dataclass
class VoiceSample:
    uri: URI
    duration_ms: int
    text: str                       # 该段对应文本（用于训练对齐）
    language: str = "zh"

@dataclass
class VoiceSpec:
    name: str
    language: str = "zh"
    samples: list[VoiceSample]
    emotion_baseline: str = "neutral"

@dataclass
class Voice:
    id: str
    provider: str                   # cosyvoice / gpt-sovits / volc-tts / azure-tts
    voice_id: str                   # 厂商侧 ID
    sample_uri: URI
    language: str
    supported_emotions: list[str]
    meta: dict
```

### 5.3 声音控制
- SSML：情绪、停顿、重音、语速
- 多情绪：同一 voice_id 切换情绪标签
- 相似度：speaker embedding cosine ≥ 0.80

---

## 6. 人设建模

输入：自然语言描述 + 可选行为样本（该角色典型语气示例片段）

输出：`PersonaDescriptor` 结构化字段（traits / tone / taboo / scenario_prompts ...）

生成路径：LLM 抽取 + 人工确认，提供内置模板（讲师 / 主播 / 客服 / 主持人 / 虚拟员工 / 故事讲述者 / 行业专家）。

更多约束与场景化 Prompt 设计见 [persona-evolution.md §4 人设训练](./persona-evolution.md#43)。

---

## 7. 知识接入（可选）

只有当 persona 真的代表"懂某个领域"时才接入。

输入：领域语料（文档 / 网页 / FAQ）+ 术语偏好
输出：`KnowledgeBinding`，挂载到当前版本

语料接入与 RAG 细节见 [knowledge-aspect.md](./knowledge-aspect.md)。

> 注意：不接入知识 ≠ 不能讲任何内容——一个形象鲜明的人设没有领域知识也能讲"段子"或"日常点评"。

---

## 8. 创建流程

```
Client                Persona Modeling Svc          Providers
  │                          │                          │
  │  create_persona_model()  │                          │
  ├─────────────────────────▶│  入队 job (create v1)    │
  │                          ├─────────────────────────▶│
  │                          │  [1] 形象生成 (avatar)    │
  │                          │  [2] 声音生成 (voice)    │
  │                          │  [3] 人设结构化          │
  │                          │  [4] 知识接入 (可选)     │
  │                          │  [5] Identity Anchor 抽取 │
  │                          │◀─────────────────────────┤
  │                          │  写库 PersonaModel + v1   │
  │◀──── 201 Created ────────│                          │
  │     { persona_model_id, version_id: v1 }             │
```

- 整段链路是**异步任务**，状态 `queued / running / succeeded / failed`
- 任意环节可中断，已成功环节的中间产物会保留以支持续跑

---

## 9. 接口

```http
POST   /v1/persona-models                              创建 PersonaModel（异步）
GET    /v1/persona-models/{id}                         查询
GET    /v1/persona-models/{id}/versions                历史版本
GET    /v1/persona-models/{id}/versions/{vid}         指定版本详情
PUT    /v1/persona-models/{id}/current-version        设置默认版本（不改版本本身）

POST   /v1/persona-models/{id}/avatars                创建 / 替换形象（异步）
GET    /v1/avatars/{aid}                               查询
DELETE /v1/avatars/{aid}                               删除（仅可删未绑定版本）

POST   /v1/persona-models/{id}/voices                  创建 / 替换声音（异步）
GET    /v1/voices/{vid}                                查询
POST   /v1/voices/{vid}/synthesize                     TTS 试听
DELETE /v1/voices/{vid}                                删除

POST   /v1/persona-models/{id}/persona                 创建 / 更新人设
GET    /v1/personas/{pid}                              查询
POST   /v1/personas/{pid}/simulate                     试运行

POST   /v1/persona-models/{id}/knowledge               绑定 / 替换知识语料（可选）
DELETE /v1/persona-models/{id}/knowledge               解绑
```

> 注意：形象 / 声音 / 人设 / 知识 都是**版本快照**。一旦新版本诞生，老版本的资产保持原样，不会被覆盖。

---

## 10. 异步任务状态

```json
{
  "task_id": "uuid",
  "type": "persona.create",
  "status": "queued | running | succeeded | failed",
  "progress": 0.0,
  "current_step": "voice_clone",
  "step_progress": { "avatar": 1.0, "voice": 0.4, "persona": 0.0 },
  "result": { "persona_model_id": "...", "version_id": "..." },
  "error": null
}
```

前端通过 WebSocket / 轮询 / Webhook 获取完成事件。

---

## 11. 错误与边界

| 场景 | 处理 |
|------|------|
| 参考图模糊 / 多张脸 | 拒绝，返回 `invalid_ref_image` |
| 声音样本含 BGM / 多说话人 | 拒绝，返回 `invalid_audio_sample` |
| 未提供声音样本 | 走商用音色，自动匹配 |
| LoRA 训练失败 | 重试 1 次 → 退回非微调路径 |
| 厂商限速 | 切到备选 Provider，记录埋点 |
| 形象 / 声音 / 人设 / 知识 不同时通过校验 | 创建失败，全链路回滚 |

---

## 12. 合规

- **形象授权**：上传参考图必须勾选"已获授权"
- **声音授权**：声音克隆必须上传"被克隆人授权书"（托管存证）
- **不可见水印**：默认烧录不可见水印用于追溯
- **真实人物复刻**：默认禁止；必须开启"真实人物"开关并通过更严格的合规审核

---

## 13. 关键指标

- 端到端创建 P50 ≤ 60s（不含知识），含知识 P95 ≤ 10min
- 形象一致性评分（CLIP / face embedding）≥ 0.85
- 声音相似度（speaker embedding cosine）≥ 0.80
- 人设试运行对话 P50 ≤ 2s

---

## 14. 上下游

- **上游**：集成方 API 调用；管理后台手动创建
- **下游**：
  - [persona-evolution.md](./persona-evolution.md)：在 v1 基础上持续训练
  - [video-generation.md](./video-generation.md)：指定 PersonaModel + version 出视频
