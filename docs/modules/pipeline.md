# 模块：工作流编排（Pipeline）

> 训练与渲染共用一套 DAG 引擎。最少节点类型、最小调度复杂度。

---

## 节点类型

| 类型 | 用途 |
|------|------|
| `avatar` | 形象 create / finetune |
| `voice` | 声音 clone / synth / finetune |
| `llm` | LLM chat（脚本生成 / 人设抽取） |
| `video` | i2v |
| `embed` | 远端 embed API |
| `compose` | FFmpeg 拼接 / 转码（**唯一本地工具**，非 ML 模型） |
| `gate` | 人机协同门 |
| `branch` | 条件分支 |

每个节点结果落 `job_steps` 表：`(id, job_id, node_id, status, attempt, outputs_json, error_json, duration_ms)`。

---

## 两条 DAG（共用同一引擎）

### 5.1 `persona.create`

```yaml
nodes:
  - id: avatar_create
    type: avatar
  - id: voice_clone
    type: voice
  - id: persona_extract
    type: llm
  - id: anchor_extract
    type: embed
    input_from: [avatar_create, voice_clone, persona_extract]
  - id: finalize
    type: branch
    input_from: anchor_extract
    config:
      on_ok: INSERT persona_versions status=ready
      on_fail: ABORT (整事务回退)
```

### 5.2 `persona.finetune`（vN → v(N+1)；仅在调 Provider SFT 时走）

```yaml
nodes:
  - id: sample_filter
    type: llm                      # quality + dedup
  - id: avatar_sft
    type: avatar
    when: ${job.scope contains avatar}
    input_from: sample_filter
  - id: voice_sft
    type: voice
    when: ${job.scope contains voice}
    input_from: sample_filter
  - id: persona_sft
    type: llm
    when: ${job.scope contains persona}
    input_from: sample_filter
  - id: anchor_extract
    type: embed
    input_from: [avatar_sft, voice_sft, persona_sft]
  - id: drift_eval
    type: gate                    # 实际是 branch，对比阈值
  - id: publish
    type: branch
    on_pass: status=ready
    on_fail: DELETE 整行事务回退 + drift_report
```

### 5.3 `video.render`

```yaml
nodes:
  - script_gen   (llm)
  - tts          (voice, fanout per scene)
  - img_gen      (avatar, fanout per scene)
  - i2v          (video, fanout per scene, depends_on=[tts, img_gen])
  - compose      (compose, depends_on i2v)
  - encode       (compose)
  - write_meta   (写到 artifacts.meta)
```

---

## 调度器（最小实现）

```rust
loop {
    let ready: Vec<Node> = dag
        .nodes.iter()
        .filter(|n| deps_done(n) && n.status == Pending)
        .collect();

    for n in ready {
        // 提交到 worker pool
        tokio::spawn(execute(n, ctx));
    }

    // 落库节点状态
    if let Some(ev) = event_rx.recv().await { persist(ev); }
}
```

- 节点结果**先写 SQLite，再标记 ready**——保证崩溃可续
- 重试：节点级 3 次指数退避
- 单 PersonaModel **不并发训练**（防版本冲突）

---

## 关键指标

- 调度延迟：节点 ready → worker ≤ 500ms (P95)
- 节点成功率 ≥ 98%
- 端到端成功率 ≥ 95%
