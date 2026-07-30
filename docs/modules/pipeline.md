# 模块设计：工作流编排（Pipeline）

> 把"训练 PersonaModel"与"用 PersonaModel 出视频"两条链路都拆成可编排、可重试、可观测的 DAG 节点。Pipeline 不只服务于渲染，也服务于**持续演进**。

---

## 1. 目标

- **可编排**：训练 / 脚本 / 音频 / 画面 / 合成按 DAG 描述
- **可重试**：节点级失败重试 + 自动回退
- **可恢复**：节点结果落盘到版本目录，断点续跑
- **可观测**：每节点有 trace / log / metric
- **可扩展**：新增节点零侵入

---

## 2. 两条主要 DAG

### 2.1 视频渲染 DAG（`video.render.v1`）

```yaml
id: video.render.v1
nodes:
  - id: script_gen
    type: llm
    config:
      prompt_template: script_gen_v3
      persona_descriptor: ${job.persona_version.persona_descriptor}
      knowledge: ${job.persona_version.knowledge}
      topic: ${job.topic}

  - id: script_review
    type: gate
    when: ${options.require_human_review}

  - id: tts
    type: voice
    fanout: scene
    config:
      voice_id: ${job.persona_version.voice.voice_id}

  - id: bgm_select
    type: asset_search
    config:
      match_by: scene_emotion

  - id: img_gen
    type: avatar_image
    fanout: scene
    config:
      avatar_dir: ${job.persona_version.avatar_dir}

  - id: i2v
    type: video
    fanout: scene
    depends_on: [tts, img_gen]
    config:
      provider: kling

  - id: lipsync
    type: lipsync
    depends_on: i2v

  - id: compose
    type: composer
    input_from: [i2v, bgm_select]

  - id: finalize
    type: encode
    output_to: ${job.media_dir}
```

> 关键：`job.persona_version` 是固化目录指针，避免渲染过程 persona 演进造成不一致。

### 2.2 人物模型演进 DAG（`persona.train.v1`）

```yaml
id: persona.train.v1
nodes:
  - id: sample_filter
    type: sample_filter
    config:
      base_version: ${job.base_version}
      min_quality: 0.6
      dedup_threshold: 0.92

  - id: avatar_train
    type: persona_train_avatar
    when: ${job.scope contains avatar}
    config:
      base_avatar_dir: ${job.base_version.dir}/avatar
      epochs: ${job.config.epochs}

  - id: voice_train
    type: persona_train_voice
    when: ${job.scope contains voice}
    config:
      base_voice_dir: ${job.base_version.dir}/voice

  - id: persona_sft
    type: persona_train_style
    when: ${job.scope contains persona}

  - id: knowledge_reindex
    type: persona_train_knowledge
    when: ${job.scope contains knowledge}

  - id: anchor_extract
    type: identity_anchor
    input_from: [avatar_train, voice_train, persona_sft]

  - id: drift_eval
    type: consistency_eval
    input_from: [avatar_train, voice_train, persona_sft, anchor_extract]
    config:
      threshold: ${job.config.consistency_threshold}

  - id: publish_or_rollback
    type: branch
    config:
      on_pass: publish_new_version(${job.target_version})
      on_fail: emit_drift_report + clear(${job.target_version}_dir)
```

> 训练跑完，产出 `personas/pm_xxx/v(N+1)/` 完整目录；不达标时整目录直接清掉（除非 `keep_partials=true`）。

---

## 3. 节点类型

| 类型 | 描述 | 用于 |
|------|------|------|
| `llm` | LLM 调用（chat / SFT） | 脚本生成 |
| `voice` | TTS 合成 | 渲染 |
| `avatar_image` | 关键帧生成 | 渲染 |
| `video` | 图生视频 | 渲染 |
| `lipsync` | 口型同步 | 渲染 |
| `asset_search` | 资产检索 | 渲染 |
| `composer` | 后期合成 | 渲染 |
| `encode` | 转封装 | 渲染 |
| `gate` | 人机协同 | 渲染（可选） |
| `branch` | 条件分支 | 通 |
| `http` | 通用 HTTP | 通 |
| `sample_filter` | 样本筛选 | 训练 |
| `persona_train_avatar` | 视觉微调 | 训练 |
| `persona_train_voice` | 声音微调 | 训练 |
| `persona_train_style` | 人设 SFT | 训练 |
| `persona_train_knowledge` | 知识索引重建 | 训练 |
| `identity_anchor` | 锚点抽取 | 训练 |
| `consistency_eval` | 漂移评估 | 训练 |
| `publish_or_rollback` | 发布决策 | 训练 |

---

## 4. 节点执行器

```rust
#[async_trait]
trait NodeExecutor: Send + Sync {
    fn kind(&self) -> &'static str;
    async fn execute(&self, ctx: &NodeContext) -> Result<NodeResult>;
    async fn resume(&self, ctx: &NodeContext, cached: &NodeResult) -> Result<NodeResult>;
}

struct NodeContext {
    job_id: String,
    node_id: String,
    inputs: Value,
    config: Value,
    cancel: CancellationToken,
}

struct NodeResult {
    outputs: Value,
    artifacts: Vec<ArtifactRef>,     // 文件路径（相对 PersonaVersion.dir 或 media dir）
    metrics: Value,
    next_hint: Option<Vec<String>>,
}
```

---

## 5. 状态持久化

每个 Job 落 SQLite `job_steps` 表：

```sql
CREATE TABLE job_steps (
    id           TEXT PRIMARY KEY,
    job_id       TEXT NOT NULL,
    node_id      TEXT NOT NULL,
    status       TEXT NOT NULL,    -- pending/running/succeeded/failed/skipped
    attempt      INT DEFAULT 1,
    inputs_json  TEXT,
    outputs_json TEXT,
    artifacts_json TEXT,
    error_json   TEXT,
    started_at   TEXT,
    finished_at  TEXT,
    duration_ms  INT,
    trace_id     TEXT
);
```

> 渲染的中间产物按惯例写到 `~/.local/share/avc/cache/jobs/{job_id}/`；最终成功才落到 `media/jobs/{job_id}/`。  
> 训练中间产物直接写到**新版本目录**（`personas/pm_xxx/v(N+1)/...`）；不达标清掉整个目录。

---

## 6. 重试与容错

### 6.1 重试
- 默认 3 次，指数退避 1s / 4s / 16s
- 节点级可覆盖（如 i2v 限速可设 5 次）
- 超过阈值标 `failed`，等待用户 `retry` 或自动回退（训练 DAG）

### 6.2 降级
- 主 Provider 失败 → 切备选（注册到 `Model Gateway`）
- 例：`kling` 失败 → `cogvideox`

### 6.3 续跑
- 进程重启扫描 `job_steps`，把 `running → pending`，复用 `succeeded` 节点的 outputs
- 渲染 DAG 中 `succeeded` 的中间产物已在 `cache/` 目录，路径写在 `artifacts_json`

---

## 7. 调度器

### 7.1 流程
```
enqueue
  ▼
ready 节点（依赖满足）
  ▼
提交 worker pool（按 kind 路由）
  ▼
完成 → 推进 / 失败重试 / 完成
```

### 7.2 策略
- 内存优先；Phase 1 引入 Redis/Kafka 队列（如需多机）
- 训练任务独占（同 persona 不并发）
- 多 persona 并行；按用户公平

### 7.3 资源池（Phase 1 引入）
| 节点 | 池 |
|------|----|
| `llm`, `persona_train_style` | llm-pool |
| `voice`, `persona_train_voice` | tts-pool |
| `avatar_image`, `persona_train_avatar` | img-pool |
| `video` | video-pool |
| `lipsync` | lipsync-pool |
| `consistency_eval`, `identity_anchor` | eval-pool |
| `compose`, `encode` | compose-pool |

> Phase 0 在主进程内用 tokio task 跑就够了。

---

## 8. 可观测性

每个节点自动注入：
- **结构化日志**：`tracing` JSON，字段含 `trace_id` / `tenant_id` / `persona_model_id` / `job_id`
- **可选 OTel**：通过 `tracing-opentelemetry` 导出（不默认开）
- **事件流**：`node.succeeded` / `node.failed` 写 SQLite 事件表，外部 collect

> 不做 dashboard。日志由用户导到自己的 Loki / ES。

---

## 9. 引擎实现

### 9.1 起步（自研）
- DAG：JSON / YAML 解析为内部 IR
- 调度：tokio task + in-memory state
- 估算：单用户千级并发内可行

### 9.2 演进（可选）
- 当并发 > 1 万或需要跨进程时，引 Temporal / 自研 Redis 队列
- 替换只动调度层，DAG 描述不变

---

## 10. 关键指标

- 调度延迟：节点 ready → 提交 worker ≤ 500 ms（P95）
- 节点成功率 ≥ 98%
- 端到端成功率 ≥ 95%
- 训练 DAG：单维度 P95 ≤ 30 min
- 渲染 DAG：60s 成片 P95 ≤ 8 min

---

## 11. 上下游

- **被调用方**：[persona-modeling.md](./persona-modeling.md)、[persona-evolution.md](./persona-evolution.md)、[video-generation.md](./video-generation.md)
- **基础设施**：SQLite、文件缓存、Provider HTTP 客户端
