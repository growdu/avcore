# 设计文档

> 极简内核设计：PersonaModel 持续完善，用某个版本出片。商业 / 开源模型 API + 单一 SQLite。

---

## 1. 顶层抽象

| 概念 | 含义 |
|------|------|
| **PersonaModel** | 一个角色。跨版本不变的顶层 ID。 |
| **PersonaModelVersion** | PersonaModel 的一次**不可变快照**（v1 / v2 / ...）。 |
| **PersonaSample** | 训练样本（图 / 音 / 行为文本 / 用户反馈）。 |
| **IterateJob** | 迭代任务：仅更新人设 / 知识 / 风格等**数据列**，不调 Provider SFT 端点。 |
| **FinetuneJob** | 微调任务：调 Provider 的 SFT / clone 端点，拿新 `model_id`，生成新版本。 |
| **KnowledgeCorpus** | 可选语料。仅当 PersonaModel 真正"懂某领域"时绑定。 |
| **Script** / **VideoJob** | 出片剧本 / 渲染任务。 |

### 1.1 完善角色 = 数据迭代 + 模型微调（两层）

AVCore **只调用 token 鉴权的商业 / 开源模型 API，不加载本地模型**。这意味着：  
"让角色变得更好"的绝大多数动作**只是数据更新**，根本不需要 Provider 介入。

```mermaid
flowchart TB
    subgraph R["refine (迭代, 80% 场景)"]
        R1["改 persona_descriptor_json"] --> R2["改 knowledge_binding_json"] --> R3["改 render options"] --> R4["同一个 PersonaVersion 升级列"]
    end
    subgraph F["finetune (微调, 少数场景)"]
        F1["追加样本"] --> F2["调 Provider SFT / clone 端点"] --> F3["拿新 voice_id / avatar_ref"] --> F4["INSERT 新 PersonaVersion"]
    end
```

| | refine（迭代） | finetune（微调） |
|---|---|---|
| 触发原因 | 人设要更严谨 / 知识要换 / 风格要稳 | 音色要更贴近 / 形象要更稳定 / 行为模式要新 |
| 调 Provider？ | 否，纯 SQL UPDATE | 是，调 Provider SFT 端点 |
| 数据库动作 | `UPDATE persona_versions`（同一行） | `INSERT persona_versions(version=N+1)` 新行 |
| 漂移兜底 | 不需要（数据不会让"脸"漂移） | 必须：阈值不达标 → DELETE 整行事务回退 |
| 频率 | 高（每次改 prompt / 加知识都跑） | 低（数周 / 数月一次） |
| 失败代价 | 低，可重写 | 高：要花 token 钱、重漂移 |

> 文档其余部分统一把"完善角色"称作 refine / finetune，不再使用"训练"作为主线表述。

核心 flow：

```mermaid
flowchart LR
    P1["PersonaModel v1"]:::snap -->|refine| P1r["v1 (refined)"]:::snap
    P1 -->|finetune| P2["v2"]:::snap
    P2 -->|refine| P2r["v2 (refined)"]:::snap
    P2 -->|finetune| P3["v3"]:::snap
    P1 --> R[VideoJob]
    P2 --> R
    P3 --> R
    P1r --> R
    P2r --> R
    classDef snap fill:#e3f2fd,stroke:#1976d2
```

子模块协作：

```mermaid
flowchart TB
    PS[persona-svc] --> ST[(avc.db)]
    IS[iterate-svc] --> ST
    FS[finetune-svc] --> ST
    RG[render-svc] --> ST
    PL[pipeline-svc] --> PS
    PL --> IS
    PL --> FS
    PL --> RG
```

---

## 2. 端到端流程

### 2.1 创建 v1

```mermaid
sequenceDiagram
    participant U as CLI
    participant PS as persona-svc
    participant PL as pipeline-svc
    participant AV as avatar Provider
    participant VO as voice Provider
    participant LM as llm Provider
    U->>PS: avc persona new yu --from samples.toml
    PS->>PL: DAG persona.create
    PL->>AV: create(spec) [token]
    AV-->>PL: avatar_primary BLOB
    PL->>VO: clone(samples) [token]
    VO-->>PL: voice_sample + voice_embed
    PL->>LM: extract_persona [token]
    LM-->>PL: persona_descriptor_json
    PL->>ST: INSERT persona_versions (status=ready)
    ST-->>PL: ok
    PS-->>U: persona_id, version=1
```

### 2.2 迭代 refine（数据层完善）

> 改的全是 persona_versions 同一行的**可改列**——`persona_descriptor_json` / `knowledge_binding_json` / `manifest_json` / `metrics_json`。  
> 不调 Provider，不新增版本号。

```mermaid
sequenceDiagram
    participant U as CLI
    participant IS as iterate-svc
    participant LM as llm Provider (可选)
    participant ST as avc.db
    U->>IS: avc persona refine yu --set traits=严谨,务实 --rebind-corpus db-internals
    IS->>LM: re_extract_persona(text) [token, 可选]
    LM-->>IS: new_persona_descriptor_json
    IS->>ST: UPDATE persona_versions SET persona_descriptor_json=? WHERE pm_id=? AND version=N
    ST-->>IS: ok
    IS-->>U: version=N, refined_at=...
```

特点：

- 同版本号升级；不破坏历史 VideoJob 绑定
- 没有漂移问题（数据层不可能让"脸"漂走）
- 失败回退：上次成功的人设内容已被上游 toml 备份，重跑即可
- 频率最高：每次调整 prompt、加新知识、改渲染选项都走它

### 2.3 微调 finetune（模型层完善）

> 追加样本 → 调 Provider SFT / clone 端点 → 拿新 `voice_id` / `avatar_ref` → INSERT 新版本 → 漂移兜底。

```mermaid
sequenceDiagram
    participant U as CLI
    participant FS as finetune-svc
    participant PL as pipeline-svc
    participant VO as voice Provider SFT
    participant AV as avatar Provider SFT
    participant ST as avc.db
    U->>FS: avc persona finetune yu --scope voice --add sample.wav --threshold 0.85
    FS->>PL: DAG persona.finetune
    PL->>PL: sample_filter (quality + dedup)
    PL->>VO: finetune(base + samples) [token]
    VO-->>PL: new_voice_id
    PL->>ST: INSERT persona_versions (version=N+1, status=building) 新行
    PL->>ST: 抽 anchor + drift_eval
    alt 漂移达标
        PL-->>FS: published v(N+1)
    else drift
        PL->>ST: DELETE FROM persona_versions WHERE version=N+1 (事务回退整行)
        PL-->>FS: rolled_back + drift_report
    end
```

特点：

- 一定产生新版本号
- 必须漂移兜底（Provider 重新训出的权重 / 嵌入可能跑偏）
- 比 refine 贵：花 token、花时间、有失败率

### 2.4 出片

```mermaid
sequenceDiagram
    participant U as CLI
    participant RG as render-svc
    participant PL as pipeline-svc
    participant LM as llm Provider
    participant VO as voice Provider
    participant IV as video Provider
    participant ST as avc.db
    U->>RG: avc render video --persona yu --version N --topic "..."
    RG->>ST: 读 persona_versions(N) → BLOB
    RG->>PL: DAG video.render
    PL->>LM: script_gen [token]
    LM-->>PL: Script
    PL->>VO: tts + img_gen (并发) [token]
    PL->>IV: i2v [token]
    PL->>ST: 写 artifacts 表 (BLOB final.mp4 + cover + subtitle + meta)
    RG-->>U: job_id
```

### 2.5 切版本 / 回滚

```sql
UPDATE persona_models SET current_version = N WHERE id = 'pm_xxx';
```

不影响已渲染视频（它们绑的是当时 version）。

---

## 3. 状态机

### 3.1 IterateJob（refine 任务账本）

```mermaid
stateDiagram-v2
    [*] --> queued --> running --> succeeded
    running --> failed
    failed --> [*]
    succeeded --> [*]
    cancelled --> [*]
```

### 3.2 FinetuneJob（finetune 任务账本）

```mermaid
stateDiagram-v2
    [*] --> queued --> running
    running --> succeeded
    running --> failed_drift
    running --> failed
    running --> cancelled
    cancelled --> [*]
    succeeded --> [*]
    failed --> [*]
    failed_drift --> [*]
```

### 3.3 VideoJob

```mermaid
stateDiagram-v2
    [*] --> queued --> running
    running --> succeeded
    running --> failed
    running --> cancelled
    cancelled --> [*]
    succeeded --> [*]
    failed --> [*]
```

---

## 4. 持久化

**单一 SQLite 文件** = 全部状态。元数据 + BLOB（avatar / voice / 嵌入向量 / 视频产物）都在 `~/.local/share/avc/avc.db`。

详见 [`storage.md`](./storage.md)。

约束：

- 一个 PersonaModelVersion = `persona_versions` 一行
- 不可变通过 INSERT 新行；DELETE 整行事务回退
- **refine 不新增行**——同 version 上 UPDATE `persona_descriptor_json` / `knowledge_binding_json` / `manifest_json` 等可改列
- 配置 / token 走独立 `~/.config/avc/avc.toml`（不入 DB）

---

## 5. 关键不变量

1. **不可变版本**：`UPDATE persona_versions` 仅允许 refine 改可改列；ready 后 BLOBs 不可写
2. **refine 同版本**：refine 不新增版本号，历史 VideoJob 绑定永远不变
3. **finetune 漂移兜底**：finetune 任务不达标，DELETE 整行事务回退，到 vN 为止
4. **历史视频锁定**：VideoJob.persona_version 不随 persona 完善漂移
5. **回滚 = 指针回拨**：切 `current_version` 不删任何东西

---

## 6. 不做什么（内核边界）

- ❌ 不内置计费 / 配额 / 多租户 / 看板
- ❌ 不加载 / 不推理任何本地模型（全 token 鉴权 API）
- ❌ 不自动创建 PersonaModel（必须由人触发）
- ❌ 不内置审核策略（外部挂）
- ❌ 不默认对象存储

> 这些都是**外部系统**的事，不是内核的事。

---

## 7. CLI 设计原则（速读）

CLI 分为 **原子** 与 **集成** 两类命令（详见 [`cli.md`](./cli.md)）：

- **原子**：`<noun> <verb>`，单一资源单一操作；可被 shell 任意组合
- **集成**：封装典型工作流（如 `persona onboard`、`persona refine`、`persona finetune`、`render run`），内部走原子

每个集成命令都接受 `--dry-run` 展开为原子清单——集成 = **原子 + 顺序 + 默认值**，可观察、可回放。

---

## 8. 后续阅读

- [architecture.md](./architecture.md) · 技术栈、子模块依赖
- [storage.md](./storage.md) · 完整 schema
- [cli.md](./cli.md) · 命令与用法
- 子模块：[persona-modeling](./modules/persona-modeling.md) · [persona-iteration](./modules/persona-iteration.md) · [video-generation](./modules/video-generation.md) · [pipeline](./modules/pipeline.md)
- [api/README.md](./api/README.md) · Provider trait
