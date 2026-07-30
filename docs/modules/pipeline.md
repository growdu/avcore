# 模块设计：工作流编排（Pipeline）

> 把"视频生成"拆解为可编排、可重试、可观测的 DAG 节点。

---

## 1. 目标

- **可编排**：脚本、音频、画面、合成按 DAG 描述
- **可重试**：节点级失败重试
- **可恢复**：节点结果持久化，支持断点续跑
- **可观测**：每节点有 trace / log / metric
- **可扩展**：新增节点零侵入

---

## 2. DAG 描述（YAML）

```yaml
id: video.generate.v1
nodes:
  - id: script_gen
    type: llm
    uses: chat
    input_from: job
    config:
      prompt_template: script_gen_v3
      persona: ${job.character.persona}
      expert: ${job.character.expert}

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
      voice: ${job.character.voice}
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
      avatar: ${job.character.avatar}

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

edges:
  - { from: script_gen, to: tts }
  - { from: script_gen, to: bgm_select }
  - { from: script_gen, to: img_gen }
  - { from: tts, to: i2v }
  - { from: img_gen, to: i2v }
  - { from: i2v, to: lipsync }
  - { from: lipsync, to: compose }
  - { from: bgm_select, to: compose }
  - { from: compose, to: finalize }
```

---

## 3. 节点类型

| 类型 | 描述 | 输入 | 输出 |
|------|------|------|------|
| `llm` | LLM 调用 | msgs | llm_response |
| `voice` | TTS 合成 | text, voice | audio + timestamps |
| `avatar_image` | 关键帧生成 | prompt, avatar | image |
| `video` | 图生视频 | image, audio, motion | clip |
| `lipsync` | 口型同步 | clip, audio | clip_synced |
| `asset_search` | 资产检索 | criteria | asset_ref |
| `composer` | 后期合成 | clips, bgm, opts | video |
| `encode` | 转码 | video | mp4 |
| `gate` | 人机协同 | prev_output | approval |
| `branch` | 条件分支 | prev_output | next_node_id |
| `http` | 通用 HTTP | url, payload | response |

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
    artifacts: list[Artifact]    # 文件 URL
    metrics: dict
    next_hint: list[str] | None  # 给调度器 hint
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

### 7.3 Worker 路由

| 节点 | Worker 池 |
|------|----------|
| `llm` | llm-pool（CPU） |
| `voice` | tts-pool（GPU/CPU） |
| `avatar_image` | img-pool（GPU） |
| `video` | video-pool（GPU） |
| `lipsync` | lipsync-pool（GPU） |
| `compose / encode` | compose-pool（CPU） |

---

## 8. 可观测性

每个节点自动注入：

- **Trace Span**：`job_id.node_id.attempt`
- **Metrics**：`node_duration_seconds{node, status}`, `node_failure_total{node, reason}`
- **Logs**：结构化 JSON，含 trace_id、tenant_id、job_id
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
- 节点平均时长（如实监控）
