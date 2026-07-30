# 模块设计：人物角色模型完善演进（Persona Evolution）

> **核心模块**。在 v1 之上，**追加样本 → 训练 → 出新版本（v2 / v3 / ...）→ 一致性兜底**。这是 AVCore 与"一次性造角"框架最大的差别。
>
> 本文档回答：**怎么补充角色、怎么演进、怎么保证不漂移**。落盘约定见 [`../storage.md`](../storage.md)。

---

## 1. 模块目标

**输入**：已存在的 `PersonaModel` + 新的训练样本（图像 / 音频 / 行为文本 / 用户反馈）

**输出**：`PersonaModelVersion` v(N+1)——**新目录**，含：
- 资产快照（avatar / voice / persona / knowledge）**全部从零拷贝生成**
- 新的 `IdentityAnchor`（与父版本对齐的锚点特征）
- 新的 `VersionMetrics`
- 与父版本关联（`parent_version_id`）

**边界**：
- 不做"对话态"短期记忆（那是对话产品的事）
- 与视频渲染解耦——可以独立被调用

---

## 2. 为什么必须持续训练

现实里没人会"一次性"就完美塑形一个角色：
- 数据是慢慢累积的（一年后才有 10000 段口播、50 套照片）
- 风格要进化（完播率高的口头禅要强化）
- 场景在扩张（从"讲解"扩到"情感陪伴"）
- 要修正过去的翻车点
- 要留住观众熟悉的样子（同时又不能完全不变）

平台必须承担"如何不漂移"的责任，而不是让用户自己解决。

> **演进 ≠ 本地推理**。训练 / 微调都在远端 Provider 上完成，AVCore 只负责：收集样本 → 打包数据集 → 调用 Provider SFT 端点 → 等结果 → 拿到新 `model_id` → 建立新版本目录（只存产物，不存权重）。

---

## 3. 样本体系

样本分四类，落地后**全部进 SQLite 的 `persona_samples` 表**：

| `kind` | 内容 | 形态 | 主要场景 |
|--------|------|------|----------|
| `image` | 参考图 / 形象照 | 文件 URI | 视觉微调 |
| `audio` | 声音样本 | 文件 URI + 文本 + 时长 | 声音克隆 / 微调 |
| `behavior_text` | 行为样例（语气 / 对白） | 纯文本 | 人设 SFT |
| `feedback` | 反馈信号 | 文本 / label / weight | 反馈闭环 |

每条样本必须：
- 带 `consent_proof`（形象 / 声音必填）
- 通过质量打分（模糊 / 多脸 / BGM 拒收）
- 唯一 key：`version_id_at_collection`（指出自哪个版本；防止自我循环）

---

## 4. 演进维度

一次训练任务可同时改多维度，也可只改一个：

### 4.1 视觉（avatar）
- 新参考图 → 调用 Provider 的 SFT/fine-tune 端点（提交样本 + base model_id）→ 远端训练
- 角度修正：发现"侧脸不像"→ 用新角度图专项远端训练
- 风格切换："写实"换"皮克斯风"→ 在 Provider 上重新训练视觉模型
- AVCore 不下载 LoRA 权重，只存 `lora/ref.json`（`model_id / provider / trained_at / base_model`）

### 4.2 声音（voice）
- 新声音样本 → 调用 Provider 的 voice clone / SFT 端点 → 远端训练
- 情绪校正：发现某情绪总翻车 → 该情绪专项样本 + Provider emotion-tuning 端点
- 多语扩展：中文 → 加英粤 → Provider 多语训练端点
- AVCore 仅持有更新后的 `voice_id / model_id`

### 4.3 人设（persona）
- 行为样本追加 → 调用兼容 OpenAI SFT 的 LLM Provider（提交 JSONL 数据集）→ 远端 SFT
- 口头禅萃取：高完播率回答 → 固化为 catchphrase（写入 `persona.json`，不需要训练）
- 禁忌收紧：出过几次错的话题 → 写入 taboo（写入 `persona.json`，不需要训练）
- 训练后的模型 `ft_model_id` 由 Provider 返回；AVCore 仅存引用

### 4.4 知识（knowledge）
- 追加语料 → 调用远端 embed API 重建向量索引；本地可选缓存 chunks + 向量以加速检索
- 知识替换：错误语料标 `deprecated`（不删，检索时权重置零）
- 领域切换：v2 法律 / v3 医学可整体换 corpus
- Embedding 来自远端 API；本地不持有"模型"，仅持有特征数据

---

## 5. 训练任务的数据契约

```rust
struct TrainingJob {
    id: String,                       // tj_xxx
    persona_model_id: String,
    base_version: u32,                // 从哪个版本开始
    target_version: u32,              // 默认 base+1（创建时分配，写入失败就释放）
    scope: Vec<Scope>,                // [avatar, voice, persona, knowledge]
    sample_ids: Vec<String>,
    config: TrainingConfig,
    status: Status,                   // queued/running/succeeded/failed_drift/cancelled
    progress: f32,
    result_version: Option<u32>,      // 成功时填入新版本号
    drift_report: Option<DriftReport>,
    created_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
}

struct TrainingConfig {
    full_retrain: bool,               // 默认 false（增量）
    epochs: u32,
    learning_rate_scale: f32,
    eval_set_ids: Vec<String>,
    consistency_threshold: f32,       // 默认 0.85
    anchors: Vec<String>,             // 金丝雀样本 IDs
    fallback_to_base: bool,           // 漂移过大自动回退
    keep_partials: bool,              // 失败时是否保留中间产物供调试
}
```

---

## 6. 训练流水线（DAG 节点）

> 所有训练节点都通过 HTTP 调用 Provider 的 SFT/fine-tune 端点；AVCore 只提交样本 + base model_id，等待远端返回新 model_id。

```
[1] sample_filter       ─ 关联 base_version，质量过滤，去重（embedding cosine < 0.92 丢弃）
        │
        ▼
[2] avatar_train (if)   ─ POST { samples, base_avatar_ref } → Provider SFT → 返回 { new_avatar_ref }
[3] voice_train (if)    ─ POST { samples, base_voice_ref  } → Provider SFT → 返回 { new_voice_ref  }
[4] persona_sft (if)    ─ POST { jsonl_dataset, base_model } → Provider SFT → 返回 { new_model_id }
[5] knowledge_reindex   ─ POST { chunks } → Provider embed → 返回 { vectors }（本地可缓存）
        │
        ▼
[6] identity_anchor_extract  ─ 调用 Provider 的特征抽取 API（face / voice / style embeddings）
        │
        ▼
[7] drift_eval          ─ face/voice/style cosine vs 父版本
        │
        ├─ 达标 (≥ threshold) ─▶ publish v(N+1)
        └─ 不达标            ─▶ fallback_to_base + drift_report
```

训练耗时的实际来源是 Provider 端的排队与计算；AVCore 通过 `task_xxx --watch` 长轮询 Provider 任务状态。

每个节点是 DAG 中的独立步，节点结果**全部落 v(N+1) 目录**。这就是为什么 v(N+1) 必须是新目录——任何中间产物都能定位。

---

## 7. 版本管理（不可变性）

```rust
struct VersionMetrics {
    identity_consistency: f32,        // 与父版本的 cos（face/voice/style 均值）
    style_consistency: f32,
    quality_score: f32,
    drift_alerts: Vec<DriftAlert>,    // ["avatar_drift_angle=side", ...]
    notes: String,
}
```

### 7.1 不变性原则
- **版本号自增、不可跳号、不重用**
- 一旦创建，目录内任何文件**永不被修改**（sha256 校验）
- 改版只能新建版本；停止只能用 deprecated 标记

### 7.2 `current_version` 与历史
- `PersonaModel.current_version` 是**指针**，可改
- 改指针只影响"之后新任务默认用哪个版本"
- 已渲染的视频绑的是**当时**的 version，不受影响

### 7.3 切版本 / A/B / 回滚
```bash
avc persona current yu --set 3               # 单一默认
avc persona ab yu --versions 2,3 --ratio 70/30   # A/B 灰度
avc persona current yu --set 1               # 一行回滚
```
回滚 = 指针回拨，不删任何数据。

---

## 8. 一致性与漂移检测

### 8.1 Identity Anchor
- 训练结束时抽取 face / voice / style embedding，落 `identity_anchor.json`
- 与父版本 anchor 算 cosine → `identity_consistency`
- canary 样本：用户标记"必须不漂移"的样本，分布在 `samples/canary/`；评估时强制跑

### 8.2 漂移评估
- 自动：金丝雀样本 + 评测集；face/voice embedding 余弦相似度；style 由 LLM-as-Judge 评分
- 人工抽检：自动通过后 90% 通过率才发布
- 不达标 → fallback_to_base，训练任务 `status=failed_drift`

### 8.3 漂移告警（不阻断发布）
- 漂移分项超阈值（如侧脸 cos < 0.7）单独告警，入事件流
- 运营可针对单维度再补样本、再训练
- 这些告警会写入 `VersionMetrics.drift_alerts`

---

## 9. CLI 操作流程

```bash
# 1. 追加样本
avc persona sample add yu \
  --kind audio \
  --uri ./new_voice.wav \
  --duration-ms 60000 \
  --text "..." \
  --consent ./auth.pdf

avc persona sample add yu \
  --kind image \
  --uri ./new_view.png \
  --tags side,neutral \
  --consent ./auth.pdf

# 2. 启动训练
avc persona evolve yu \
  --scope avatar,voice,persona \
  --base-version 2 \
  --anchors ./samples/canary/ \
  --consistency-threshold 0.85 \
  --fallback-to-base

# 3. 跟任务
avc task show task_xxx --watch

# 4. 看报告
avc training report task_xxx --json
```

报告 JSON 例子：

```json
{
  "persona_model_id": "pm_01H...",
  "base_version": 2,
  "candidate_version": 3,
  "metrics": {
    "identity_consistency": 0.92,
    "style_consistency": 0.88,
    "quality_score": 0.84
  },
  "per_dim_drift": {
    "avatar": {"score": 0.92, "warning": null},
    "voice":  {"score": 0.91, "warning": null},
    "style":  {"score": 0.88, "warning": "tone_more_formal_than_parent"}
  },
  "samples_used": 120,
  "duration_min": 38,
  "decision": "publish"   // 或 "rollback"
}
```

---

## 10. 样本治理

```bash
avc persona sample list yu --kind audio
avc persona sample rm sample_01H...
avc persona sample consign sample_01H...    # 标金丝雀（必须不漂移）
avc persona sample stats yu               # 数量 / 质量 / 标签分布
```

入库前必跑校验：
1. `consent_proof` 文件存在 + hash 与声明一致
2. 质检：image → CLIP 清晰度 + face detection；audio → SNR + VAD
3. 与已有样本 embedding cosine ≥ 0.92 才保留（去重）

---

## 11. Provider 实现（全部 token 鉴权的商业 / 开源 API）

| 维度 | Provider（规划） |
|------|----------------|
| 视觉 | `kling_avatar_finetune`, `heygen_avatar_finetune`, `replicate_flux_lora_trainer`, `doubao_image_finetune`, `seededit_finetune` |
| 声音 | `elevenlabs_voice_clone`, `azure_speech_personal_voice`, `doubao_voice_finetune`, `openai_tts_finetune` |
| 人设 | `openai_compat_sft`（兼容 OpenAI / Anthropic / DeepSeek / 智谱 / 豆包 SFT 端点） |
| 知识 | `openai_embed`, `volcengine_embed`, `alibaba_embed`, `cohere_embed`（均为远端 API） |

切换 = 改 `provider.json` 的 `auth.endpoint` 字段 + 替换 trait 实现，主仓不动。

> 每个 Provider 必须有 `api_key`（或兼容 token）；无 `api_key` 配置时 `avc` 会拒绝调用并提示 `avc config set provider.<name>.api_key ...`。

---

## 12. 调度与资源

- 实际计算发生在 Provider 端，AVCore 端是轻量编排
- Phase 0：直接在主进程用 tokio task 长轮询 Provider 任务状态
- Phase 1：可在 Provider 端为高频训练任务预留 "scheduled slot"（由 Provider 配额决定）
- 单 PersonaModel **不并发训练**（防版本冲突）
- 多 PersonaModel 可并行；按 Provider 限速 (rate limit) 公平分享
- 失败也记 cost（按 Provider 返回的 `usage` 字段），防止刷调用
- **不涉及本地 GPU / CPU 显存**——选型不要混淆 "Provider 端排队" 与 "本地推理"

---

## 13. 反馈回灌

```
$ avc job feedback job_xxx --signal looks_unlike --note "侧脸不像本人"
        │
        ▼
转 PersonaSample(kind=feedback, weight=1.0) 写入 samples 表
        │
        ▼
下次 evolve 自动消费（除非用户标记 ignore=false）
```

为什么是默认自动消费？
- 反馈是角色演进的**主要燃料**
- 手动开关 `avc config set evolution.auto_consume_feedback true|false`

---

## 14. 关键指标

- 训练耗时：单维度 P95 ≤ 30 min（视觉 / 声音），人设 P95 ≤ 2h
- 跨版本一致性 ≥ 0.85（与父版本）
- 训练成功率 ≥ 95%
- 反馈 → 样本 → 新版本闭环 P95 ≤ 24h
- 老版本可继续被引用 ≥ 24 个月

---

## 15. 上下游

- **上游**：[persona-modeling.md](./persona-modeling.md)（v1 起点）、[video-generation.md](./video-generation.md)（反馈回灌）
- **下游**：[video-generation.md](./video-generation.md)（锁定 version 渲染）
