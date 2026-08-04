# 模块：人物角色模型迭代与微调（Persona Iteration & Finetune）

> 在 v1 之上：让 PersonaModel 变得更好。  
> 本框架**只调用 token 鉴权 API**，不加载本地模型——所以"完善角色"在绝大多数情况下**只是数据更新**，只有少数场景才调 Provider 的 SFT / clone 端点。

---

## 1. 两层语义：refine vs finetune

```mermaid
flowchart TB
    subgraph R["refine (迭代) — 80% 场景"]
        R1["改 persona_descriptor_json"] --> R2["改 knowledge_binding_json"] --> R3["改 render options"]
        R3 --> R4["UPDATE 同一 PersonaVersion 的可改列"]
    end
    subgraph F["finetune (微调) — 少数场景"]
        F1["追加样本"] --> F2["调 Provider SFT / clone 端点"] --> F3["拿新 voice_id / avatar_ref"]
        F3 --> F4["INSERT 新 PersonaVersion + 漂移兜底"]
    end
```

| 维度 | refine | finetune |
|---|---|---|
| 一句话 | 改 prompt / 人设 / 知识 / 渲染参数 | 重新训声音 / 形象 / 行为嵌入 |
| 调 Provider？ | 否（或仅 LLM 抽人设） | 是，调 Provider SFT / clone 端点 |
| 数据库动作 | `UPDATE persona_versions` 同一行可改列 | `INSERT persona_versions(version=N+1)` 新行 |
| 漂移兜底 | 不需要 | 必须（阈值不达标 → DELETE 整行事务回退） |
| 频率 | 高（每次调整 prompt / 加知识 / 改风格） | 低（数周 / 数月一次） |
| 失败代价 | 低，可重写 | 高：要花 token 钱、有重训时间 |
| 对应 Provider 节点 | 仅 `llm.extract_persona`（可选） | `avatar.finetune` / `voice.finetune` / `llm.behavior_finetune` |

> "训练"这个说法不在本框架主叙事里——调一次 Provider SFT 端点才叫 finetune；其他都是 refine。

---

## 2. 任务账本

### 2.1 IterateJob（refine 任务账本）

```rust
struct IterateJob {
    id: String,                       // ij_<ULID>
    persona_model_id: String,
    target_version: u32,              // 同版本号升级 = current_version
    changes_json: String,             // {persona_descriptor?: {...}, knowledge_binding?: {...}, manifest?: {...}}
    status: Status,                   // succeeded（iterate 是同步单步 op，只写 succeeded；
                                       //   未来切到异步实现后可能引入 queued/running/failed/cancelled）
    started_at: Option<String>,
    finished_at: Option<String>,
}
```

### 2.2 FinetuneJob（微调任务账本）

```rust
struct FinetuneJob {
    id: String,                       // fj_<ULID>
    persona_model_id: String,
    base_version: u32,                // 从哪个版本开始
    target_version: u32,              // 预占 = base+1
    scope: Vec<Scope>,                // [avatar, voice, persona]
    config: FinetuneConfig,
    status: Status,                   // running/succeeded/failed_drift/cancelled
    result_version: Option<u32>,
    drift_report: Option<DriftReport>,
}

struct FinetuneConfig {
    full_retrain: bool,               // false=增量
    epochs: u32,
    consistency_threshold: f32,       // 默认 0.85
}
```

---

## 3. refine 数据流

### 3.1 可改列清单

refine 只动以下列，**绝不触碰 avatar / voice / anchor 的 BLOBs**：

| 列 | 改的什么 |
|---|---|
| `persona_descriptor_json` | traits / tone / catchphrases / taboos / scenario_prompts |
| `knowledge_binding_json` | 绑定 / 解绑 corpus；切换 grounding_mode |
| `manifest_json` | render options（分辨率 / 字幕样式 / 镜头偏好） |

> `metrics_json` 在当前 v0.3.3 **不被 `iterate apply` 写入**——
> 仅作"未来 / 只读"统计列（不入可改清单）。如需更新 metrics，应走单独的
> 度量作业或手工 SQL。

### 3.2 DAG（极简）

```mermaid
flowchart LR
    EX[llm.extract_persona (可选)] --> WR["UPDATE persona_versions<br/>SET persona_descriptor_json=?, knowledge_binding_json=?, manifest_json=?<br/>WHERE pm_id=? AND version=N"]
    WR --> LOG[INSERT iterate_jobs<br/>status='succeeded']
```

> 通常**不调 LLM**——人设直接来自上游 `persona.toml`；只有从自然语言推断人设时才调一次。  
> 这一支没有漂移问题：SQL UPDATE 单事务即可。

### 3.3 失败回退

- refine 写入前先在临时表 / 内存中构造新行；失败时不写
- 上次成功的人设内容应被上游 toml / git 备份，可手工 `UPDATE` 覆盖
- 合并规则：`persona_descriptor` / `knowledge_binding` / `manifest` 三列**按 JSON Pointer 规则 deep merge**（`svc::iterate::merge_value`）：
  - 同 key 嵌套 object → 递归合并
  - 同 key 非 object → 覆盖
  - patch 里 `null` → 删 key
  - 数组（catchphrases 等）走 `set-traits` / `set-catchphrase` 等原子 verb，**不**走 iterate apply

---

## 4. finetune 数据流

> 仅在调 Provider SFT 端点时才走这条路径。

### 4.1 DAG 流水线

```mermaid
flowchart LR
    SF[sample_filter] -->|"if avatar"| AT[avatar SFT vendor CLI]
    SF -->|"if voice"| VT[voice SFT vendor CLI]
    SF -->|"if persona"| PT[persona SFT]
    AT --> AN[anchor extract]
    VT --> AN
    PT --> AN
    AN --> DE[drift_eval]
    DE -->|"≥ threshold"| PUB[→ v(N+1) ready]
    DE -->|"< threshold"| RB[DELETE 整行事务回退]
```

> 所有 SFT 节点都走 Provider 的远端 SFT 端点——**AVCore 不下载权重**。
>
> **vendor CLI 协议**（`OpenAiCompat{Avatar,Voice}Provider.finetune` 在 `cfg.binary` 设了的情况下）：
> 1. `binary finetune submit --ref-image <paths...>`（avatar）/ `--ref-audio <paths...>`（voice）→ stdout `task_id=...`
> 2. `binary finetune status --task-id <id>` → stdout `status=done|pending|failed`，500ms poll、5 min timeout
> 3. `binary finetune fetch --task-id <id> --out <path>` → 写真 PNG/WAV 文件
>
> 仿 CliVideoProvider 三段式；fetch 出来的 tmp 文件由 `TempFileGuard` 在 drop 时清。
> 未配 `binary` 时按既有行为报 "requires a vendor CLI binary"（`avc finetune publish` 走手动 drift 也保留）。

### 4.2 样本池（persona_samples）

| kind | 形态 |
|------|------|
| image | blob BLOB + mime |
| audio | blob BLOB + transcript TEXT |
| behavior_text | text TEXT |
| feedback | text + weight（来自 `avc job feedback`） |

每条样本带 `consent_proof`（授权引用）。`avc sample add` 入库前自动跑质量检查。

### 4.3 一致性兜底

v0.3.1 起 drift 评估是**多维**（face / voice / style），每个维度独立算 cosine，
最终 `passed` = **所有** present 维度 cosine ≥ 阈值。详细多维模型见
[§4.4](#44-多维-drift)。

```rust
async fn drift_eval(parent: &Anchor, target: &Anchor, cfg: &Cfg) -> DriftReport {
    // parent / target 每个 dim 都可能缺失（None）；只对 present 的算 cosine
    let present: Vec<f32> = [parent.face, target.face,
                             parent.voice, target.voice,
                             parent.style, target.style]
        .chunks_exact(2)
        .filter_map(|pair| match pair {
            [Some(a), Some(b)] => Some(cosine(a, b)),
            _ => None,
        })
        .collect();
    let avg = if present.is_empty() { 1.0 } else { present.iter().sum::<f32>() / present.len() as f32 };
    let passed = present.iter().all(|c| *c >= cfg.consistency_threshold);
    DriftReport {
        face: present.get(0).copied().unwrap_or(0.0),
        voice: present.get(1).copied().unwrap_or(0.0),
        style: present.get(2).copied().unwrap_or(0.0),
        avg,
        passed,
    }
}
```

> `passed = present.iter().all(|c| *c >= threshold)`——任一维度未达阈值就整体失败。
> `drift_report` 写回 `finetune_jobs.drift_report_json`；不达标 → DELETE 整行事务回退。

### 4.4 多维 drift

> **Phase 2.5.1 起（v0.3.1）**：drift 不再只看 voice。PersonaModel 在 `face` / `voice` / `style`
> 三个维度上各有一个 anchor embedding；finetune run 时对**所有维度**并行评估，
> 任一维度 cosine < threshold → 整体回退。

#### 4.4.1 三维数据列

每个维度的 anchor 存在 `persona_versions` 三个独立列组（migration 0002）：

| 维度 | blob | dim | sha256 |
|------|------|-----|--------|
| face | `face_embed` | `face_embed_dim` | `face_embed_sha256` |
| voice | `voice_embed` | `voice_embed_dim` | `voice_embed_sha256` |
| style | `style_embed` | `style_embed_dim` | `style_embed_sha256` |

- face / style 是 v0.3.1 新增（migration `0002_drift_dimensions.sql`）。
- voice 在 migration 0001 已建。
- 同一 person + 同一 version 在每个维度上都是**稳定锚点**（commit 时复制到
  `anchor_*_emb`，供后续版本对比）。

#### 4.4.2 seed text 协议

每个维度的"anchor embedding" = `embed.<name>.embed("persona:<name>:<dim>:<v>")`
（`svc::drift::Dimension::seed_text`）。同维度 + 同 persona + 同 version 总是返回
同一 seed → 同一向量空间同一切片，是稳定锚点：

```
persona:yu:face:1
persona:yu:voice:1
persona:yu:style:1
persona:yu:face:2
...
```

不下载 CLIP / face_recognition 等本地模型——通过"维度不同 → 种子文本不同 →
同一 embed 空间不同切片"实现多维 drift。

#### 4.4.3 passed 语义

```rust
let present_cosines: Vec<f32> = [voice_cosine, face_cosine, style_cosine]
    .iter()
    .filter_map(|c| *c)
    .collect();
let avg = if present_cosines.is_empty() {
    1.0
} else {
    present_cosines.iter().sum::<f32>() / present_cosines.len() as f32
};
let passed = present_cosines.iter().all(|c| *c >= threshold);
```

- `present_cosines` = 该次 run 实际算出的 dim cosine（不在 scope 内的 dim
  不参与；未配 embed 时该 dim → None）。
- `passed` 严格要求**全部** present 维度 ≥ threshold（不是平均），任一不达标 → 整体回退。
- `avg` 仅供 `drift_report_json` 输出可视化，不参与 passed 决策。

> 与 §4.3 块一致：任一 dim 拖后腿 → `finetune_jobs.status='failed_drift'` +
> `DELETE persona_versions target`（见 `svc::finetune::run` 与 `publish`）。

---

## 5. 不可变与回滚（仅 finetune 适用）

- finetune 后 v(N+1) 通过 INSERT 新行产生，原 vN 一字不改
- 漂移达标 → `UPDATE status='ready'`
- 漂移不达标 → `DELETE FROM persona_versions WHERE version=N+1` + `UPDATE finetune_jobs SET status='failed_drift'`
- 强制回滚：`avc persona current yu --set 1` 即把指针拨回

> refine 不存在"回滚"语义——refine 是同版本号上对可改列的覆盖；用上游 toml / git 找回历史值即可。

---

## 6. 命令

### 6.1 refine（原子 + 集成）

**原子**（精细操作）：

```bash
avc persona set-traits     yu --version 1 --traits 严谨,务实
avc persona set-catchphrase yu --version 1 --add "我们直接看源码"
avc persona set-render     yu --version 1 --resolution 1080p --subtitle-style minimal
avc corpus attach          yu --version 1 --corpus db-internals
avc corpus detach          yu --version 1
avc iterate list --persona yu
avc iterate show ij_xxx
```

**集成**（典型 80% 路径）：

```bash
avc persona refine yu --from ./yu.v2.toml
# 内部 = set-traits + set-catchphrase + set-render + corpus attach/detach (按 toml diff)
# 通常不调 Provider；纯 SQL UPDATE
```

`avc persona refine yu --from ./yu.v2.toml --dry-run` 应输出：

```
plan (no changes made):

  1. set-traits   yu --version 1 --traits 严谨,务实                  (atomic)
  2. set-catchphrase yu --version 1 --add "我们直接看源码"             (atomic)
  3. set-render   yu --version 1 --resolution 1080p                 (atomic)
  4. corpus attach yu --version 1 --corpus db-internals              (atomic)
```

### 6.2 finetune（原子 + 集成）

**原子**（精细操作）：

```bash
avc sample add yu --kind audio --uri ./new.wav --text "..." --consent ./auth.pdf
avc finetune start yu --scope voice --base-version 1 --threshold 0.85
avc finetune run fj_xxx --embed openai_embed           # 端到端：SFT → drift → publish/rollback
avc finetune drift eval fj_xxx --embed openai_embed    # 只算 drift 不动 fj.status
avc finetune list yu                                   # 位置参数（v0.3.3 改）
avc finetune show fj_xxx
avc finetune publish fj_xxx --passed                   # 测试用：手动 publish（仅 --passed 生效）
avc finetune report fj_xxx --json
avc finetune cancel fj_xxx
```

> `avc finetune run` 是 Phase 2.5 新加的端到端 verb（v0.3.1 起走 3-dim drift）：
> 1. 校验 `fj.status == 'running'`（已 published → Conflict）
> 2. 拉 `persona_samples` kind=image/audio → materialize 到 tmp → 调 `provider.finetune()`
> 3. 把 vendor 返的 PNG/WAV 写到 `persona_versions` target 行（`voice_sample` / `avatar_primary`）
> 4. **3-dim drift**：
>    - voice scope → `embed.<name>.embed("persona:<name>:voice:<v>")` → 写 `voice_embed` → 与 base cosine
>    - avatar scope → `embed.<name>.embed("persona:<name>:face:<v>")` → 写 `face_embed` → 与 base cosine
>    - style 永远算 → `embed.<name>.embed("persona:<name>:style:<v>")` → 写 `style_embed` → 与 base cosine
>    （seed text 协议见 §4.4.2）
> 5. `present_cosines.iter().all(|c| *c >= threshold)` → `UPDATE persona_versions SET status='ready'` + `finetune_jobs.status='succeeded'`
>    任一不达 → `DELETE persona_versions target` + `finetune_jobs.status='failed_drift'`
>
> 退出码：succeeded → 0、failed_drift → 4、缺 `--embed`（任一 dim）→ 2、Provider 错误 → 11/12。

**集成**（典型工作流）：

```bash
# v0.3 起 persona finetune 子动词已合并到顶层 finetune.*；无 `avc persona finetune`
avc finetune start yu --scope voice --base-version 2 --threshold 0.85
avc sample add yu --kind audio --uri ./feedback_*.wav --consent ./auth.pdf   # 反馈样本（替代 --with-feedback 自动收集）
avc finetune run fj_xxx --embed openai_embed
```

`avc finetune start yu --scope voice --base-version 1 --dry-run` 应输出：

```
plan (no changes made):

  1. finetune start yu --scope voice --base-version 1    (atomic)
  2.   ↳ 3-dim drift eval: voice + face + style
  3. publish_or_rollback branch
  4. persona commit yu --version <v>   if drift ok       (atomic)
  5. persona promote yu --to <v>      if drift ok        (atomic)
```

---

## 7. 关键指标

- **refine**：单次 P95 ≤ 1s（纯 SQL）；人设改动可在分钟内传播到下一次出片
- **finetune**：单维度微调 P95 ≤ 30 min；跨版本一致性 ≥ 0.85；微调成功率 ≥ 95%
