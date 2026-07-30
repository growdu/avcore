# 模块设计：人物角色模型完善演进（Persona Evolution）

> 模型不是"造一次就定型"——它需要**持续训练**。本模块负责在 v1 之后追加样本、微调、产出新版本，并保证跨版本身份**不漂移**。

> 这是 AVCore 与"一次性造角"框架最大的差别：把 persona 当成**会慢慢变好、但不能变得认不出来**的活模型。

---

## 1. 模块目标

**输入**：已存在的 `PersonaModel` + 新的训练样本（图像 / 音频 / 行为文本 / 用户反馈）

**输出**：`PersonaModel` 的新版本（v2 / v3 / ...），含：
- 资产快照（avatar / voice / persona / knowledge）均**不可变**
- 新的 `IdentityAnchor`（与上一版本对齐的锚点特征）
- 新的 `VersionMetrics`（一致性 / 风格 / 质量分）
- 与上一版本的关联（`parent_version_id`）

**边界**：
- 本模块只做"训练态"的演进，不做"对话态"的短期记忆
- 本模块独立于视频生成，可独立被调用（持续运营一个模型是常态）

---

## 2. 为什么必须持续训练

真实场景里没人会"一次性"就完美塑形一个角色：

- **数据是慢慢累积的**——视频号运营 1 年后才有 10000 段口播、50 套形象照
- **风格要进化**——新人设发现"用某句口头禅完播率最高"，下次训练就要强化它
- **场景在扩张**——从"知识讲解"扩到"情感陪伴"，就需要新风格微调
- **要修正过去**——v1 翻车了（太严肃 / 不像本人），必须能修正
- **要留住观众熟悉的样子**——同时又不能完全不变

因此演进是默认动作，不是进阶玩法。平台要承担"如何不漂移"的责任，而不是让用户自己解决。

---

## 3. 训练样本体系

```python
@dataclass
class PersonaSample:
    id: str
    persona_model_id: str
    kind: str                       # image / audio / behavior_text / feedback
    uri: URI | None                 # 媒体类样本（image/audio）
    text: str | None                # 文本类样本（行为 / 用户反馈）
    source: str                     # user_upload / system_extracted / feedback_pool
    version_id_at_collection: str | None
    consent_proof: str | None       # 授权存证 ID
    tags: list[str]                 # 训练用标签：neutral / angry / teach / ...
    quality_score: float | None     # 质检分数（可选）
    created_at: datetime

@dataclass
class SampleFeedback:               # 视频生成过程中收集
    job_id: str
    persona_model_id: str
    version_id: str
    signal: str                     # "wrong_voice" / "wrong_style" / "looks_unlike" / "thumbs_up"
    note: str | None
    weight: float = 1.0
    created_at: datetime
```

### 3.1 样本来源
| 来源 | 样本类型 | 说明 |
|------|----------|------|
| 用户主动上传 | image / audio / behavior_text | 平时添加训练素材 |
| 视频生成回灌 | feedback | 用户对成片点"不像 / 不对"，转成可训练信号 |
| 对话回灌 | behavior_text | 把多轮对话中被采纳的回答提炼成"角色风格样本" |
| 运营手动标注 | image / audio | 标"这一张很标准"或"这一张很像本人"，用于筛选重训样本 |

### 3.2 样本治理
- 每个样本入库前必须通过：
  - **授权存证**（形象 / 声音必须）
  - **质量打分**（模糊 / 多脸 / 含 BGM 拒收）
  - **去重**（embedding cosine < 0.92 直接丢弃）
- 样本与 `version_id_at_collection` 绑定，便于追溯"哪个版本生成的内容又被喂回去了"

---

## 4. 演进维度

一次训练任务可以同时改多维度，也可以只改一个：

### 4.1 视觉维度
- 追加参考图 → 再 LoRA 微调 → 视觉一致性进一步收敛
- 视觉漂移修正：发现某些角度不像 → 用新角度图专项训练
- 风格微调：从"写实"换到"皮克斯风"需要重新训练视觉资产

### 4.2 声音维度
- 追加声音样本 → TTS 模型增量训练 → 音质 / 情绪覆盖更全
- 情绪校正：发现某情绪总翻车 → 补充该情绪样本专项训练
- 多语扩展：从中文扩到英粤 → 加入多语种样本训练

### 4.3 人设维度
- 行为样本追加 → 微调对话风格 / system prompt
- 口头禅强化：高频优质对话 → 萃取为 catchphrase
- 禁忌收紧：出过几次错的话题 → 写入 taboo

### 4.4 知识维度
- 追加语料 → 重新索引 → 检索效果提升
- 知识替换：发现旧语料错误 → 标记 `deprecated` 不删，但权重置零
- 领域切换：v2 是"法律专家"、v3 是"医学助手"，可以彻底换 corpus

---

## 5. 训练任务

### 5.1 数据契约

```python
@dataclass
class PersonaTrainingJob:
    id: str
    persona_model_id: str
    base_version_id: str            # 基于哪个版本训练
    target_version: int             # 默认 base+1
    scope: list[str]                # ["avatar", "voice", "persona", "knowledge"]
    sample_ids: list[str]
    config: TrainingConfig
    status: str                     # queued / running / succeeded / failed / cancelled
    progress: float = 0.0
    current_step: str | None
    result_version_id: str | None
    metrics: dict | None
    created_at: datetime
    finished_at: datetime | None

@dataclass
class TrainingConfig:
    # 维度开关
    train_avatar: bool = True
    train_voice: bool = True
    train_persona: bool = True
    train_knowledge: bool = False
    # 训练策略
    full_retrain: bool = False      # True = 全量；False = 增量
    learning_rate_scale: float = 0.1
    epochs: int = 3
    eval_set_ids: list[str] = []    # 评测样本集
    # 一致性约束（与 base_version 对齐）
    consistency_threshold: float = 0.85  # 低于此值视作漂移
    anchors: list[str] = []         # 强制保留的样本（不允许漂移的"金丝雀"样本）
    # 兜底
    fallback_to_base: bool = True   # 漂移过大时自动回退到 base 版本
```

### 5.2 训练流水线（DAG 节点）

```
[1] 样本筛选 ─ 关联 base_version，质量过滤，去重
        │
        ▼
[2] 视觉训练 (可选) ─ LoRA / InstantID 微调
        │
        ▼
[3] 声音训练 (可选) ─ CosyVoice / SoVITS / F5 增量
        │
        ▼
[4] 人设训练 (可选) ─ SFT + 偏好对齐
        │
        ▼
[5] 知识重建 (可选) ─ embed + 索引重建
        │
        ▼
[6] Identity Anchor 抽取 ─ 与 base 对齐
        │
        ▼
[7] 漂移评估 ─ 仿冒度 / 风格度 / 知识度
        │
        ▼
[8] 决策 ─ 达标 → 发布新版本；不达标 → 退回 base + 报告
```

### 5.3 与上游样本采集解耦
- 训练任务只消费**已入库的样本**
- 样本采集（视频反馈回灌 / 用户上传）独立运行，避免训练任务被杂数据污染

---

## 6. 版本管理

```python
@dataclass
class VersionMetrics:
    identity_consistency: float      # vs 父版本：face / voice / style embedding cosine 均值
    style_consistency: float         # LLM-as-Judge：风格保持
    quality_score: float             # 人工 / 模型打分
    drift_alerts: list[str]          # ["avatar_drift_angle=side", "voice_drift_emotion=sad"]
    notes: str
```

### 6.1 版本策略
- 版本号自增，不可跳号，不重用
- 老版本保留：用户视频生成链接必须能定位到具体版本，因此版本是**不可变快照**
- 默认版本（`PersonaModel.current_version`）可改，改了不影响下游已生成的视频

### 6.2 版本选择
- 视频生成请求可指定 `version_id`：不指定 = 当前默认版本
- 支持 A/B：A/B 两版本并行可用租户，灰度比例按 config 控制

### 6.3 回滚
- 不达标的新版本标记 `deprecated`，不再被自动选中
- 也可以**强制回滚**：把 `current_version` 指回旧版本，新任务走旧版本
- 历史视频不受影响（它们绑定的是冻结的 version_id）

---

## 7. 一致性与漂移检测

### 7.1 Identity Anchor
- 训练结束时抽取：face_embedding / voice_embedding / style_embedding
- 存到 `PersonaModelVersion.identity_anchor`
- 与父版本 anchor 算 cosine，作为 `identity_consistency` 主要指标

### 7.2 漂移评估
- 自动化：
  - 把"金丝雀样例"（`config.anchors`）跑一遍新旧版本，按相同 prompt 出图 / 出音
  - face / voice embedding 余弦相似度
  - 风格相似度（用 LLM-as-Judge + 提示词模板）
- 人工抽检：
  - 自动评估通过后，人工抽检通过率需 ≥ 90%
  - 不达标 → 走 `fallback_to_base`

### 7.3 漂移告警
- 漂移分项超阈值时单独告警，例如：
  - `avatar_drift_angle=side`：侧脸最不像
  - `voice_drift_emotion=sad`：悲伤情绪最不像
- 告警进入事件流，运营可选择定向补样本再训练

---

## 8. 接口

```http
# 样本
POST   /v1/persona-models/{id}/samples                  提交样本
GET    /v1/persona-models/{id}/samples                  列出样本
DELETE /v1/persona-samples/{sid}                        移除样本
POST   /v1/persona-samples/{sid}/consign                标记为"金丝雀样本"

# 训练
POST   /v1/persona-models/{id}/training-jobs            创建训练任务
GET    /v1/training-jobs/{jid}                          查询
POST   /v1/training-jobs/{jid}/cancel                   取消
POST   /v1/training-jobs/{jid}/resume                   续跑
GET    /v1/training-jobs/{jid}/report                   训练报告（含漂移 / 一致性）

# 版本
PUT    /v1/persona-models/{id}/current-version          设置默认版本
POST   /v1/persona-models/{id}/versions/{vid}/deprecated 停用版本（不在新生成任务中默认使用）
POST   /v1/persona-models/{id}/ab                        开启 A/B 流量分配

# 反馈回灌（用于自动生成样本）
POST   /v1/jobs/{jid}/feedback                          用户对成片反馈（产生 PersonaSample）
GET    /v1/persona-models/{id}/feedback-pool            反馈聚合
```

---

## 9. Provider 适配

| 维度 | Provider | 说明 |
|------|----------|------|
| 视觉 | `sdxl_lora_incremental` | 增量 LoRA |
| 视觉 | `kling_avatar_finetune` | 商用微调 |
| 声音 | `cosyvoice_incremental` | 自托管增量 |
| 声音 | `gpt_sovits_incremental` | 少样本增量 |
| 人设 | `llm_sft` | OpenAI / Anthropic / 国产大模型 SFT |
| 知识 | `embed_reindex` | 重 embedding + 索引重建 |

切换 Provider 不影响 PersonaModel 抽象。

---

## 10. 调度与成本

- 训练任务是 **GPU 重任务**，独立 worker pool：`train-pool`
- 单 PersonaModel 不并发训练（防版本错乱），但多 PersonaModel 可并行
- 估算代价（在 UI 上展示给用户）：
  - 训练前给预估 GPU 时长、token 数、预计费用
  - 失败也要计费（防止恶意刷训练）

---

## 11. 与视频生成的关系

```
PersonaModel (v3) ─── 锁定 version ───▶ Video Job
   │  current_version = v3
   │
   └─ 历史视频仍绑定自己生成时的 v1 / v2，不会跟随 default 漂移到 v3
```

这就是版本不变的意义：观众的旧视频永远长得一样，运营侧继续做 v4、v5 训练都不会破坏存量体验。

---

## 12. 关键指标

- 训练耗时：单维度训练 P95 ≤ 30 min（视觉 / 声音），人设 P95 ≤ 2h
- 跨版本一致性 ≥ 0.85（与父版本）
- 训练成功率 ≥ 95%
- 反馈 → 样本 → 新版本闭环时延 ≤ 24h（P95）
- 老版本可被继续调用 ≥ 24 个月（合规 / 复盘需要）

---

## 13. 上下游

- **上游**：[persona-modeling.md](./persona-modeling.md)（v1 起点）、[video-generation.md](./video-generation.md)（反馈回灌）
- **下游**：[video-generation.md](./video-generation.md)（锁定 version 渲染）
