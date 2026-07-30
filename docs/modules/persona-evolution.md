# 模块：人物角色模型完善演进（Persona Evolution）

> 在 v1 之上：**追加样本 → 训练 → 出 v(N+1) → 一致性兜底**。
> 历史视频永远绑生成时的 version，不随 persona 演进而改变。

---

## 训练任务

```rust
struct TrainingJob {
    id: String,                       // tj_xxx
    persona_model_id: String,
    base_version: u32,                // 从哪个版本开始
    target_version: u32,              // 预占 = base+1
    scope: Vec<Scope>,                // [avatar, voice, persona]
    config: TrainingConfig,
    status: Status,                   // queued/running/succeeded/failed_drift/failed
    result_version: Option<u32>,
    drift_report: Option<DriftReport>,
}

struct TrainingConfig {
    full_retrain: bool,               // false=增量
    epochs: u32,
    consistency_threshold: f32,       // 默认 0.85
}
```

---

## DAG 流水线

```mermaid
flowchart LR
    SF[sample_filter] -->|"if avatar"| AT[avatar SFT]
    SF -->|"if voice"| VT[voice SFT]
    SF -->|"if persona"| PT[persona SFT]
    AT --> AN[anchor extract]
    VT --> AN
    PT --> AN
    AN --> DE[drift_eval]
    DE -->|"≥ threshold"| PUB[→ v(N+1) ready]
    DE -->|"< threshold"| RB[DELETE 整行事务回退]
```

> 所有 SFT 节点都走 Provider 的远端 SFT 端点——**AVCore 不下载权重**。

---

## 样本池（persona_samples）

| kind | 形态 |
|------|------|
| image | blob BLOB + mime |
| audio | blob BLOB + transcript TEXT |
| behavior_text | text TEXT |
| feedback | text + weight（来自 `avc job feedback`） |

每条样本带 `consent_proof`（授权引用）。`avc persona sample add` 入库前自动跑质量检查。

---

## 一致性兜底：最简单的实现

```rust
async fn drift_eval(parent_anchor: &Anchor, new_anchor: &Anchor, cfg: &Cfg) -> DriftReport {
    let face = cosine(parent_anchor.face, new_anchor.face);
    let voice = cosine(parent_anchor.voice, new_anchor.voice);
    let style = cosine(parent_anchor.style, new_anchor.style);
    let avg = (face + voice + style) / 3.0;
    DriftReport { face, voice, style, avg, passed: avg >= cfg.consistency_threshold }
}
```

> 仅 `cos >= threshold` → 发布；否则 DELETE 整行事务回退。`drift_report` 写回 `training_jobs.drift_report_json`。

---

## 不可变与回滚

- v(N+1) 通过 INSERT 新行产生，原 vN 一字不改
- 漂移达标 → `UPDATE status='ready'`
- 漂移不达标 → `DELETE FROM persona_versions WHERE version=N+1` + `UPDATE training_jobs SET status='failed_drift'`
- 强制回滚：`avc persona current yu --set 1` 即把指针拨回

---

## CLI

```bash
avc persona sample add yu --kind audio --uri ./new.wav --text "..." --consent ./auth.pdf
avc persona evolve yu --scope voice --base-version 2 --threshold 0.85
avc task show tj_xxx --watch
avc training report tj_xxx --json
```

---

## 关键指标

- 单维度训练 P95 ≤ 30 min
- 跨版本一致性 ≥ 0.85
- 训练成功率 ≥ 95%
