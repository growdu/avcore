# 模块设计：工作流编排（Pipeline）

> 把"训练人物角色模型"与"用模型出视频"两条链路都拆成可编排、可重试、可观测的 DAG 节点。Pipeline 不只服务于渲染，也服务于**持续演进**。

---

## 1. 目标

- **可编排**：训练 / 脚本 / 音频 / 画面 / 合成按 DAG 描述
- **可重试**：节点级失败重试
- **可恢复**：节点结果持久化，支持断点续跑
- **可观测**：每节点有 trace / log / metric
- **可扩展**：新增节点零侵入

---

## 2. 两条主要 DAG

### 2.1 视频生成 DAG（`video.generate.v1`）

```yaml
id: video.generate.v1
nodes:
  - id: script_gen
    type: llm
    input_from: job
    config:
      prompt_template: script_gen_v3
      persona: ${job.persona_version.persona_descriptor}
      knowledge: ${job.persona_version.knowledge}      # 可空
      topic: ${job.topic}

  - id: script_review
    type: gate
    input_from: script_gen
    when: ${options.require_human_review}
    on_approve: continue
    on_reject: abort

  - id: tts
    type: voice
    input_from: script_gen
    fanout: scene
    config:
      voice_id: ${job.persona_version.voice_id}
      ssml: true

  - id: bgm_select
    type: asset_search
    input_from: script_gen
    config:
      asset_type: bgm
      match_by: scene_emotion

  - id: img_gen
    type: avatar_image
    input_from: script_gen
    fanout: scene
    config:
      avatar_id: ${job.persona_version.avatar_id}

  - id: i2v
    type: video
    input_from: [tts, img_gen]
    fanout: scene
    depends_on: [tts, img_gen]
    config:
      provider: kling

  - id: lipsync
    type: lipsync
    input_from: i2v
    depends_on: i2v

  - id: compose
    type: composer
    input_from: [i2v, bgm_select]
    config:
      subtitle: ${options.enable_subtitle}
      watermark: ${options.enable_watermark}

  - id: finalize
    type: encode
    input_from: compose
    output: artifacts
```

> 关键：`job.persona_version` 是固化快照，避免渲染过程中 persona 演进造成不一致。

### 2.2 人物模型演进 DAG（`persona.train.v1`）

```yaml
id: persona.train.v1
nodes:
  - id: sample_filter
    type: sample_filter
    config:
      base_version_id: ${job.base_version_id}
      sample_ids: ${job.sample_ids}
      min_quality: 0.6
      embedding_dedupe: 0.92

  - id: avatar_train
    type: persona_train_avatar
    when: ${job.scope contains "avatar"}
    input_from: [sample_filter]
    config:
      base_avatar_id: ${job.base_version.avatar_id}
      epochs: ${job.config.epochs}
      anchors: ${job.config.anchors}

  - id: voice_train
    type: persona_train_voice
    when: ${job.scope contains "voice"}
    input_from: [sample_filter]
    config:
      base_voice_id: ${job.base_version.voice_id}
      epochs: ${job.config.epochs}

  - id: persona_sft
    type: persona_train_style
    when: ${job.scope contains "persona"}
    input_from: [sample_filter]
    config:
      base_persona_id: ${job.base_version.persona_descriptor}
      lr_scale: ${job.config.learning_rate_scale}

  - id: knowledge_reindex
    type: persona_train_knowledge
    when: ${job.scope contains "knowledge"}
    input_from: [sample_filter]
    config:
      corpus_ids: ${job.persona.knowledge.corpus_ids}

  - id: anchor_extract
    type: identity_anchor
    input_from: [avatar_train, voice_train, persona_sft]
    config:
      reference_version_id: ${job.base_version_id}

  - id: drift_eval
    type: consistency_eval
    input_from: [avatar_train, voice_train, persona_sft, anchor_extract]
    config:
      threshold: ${job.config.consistency_threshold}
      fallback_to_base: ${job.config.fallback_to_base}

  - id: publish_or_rollback
    type: branch
    input_from: drift_eval
    config:
      when_succeeded: publish_new_version
      when_failed: emit_drift_report + abort
```

> 该 DAG 跑完一条 `PersonaTrainingJob`，产出候选 `PersonaModelVersion`。

---

## 3. 节点类型

| 类型 | 描述 | 输入 | 输出 |
|------|------|------|------|
| `llm` | LLM 调用 | msgs | llm_response |
| `voice` | TTS 合成 | text, voice_id | audio + timestamps |
| `avatar_image` | 关键帧生成 | prompt, avatar_id | image |
| `video` | 图生视频 | image, audio, motion | clip |
| `lipsync` | 口型同步 | clip, audio | clip_synced |
| `asset_search` | 资产检索 | criteria | asset_ref |
| `composer` | 后期合成 | clips, bgm, opts | video |
| `encode` | 转码 | video | mp4 |
| `gate` | 人机协同 | prev_output | approval |
| `branch` | 条件分支 | prev_output | next_node_id |
| `http` | 通用 HTTP | url, payload | response |
| `persona_train_avatar` | 视觉微调 | samples, base_avatar_id | new_avatar |
| `persona_train_voice` | 声音微调 | samples, base_voice_id | new_voice |
| `persona_train_style` | 人设 SFT | samples, base_persona | new_persona |
| `persona_train_knowledge` | 知识索引 | corpus_ids | new_knowledge |
| `sample_filter` | 样本筛选 | samples, base_version | filtered_samples |
| `identity_anchor` | 锚点抽取 | new_assets | embeddings |
| `consistency_eval` | 漂移评估 | new_assets, base_version | consistency_report |
| `publish_or_rollback` | 发布决策 | consistency_report | new_version_id \| drift_report |

---

## 4. 节点执行器

```python
class NodeExecutor(Protocol):
    def execute(self, ctx: NodeContext) -> NodeResult: ...
    def resume(self, ctx: NodeContext, cached: NodeResult) -> NodeResult: ...

@dataclass
class NodeContext:
    job_id: str
    node_id: str
    inputs: dict[str, Any]
    config: dict
    trace: TraceSpan
    cancel_token: CancelToken

@dataclass
class NodeResult:
    outputs: dict[str, Any]
    artifacts: list[Artifact]
    metrics: dict
    next_hint: list[str] | None
```

---

## 5. 状态持久化

每个 Job 在数据库持久化：

```sql
CREATE TABLE job_steps (
    id           UUID PRIMARY KEY,
    job_id       UUID NOT NULL,
    node_id      TEXT NOT NULL,
    status       TEXT NOT NULL,        -- pending/running/succeeded/failed/skipped
    attempt      INT DEFAULT 1,
    inputs       JSONB,
    outputs      JSONB,
    artifacts    JSONB,
    error        JSONB,
    started_at   TIMESTAMPTZ,
    finished_at  TIMESTAMPTZ,
    duration_ms  INT,
    trace_id     TEXT
);
```

- 节点开始时：`status=running, started_at=now()`
- 节点成功：`status=succeeded, outputs=..., finished_at=now()`
- 节点失败：`status=failed, error=..., attempt++`
- 节点恢复：检查上一 attempt 的 `outputs` 是否可用

---

## 6. 重试与容错

### 6.1 重试策略
- 默认：3 次，指数退避（1s / 4s / 16s）
- 可针对节点类型覆盖（如 i2v 限速，重试 5 次）
- 死信：超过重试上限 → 标记 `failed` 并触发告警

### 6.2 降级
- 主 Provider 失败 → 切备 Provider
- 例如：`kling` 失败 → `cogvideox`
- 通过 Provider 注册 + 路由表实现

### 6.3 断点续跑
- 重启后扫描 `job_steps` 中 `succeeded` 的节点 → 直接复用 `outputs`
- 仅重跑 `pending / running / failed` 节点

---

## 7. 调度器

### 7.1 调度流程

```
入队 (queued)
   │
   ▼
资源检查 ──▶ 等待 GPU 配额 ──▶ 进入 running
   │
   ▼
取下一个 ready 节点（依赖已满足）
   │
   ▼
提交到 worker（按节点类型路由到 worker pool）
   │
   ▼
监听结果 ──▶ 推进 / 失败重试 / 完成
```

### 7.2 调度策略
- **FIFO**：默认
- **优先级**：VIP 租户优先
- **抢占**：高优任务可抢占低优 worker（K8s 配合）
- **公平**：每租户最少 1 worker 槽位
- **训练任务独占**：同一 persona 不并发训练

### 7.3 Worker 路由

| 节点类型 | Worker 池 |
|----------|----------|
| `llm` | llm-pool（CPU） |
| `voice` | tts-pool（GPU/CPU） |
| `avatar_image` | img-pool（GPU） |
| `video` | video-pool（GPU） |
| `lipsync` | lipsync-pool（GPU） |
| `persona_train_avatar` | train-pool（GPU） |
| `persona_train_voice` | train-pool（GPU） |
| `persona_train_style` | llm-pool（CPU/GPU） |
| `consistency_eval` | eval-pool（GPU） |
| `compose / encode` | compose-pool（CPU） |

---

## 8. 可观测性

每个节点自动注入：

- **Trace Span**：`job_id.node_id.attempt`
- **Metrics**：`node_duration_seconds{node, status}`, `node_failure_total{node, reason}`
- **Logs**：结构化 JSON，含 trace_id、tenant_id、job_id、persona_model_id
- **事件**：`node.succeeded` / `node.failed` 入事件流

集成 OpenTelemetry，可导出到 Jaeger / Tempo。

---

## 9. 引擎实现建议

### 9.1 起步（自研）
- DAG 解析：JSON Schema 校验
- 调度器：基于 Redis 的轻量队列
- 状态：PostgreSQL
- 估算：千级并发任务内可承载

### 9.2 演进（Temporal / Argo）
- 当并发 > 1 万或跨服务编排复杂时迁移
- 迁移时仅替换引擎实现，DAG 描述保持兼容

---

## 10. 关键指标

- 调度延迟：节点 ready → 提交 worker ≤ 500ms（P95）
- 节点成功率：≥ 98%
- 端到端成功率：≥ 95%
- 训练 DAG 单次 ≤ 30 min（P95，视觉 / 声音）
- 渲染 DAG 单次 ≤ 8 min（P95，60s 成片）

---

## 11. 上下游

- **被调用方**：[persona-modeling.md](./persona-modeling.md)、[persona-evolution.md](./persona-evolution.md)、[video-generation.md](./video-generation.md)
- **基础设施依赖**：对象存储、向量库、消息队列、PostgreSQL、Redis
