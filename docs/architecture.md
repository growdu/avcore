# AVCore 架构文档（Architecture Document）

> 回答"用什么技术、怎么组织、核心流程怎么跑、子模块怎么协作、信息怎么存"。配套设计文档 [`design.md`](./design.md)。

> 📐 **本图多**——任何核心场景都至少配 2 张图（流程图 + 时序/状态/ER/gitgraph）。如果只想 5 分钟看懂全貌，跳到 §3 五个子模块 与 §4 核心流程。

---

## 1. 一句话总结与强约束

**AVCore = Rust 单二进制 + 本地 SQLite + 本地文件系统 + 统一 DAG Pipeline + 一组 trait 化的 Provider 适配器。**

🔒 **强约束** —— AVCore **只调用商业 / 开源模型的 HTTP API（全部 token 鉴权），不加载、不推理任何本地模型**。每个 Provider 必须有 `api_key`；训练 / 微调都在远端完成，本框架只持有引用。

不暴露 HTTP / gRPC 服务，不做 SaaS 控制台，不内嵌计费 / 可观测性 dashboard——把这些都交给外部系统。

---

## 2. 顶层形态

### 2.1 分层视图（flowchart）

```mermaid
flowchart TB
    subgraph CLIENT["客户端"]
        direction LR
        cli["avc CLI<br/>avc 命令"]
        repl["avc REPL<br/>交互 shell"]
        lib["Rust crate<br/>(集成方)"]
    end

    subgraph CORE["核心服务（单进程 tokio）"]
        direction TB
        persona["persona-svc<br/>建模/查询"]
        evolution["evolution-svc<br/>训练/版本/漂移"]
        render["render-svc<br/>脚本/出片"]
        corpus["corpus-svc<br/>知识语料"]
        pipeline["pipeline-svc<br/>DAG 调度"]
        task["task / job-svc<br/>异步任务账本"]
    end

    subgraph PROV["Provider 适配器（trait）"]
        direction TB
        avatar["avatar<br/>kling/doubao/seedream/replicate"]
        voice["voice<br/>elevenlabs/azure/doubao/openai"]
        llm["llm<br/>openai_compat"]
        video["video<br/>kling/seedance/pika/runway"]
        embed["knowledge<br/>openai/cohere/volc"]
    end

    subgraph STORAGE["本地存储"]
        direction TB
        db[("SQLite<br/>avc.db")]
        fs[/"FS<br/>~/.local/share/avc/"/]
        cfg[/"avc.toml<br/>(token 加密)"/]
    end

    subgraph EXTERNAL["远端模型 API"]
        direction LR
        kling[("Kling")]
        openai[("OpenAI")]
        doubao[("豆包 / 火山")]
        replicate[("Replicate")]
        other[("其他...")]
    end

    cli --> persona
    cli --> evolution
    cli --> render
    cli --> corpus
    repl --> pipeline
    lib --> pipeline
    lib --> persona

    persona --> pipeline
    evolution --> pipeline
    render --> pipeline

    pipeline --> avatar
    pipeline --> voice
    pipeline --> llm
    pipeline --> video
    pipeline --> embed

    persona --> db
    persona --> fs
    evolution --> db
    evolution --> fs
    render --> db
    render --> fs
    corpus --> db
    corpus --> fs

    avatar -.HTTP + Bearer.-> kling
    avatar -.HTTP + Bearer.-> doubao
    avatar -.HTTP + Bearer.-> replicate
    voice -.HTTP + Bearer.-> openai
    video -.HTTP + Bearer.-> kling
    llm -.HTTP + Bearer.-> openai
    embed -.HTTP + Bearer.-> openai
```

### 2.2 进程内协作（sequence）

下面这张图展示一次 `avc persona new` 在**单进程内**跑过的所有 in-process 调用——尽管图中画了多个服务，实际都在同一进程、同一 tokio runtime 里。

```mermaid
sequenceDiagram
    autonumber
    participant U as 用户 / CLI
    participant PS as persona-svc
    participant TS as task-svc
    participant PL as pipeline-svc
    participant AP as avatar Provider
    participant VP as voice Provider
    participant LP as llm Provider
    participant ST as storage (SQLite + FS)

    U->>PS: avc persona new "Yu" --from sample.toml
    PS->>PS: 解析 + 校验输入
    PS->>ST: 预创建 personas/pm_xxx/v1/<br/>(空目录 + manifest.status=building)
    PS->>TS: 创建 task_tsk_xxx (running)
    PS->>PL: 提交 DAG persona.create.v1
    PL->>AP: create(spec)  [HTTP+token]
    AP-->>PL: {avatar_ref, primary.png}
    PL->>VP: clone(samples)  [HTTP+token]
    VP-->>PL: {voice_ref, sample.wav, embed.bin}
    PL->>LP: chat(extract persona)  [HTTP+token]
    LP-->>PL: {persona_descriptor}
    PL->>AP: 抽取 identity_anchor.face  [远端 embed]
    PL->>VP: 抽取 identity_anchor.voice [远端 embed]
    PL->>ST: 写 v1/avatar / v1/voice / v1/persona.json / v1/identity_anchor.json
    PL->>ST: 落 manifest.json, status=ready
    PL-->>PS: DAG 成功
    PS->>TS: 标 task succeeded
    PS->>ST: write SQLite persona_models + persona_versions
    PS-->>U: 返回 persona_id + version=1
```

---

## 3. 五个子模块

| 子模块 | 主要服务 | 负责 | 不负责 |
|--------|----------|------|--------|
| [persona-modeling](./modules/persona-modeling.md) | `persona-svc` | PersonaModel v1 创建 | 训练、渲染 |
| [persona-evolution](./modules/persona-evolution.md) | `evolution-svc` | 训练任务 / 版本管理 / 漂移兜底 | v1 创建、渲染 |
| [video-generation](./modules/video-generation.md) | `render-svc` | 脚本 / DAG 出片 | 训练、人设设计 |
| [pipeline](./modules/pipeline.md) | `pipeline-svc` | 节点编排 / 调度 / 重试 / 断点 | 具体 Provider 调用 |
| [knowledge-aspect](./modules/knowledge-aspect.md) | `corpus-svc` | 语料 / RAG | 形象 / 声音 / 人设 |

### 3.1 子模块依赖图（flowchart）

```mermaid
flowchart LR
    PM[persona-modeling]
    EV[persona-evolution]
    VG[video-generation]
    PL[pipeline]
    KA[knowledge-aspect]

    PL -->|"调度训练节点"| EV
    PL -->|"调度渲染节点"| VG
    PL -->|"调度建模节点"| PM
    EV -->|"读样本池 + 写新版本"| ST[(storage)]
    PM -->|"写 v1"| ST
    VG -->|"读锁定 version<br/>写 media/jobs/"| ST
    KA -->|"语料索引"| VG
    KA -->|"训练时重建索引"| EV
    EV -->|"反馈信号回流"| ST
```

### 3.2 单一子模块内部协作（以 evolution 为例）

```mermaid
flowchart TB
    subgraph evolution["evolution-svc"]
        EP[入口] --> SF[sample_filter]
        SF -->|"scope=avatar"| AT[avatar_train 节点]
        SF -->|"scope=voice"| VT[voice_train 节点]
        SF -->|"scope=persona"| PT[persona_sft 节点]
        SF -->|"scope=knowledge"| KR[knowledge_reindex 节点]
        AT --> AN[anchor_extract]
        VT --> AN
        PT --> AN
        KR --> AN
        AN --> DE[drift_eval]
        DE -->|"≥ threshold"| PUB[publish v(N+1)]
        DE -->|"< threshold"| RB[rollback + drift_report]
        PUB --> FS[(storage<br/>写入新目录)]
        RB --> FS
    end
```

> 其他子模块的内部结构见各自章节。下面把所有跨子模块的**用户场景**完整画出。

---

## 4. 核心流程

下面四个流程构成 AVCore 的全部用户故事：

| 流程 | 触发命令 | 起始数据 | 产物 |
|------|----------|----------|------|
| 创建 v1 | `avc persona new "Yu" --from ./samples.toml` | 设定 + 样本 | `personas/pm_xxx/v1/` + SQLite row |
| 持续训练 | `avc persona evolve yu --scope voice --add ./new.wav` | 样本池 | `personas/pm_xxx/v2/` (or rollback) |
| 出片 | `avc render video --persona yu --topic "..."` | topic + 锁定 version | `media/jobs/job_xxx/final.mp4` |
| 反馈回灌 | `avc job feedback job_xxx --signal looks_unlike` | 反馈 | `persona_samples(kind=feedback)` |

### 4.1 流程 A：创建 PersonaModel v1

#### 4.1.1 逻辑流（flowchart）

```mermaid
flowchart TB
    A["avc persona new 'Yu' --from ./samples.toml"] --> B{input 校验}
    B -->|failed| ER1[返回 invalid_input]
    B -->|ok| C[预创建 v1 目录<br/>manifest.status=building]
    C --> D[avatar Provider<br/>create(spec)]
    D --> E[voice Provider<br/>clone(samples)]
    E --> F[llm Provider<br/>extract_persona]
    F --> G{KnowledgeBinding?}
    G -->|yes| H[corpus-svc: index chunks]
    G -->|no| I[skip]
    H --> J
    I --> J
    J[抽取 identity_anchor<br/>face/voice/style embedding] --> K[写 v1/avatar/voice/persona/identity_anchor]
    K --> L[manifest.status=ready + 写 SQLite]
    L --> M[任务 succeeded<br/>返回 persona_id]
```

#### 4.1.2 异常路径（flowchart）

```mermaid
flowchart TB
    A["任一 Provider 失败"] --> B{kind?}
    B -->|可重试<br/>网络/限速| C["retry 自动<br/>最多 N 次"]
    C -->|"仍失败"| D[mark task failed]
    B -->|不可重试<br/>输入/授权| D
    B -->|Provider 限速且有备选| E["降级到备用 Provider<br/>(路由表)"]
    E -->|"仍失败"| D
    D --> F{keep_partials?}
    F -->|true| G[保留 v1 目录中间产物<br/>供调试]
    F -->|false 默认| H[删除 v1 目录<br/>+ SQLite rollback]
    G --> I[avc task show task_xxx<br/>查看产物]
    H --> I
```

### 4.2 流程 B：持续训练 v1 → v2

#### 4.2.1 整体时序（sequence）

```mermaid
sequenceDiagram
    autonumber
    participant U as 用户
    participant EV as evolution-svc
    participant SM as sample pool (SQLite)
    participant PL as pipeline-svc
    participant AP as avatar Provider<br/>(SFT endpoint)
    participant VP as voice Provider<br/>(SFT endpoint)
    participant LP as llm Provider<br/>(SFT endpoint)
    participant EP as embed Provider<br/>(identity_anchor)
    participant ST as storage

    U->>EV: avc persona evolve yu --scope avatar,voice<br/>--base-version 1 --anchors ./canary/<br/>--threshold 0.85
    EV->>SM: 列 sample_ids by version_id_at_collection + scope
    SM-->>EV: N 个样本
    EV->>EV: 预创建 personas/pm_xxx/v2/ (空)
    EV->>PL: 提交 DAG persona.train.v1<br/>job_id = tj_xxx
    PL->>PL: 节点 sample_filter (去重 + 质检 + consent 校验)
    PL->>AP: finetune(base_avatar_ref, samples) HTTP POST
    AP-->>PL: 任务 ID + 长轮询 → { new_avatar_ref }
    PL->>VP: finetune(base_voice_ref, samples) HTTP POST
    VP-->>PL: 任务 ID + 长轮询 → { new_voice_ref }
    PL->>EP: extract face/voice/style embedding
    EP-->>PL: 新 anchor
    PL->>PL: drift_eval (cos vs parent)
    alt cos ≥ threshold
        PL->>ST: 拷贝/写所有 v2 资产 + manifest.json
        PL->>ST: SQLite: persona_versions +1, training_jobs.status=succeeded
        PL-->>EV: published v2
        EV-->>U: 训练报告 + 提示 "avc persona current yu --set 2"
    else drift detected
        PL->>ST: 删除 personas/pm_xxx/v2/ 整个目录
        PL->>ST: SQLite: training_jobs.status=failed_drift, drift_report_json=...
        PL-->>EV: rolled_back
        EV-->>U: 失败 + drift_report<br/>(展示每个维度 cos 值)
    end
```

#### 4.2.2 DAG 节点拓扑（flowchart）

```mermaid
flowchart LR
    A[sample_filter] -->|"if scope=avatar"| B[avatar_train]
    A -->|"if scope=voice"| C[voice_train]
    A -->|"if scope=persona"| D[persona_sft]
    A -->|"if scope=knowledge"| E[knowledge_reindex]
    B --> F[anchor_extract]
    C --> F
    D --> F
    E --> F
    F --> G[drift_eval<br/>vs base]
    G -->|"≥ threshold"| H[publish_or_rollback<br/>→ v2 ready]
    G -->|"< threshold"| I[publish_or_rollback<br/>→ rollback]
```

#### 4.2.3 训练任务状态机

```mermaid
stateDiagram-v2
    [*] --> queued
    queued --> running: worker 接管
    running --> succeeded: 漂移达标<br/>v(N+1) ready
    running --> failed_drift: 漂移不达标<br/>触发回退
    running --> failed: Provider 不可重试错误
    running --> cancelled: 用户取消
    failed --> queued: 用户 retry
    failed_drift --> queued: 用户 collect 新样本后 retry
    cancelled --> [*]
    succeeded --> [*]
    failed --> [*]
    failed_drift --> [*]
```

#### 4.2.4 版本时间轴（gitgraph）

```mermaid
gitgraph
    commit id: "v1<br/>initial"
    commit id: "samples +n"
    branch retry-1
    commit id: "v2 candidate<br/>drift=0.79"
    commit id: "rollback"
    checkout main
    commit id: "samples +m<br/>(canary)"
    branch retry-2
    commit id: "v2 candidate<br/>drift=0.92"
    commit id: "publish v2"
    checkout main
    commit id: "samples +k"
    branch alt
    commit id: "v3 candidate<br/>drift=0.88"
    commit id: "publish v3"
    checkout main
    commit id: "current=v3"
```

> 实际产品里 `current_version` 是 PersonaModel 的一个独立指针，可改回拨而不影响已渲染视频。

### 4.3 流程 C：视频渲染

#### 4.3.1 用户视角时序（sequence）

```mermaid
sequenceDiagram
    autonumber
    participant U as 用户
    participant RS as render-svc
    participant PL as pipeline-svc
    participant LP as llm Provider
    participant VP as voice Provider
    participant AP as avatar Provider
    participant IV as video Provider
    participant ST as storage

    U->>RS: avc render video --persona yu --version 2 --topic "InnoDB Buffer Pool 替换算法"
    RS->>ST: 读 personas/pm_xxx/v2/<br/>锁 version=2
    RS->>PL: 提交 DAG video.render.v1<br/>job_id=job_xxx
    PL->>LP: LLM 脚本生成 (system prompt from persona + RAG)
    LP-->>PL: Script JSON
    PL->>VP: tts(scene.batch) [并发]
    VP-->>PL: audio + word_timestamps
    PL->>AP: img_gen(scene.batch) [并发]
    AP-->>PL: keyframes
    PL->>IV: i2v(scene.batch) [依赖 tts+img]
    IV-->>PL: clips
    PL->>PL: lipsync + compose + encode
    PL->>ST: 写 media/jobs/job_xxx/{final.mp4, cover.jpg, subtitle.srt, meta.json}
    PL-->>RS: succeeded
    RS-->>U: job_id + 产物路径
```

#### 4.3.2 DAG 节点拓扑（flowchart）

```mermaid
flowchart LR
    SG[script_gen] --> SR{script_review<br/>人工门}
    SR -->|"require_human_review=true"| HALT[暂停 等待人工]
    SR -->|"默认 auto"| TT[tts]
    SR --> BGM[bgm_select]
    SG --> IMG[img_gen]
    TT --> I2V[i2v]
    IMG --> I2V
    I2V --> LIP[lipsync<br/>数字人模态跳过]
    BGM --> CMP[compose]
    LIP --> CMP
    CMP --> ENC[encode]
    ENC --> OUT["media/jobs/job_xxx/<br/>final.mp4 + meta.json"]
```

#### 4.3.3 渲染任务状态机

```mermaid
stateDiagram-v2
    [*] --> queued
    queued --> running: worker 接管
    running --> succeeded: 所有节点完成<br/>产物落盘
    running --> failed: 关键节点失败<br/>重试耗尽
    running --> partial: 节点失败但部分产物可用<br/>(avc job retry 自动重跑未完成节点)
    running --> cancelled: 用户取消
    failed --> queued: 用户 retry
    partial --> running: 用户 retry
    cancelled --> [*]
    succeeded --> [*]
    failed --> [*]
```

### 4.4 流程 D：反馈回灌

```mermaid
flowchart TB
    U["avc job feedback job_xxx --signal looks_unlike"] --> RS[render-svc]
    RS --> SM["persona_samples 表<br/>kind=feedback<br/>weight=1.0"]
    SM -.->|"下次 evolve 自动消费"| EV[evolution-svc]
    EV -.->|"漂移评估<br/>影响下次 v(N+1) 风格"| EVNEW[新版本]
```

### 4.5 流程 E：切版本 / 回滚 / A/B

```mermaid
sequenceDiagram
    autonumber
    participant U as 用户
    participant PS as persona-svc
    participant ST as storage (SQLite)

    U->>PS: avc persona current yu --set 3
    PS->>ST: UPDATE persona_models<br/>SET current_version=3
    PS->>U: ok
    Note over U,ST: 后续任务默认用 v3；<br/>已渲染视频仍绑 v1/v2<br/>(Job 表 persona_version 字段不变)

    U->>PS: avc persona deprecated yu --version 1
    PS->>ST: UPDATE persona_versions<br/>SET status='deprecated' WHERE version=1
    PS->>U: ok
    Note over U,ST: v1 不再被默认选中<br/>但不删，已渲染的仍能溯源
```

---

## 5. Provider 与 token 鉴权

### 5.1 鉴权流（sequence）

```mermaid
sequenceDiagram
    autonumber
    participant U as CLI
    participant CFG as config-svc
    participant TF as token preflight
    participant PR as Provider Adapter
    participant EXT as 远端 API

    U->>CFG: avc config set provider.avatar.kling.api_key "klg_..."
    CFG->>CFG: 加密落 avc.toml (chmod 600)
    Note over U,CFG: 任何调用前先 preflight

    U->>PR: 任意 Provider 调用
    PR->>TF: preflight(provider_name)
    TF->>CFG: 读 api_key
    alt 缺失
        TF-->>PR: Err(ProviderTokenMissing)
        PR-->>U: error[E0501] provider_unauthenticated<br/>hint: avc config set ...
    else 有
        TF-->>PR: ok
        PR->>EXT: HTTP POST {Bearer <token>}
        EXT-->>PR: 200 / 401 / 429
        alt 401/403
            PR-->>U: error[E0502] provider_unauthorized<br/>(需重新生成 token)
        else 429
            PR->>PR: 退避重试 N 次
        else 2xx
            PR-->>U: 产物
        end
    end
```

### 5.2 Provider 路由降级（flowchart）

```mermaid
flowchart LR
    CALL[Provider 调用] --> P1[kling_avatar 主]
    P1 -->|失败 / 不可用| P2[heygen_avatar 备1]
    P2 -->|失败| P3[doubao_image 备2]
    P3 -->|失败| FAIL[failed: 标 task failed]
    P1 -->|成功| OK[使用 P1 返回]
```

> Provider 注册到 `provider_registry`，`avc provider config` 可调整主/备顺序与启用列表。

---

## 6. 信息存储（重点：本框架怎么存）

> **一句话**：全部状态进单一 SQLite 文件 `~/.local/share/avc/avc.db`，配置 / token 走 `~/.config/avc/avc.toml`。
>
> 用户给定的规模约束：≤ 50 persona，单机运行。在这个量级，**单一 SQLite + BLOB 列**比"FS + SQLite"和"对象存储"都简单。详见 [`storage.md`](./storage.md)。



### 6.1 顶层文件布局（graph）

```mermaid
graph LR
    CFG_DIR["$HOME/.config/avc/"] --> TOML[("avc.toml<br/>provider token<br/>+ 开关")]
    DATA_DIR["$HOME/.local/share/avc/"] --> DB[("avc.db<br/>所有元数据 + BLOB")]
    DATA_DIR --> WAL[avc.db-wal]
    DATA_DIR --> SHM[avc.db-shm]

    classDef stable fill:#e8f5e9,stroke:#2e7d32
    class TOML,DB stable
```

> **唯一两个稳定文件：avc.toml（配置）+ avc.db（数据）**。其余 WAL/SHM 是 SQLite 运行时临时，关闭后回收。详见 [`storage.md`](./storage.md)。

### 6.2 SQLite Schema（erDiagram）

```mermaid
erDiagram
    persona_models ||--o{ persona_versions : has
    persona_models ||--o{ persona_samples : collects
    persona_models ||--o{ training_jobs : trains
    persona_versions ||--o{ training_jobs : produces
    persona_versions ||--o{ jobs : locked_by
    scripts ||--o{ jobs : executes
    knowledge_corpora ||--o{ corpus_chunks : contains
    knowledge_corpora ||--o{ persona_versions : bound_to

    persona_models {
        TEXT id PK
        TEXT name
        TEXT archetype
        INTEGER current_version
        TEXT status
        DATETIME created_at
        DATETIME updated_at
    }

    persona_versions {
        TEXT persona_model_id PK
        INTEGER version PK
        INTEGER parent_version FK
        TEXT dir_path
        TEXT status
        TEXT training_job_id FK
        TEXT manifest_json
        DATETIME created_at
    }

    persona_samples {
        TEXT id PK
        TEXT persona_model_id FK
        TEXT kind  "image|audio|behavior_text|feedback"
        TEXT uri_or_text
        INTEGER version_id_at_collection
        TEXT consent_proof
        TEXT tags_json
        REAL quality_score
        DATETIME created_at
    }

    training_jobs {
        TEXT id PK
        TEXT persona_model_id FK
        INTEGER base_version FK
        INTEGER target_version
        TEXT scope_json
        TEXT config_json
        TEXT status  "queued|running|succeeded|failed_drift|failed|cancelled"
        INTEGER result_version FK
        TEXT drift_report_json
        DATETIME started_at
        DATETIME finished_at
    }

    jobs {
        TEXT id PK
        TEXT script_id FK
        TEXT persona_model_id FK
        INTEGER persona_version FK
        TEXT status
        TEXT options_json
        TEXT artifacts_json
        DATETIME created_at
        DATETIME finished_at
    }

    scripts {
        TEXT id PK
        TEXT persona_model_id FK
        INTEGER persona_version FK
        TEXT topic
        TEXT scenes_json
        DATETIME created_at
    }

    knowledge_corpora {
        TEXT id PK
        TEXT name
        TEXT source_type
        TEXT language
        INTEGER chunk_count
        INTEGER index_version
        DATETIME created_at
    }

    corpus_chunks {
        TEXT id PK
        TEXT corpus_id FK
        INTEGER ordinal
        TEXT content
        INTEGER token_count
        INTEGER deprecated
        TEXT meta_json
    }

    job_steps {
        TEXT id PK
        TEXT job_id FK
        TEXT node_id
        TEXT status
        INTEGER attempt
        TEXT outputs_json
        TEXT artifacts_json
        TEXT error_json
        INTEGER duration_ms
        TEXT trace_id
    }

    audit_log {
        INTEGER id PK
        DATETIME ts
        TEXT actor
        TEXT action
        TEXT target_kind
        TEXT target_id
        TEXT detail_json
    }
```

### 6.3 不可变行的语义

每个 `PersonaModelVersion` = `persona_versions` 表的一行，包含所有元数据与 BLOB 资产。版本永远以新增行的方式产生，旧行不被 UPDATE。

```mermaid
graph TD
    ROW["persona_versions 一行<br/>＝ 一个 PersonaModelVersion"]
    ROW --> AV["avatar_* (BLOB)"]
    ROW --> VO["voice_* (BLOB)"]
    ROW --> PD[persona_descriptor_json]
    ROW --> KB[knowledge_binding_json]
    ROW --> IA["anchor_*_emb (BLOB)"]
    ROW --> MF[manifest_json]
    ROW --> META["status + created_at + sha256"]

    AV -.sha256.-> VERIFY[avc verify]
    VO -.sha256.-> VERIFY
    IA -.sha256.-> VERIFY

    classDef blob fill:#fff3e0,stroke:#e65100
    class AV,VO,IA blob
```

> 漂移不达标 → `DELETE FROM persona_versions WHERE version=N+1` 在事务内回退 = 整个版本消失。详见 [`storage.md §3`](./storage.md)。

## 7. 跨场景对照

| 场景 | 服务链 | 落盘位置 | 关键事件 |
|------|--------|----------|----------|
| 首次创建 persona | persona-svc → pipeline-svc → 3~4 Provider | `persona_versions` 新行 | `task_succeeded` / version=1 |
| 持续训练 | evolution-svc → pipeline-svc → 1~3 Provider | 预占 v(N+1)；成功 → INSERT；失败 → DELETE 事务回退 | `training_jobs.status` |
| 出片 | render-svc → pipeline-svc → LLM/TTS/i2v Provider | `jobs` + `artifacts.content BLOB` | `jobs.status` |
| 反馈回灌 | render-svc → `persona_samples` | SQLite 即可 | `sample(kind=feedback)` |
| 跨机迁移 | export / import | 整个 `avc.db` (or tar.zst 单 persona) | `avc export` / `import` |
| 紧急回滚 | persona-svc 改 current_version 指针 | UPDATE `persona_models` | `persona_models.current_version=vPrev` |

---

## 8. 关键技术决策（ADR 摘要）

| 编号 | 决策 | 备选 | 理由 |
|------|------|------|------|
| ADR-001 | **Rust + 单二进制** 为主仓 | Python 服务 / Go / Node | 启动快、类型强、与"CLI 优先"对齐 |
| ADR-002 | **SQLite + 本地文件系统** 起手（仅存元数据与产物引用） | Postgres / MinIO | 零运维、可拷走、单用户足够；模型权重不在本仓 |
| ADR-003 | **DAG 节点编排引擎自研** | Temporal / Argo | 起步要轻、可演进 |
| ADR-004 | **Provider 通过 trait 抽象** + 内置实现 | 配置化插件框架 | 内置足够简单；插件框架可后续加 |
| ADR-005 | **PersonaModelVersion 不可变** | 可变 + 软删 | 历史视频必须稳定 |
| ADR-006 | **训练任务独占（同一 persona 不并发）** | 并行训练 | 防止版本冲突 |
| ADR-007 | **不内嵌计费 / 可观测性 dashboard** | 内嵌 SaaS 化 | 与开源核心定位冲突 |
| ADR-008 | **CLI + REPL 双形态** | 仅 CLI / 仅 REPL | 自动化 + 探索各有需求 |
| ADR-009 | **Provider 通过 HTTP/gRPC，不在本仓跑模型** | 内置本地 GPU | 主仓是编排层，模型在 Provider |
| ADR-010 | **仅调用 token 鉴权的商业 / 开源模型 API** | 自托管 / 本地推理 | 与"开源核心、可商用接入"定位一致；免除 GPU / 驱动 / 推理服务治理负担 |

---

## 9. 演进路线

### Phase 0 — 最小闭环（4 周）
- `avc persona new` 跑通：v1 生成（avatar / voice / persona 全部 token API）
- `avc render video` 跑通：1 条视频（脚本 + tts + i2v + compose，全部远端 API）
- 不做版本管理（先用 `current_version = 1`）
- 验收：`avc persona new Yu → avc render video --persona yu --topic hi` 出一个能看的 mp4

### Phase 1 — Provider 矩阵 + 持续训练（8 周）
- 形象（商用）：kling_avatar / heygen_avatar / doubao_image / seedream
- 形象（开源-via-API）：replicate_flux_lora / hf_inference
- 声音：volc_tts / azure_tts / elevenlabs / doubao_tts / openai_tts
- LLM：openai_compat
- 视频：kling / doubao_seedance / pika / runway / replicate_cogvideox
- 微调：openai_compat_sft / replicate_trainer / kling_avatar_finetune
- 多版本 + 漂移评估 + 切版本 + 强制回滚
- 反馈回灌（手动 + 自动）

### Phase 2 — 可选插件能力（4 周）
- **对象存储插件**（`avc storage plugin install s3`）—— 默认仍推荐本地 FS，本插件只在空间 / 共享成为瓶颈时启用
- OpenTelemetry 可选导出
- 训练并行（同 persona 多 base / 多 worker）
- 评测集 / canary 样本管理

### Phase 3 — 平台化扩展（不属本仓范围）
- Web 控制台、模板市场、A/B 实验、多租户 SaaS — 由独立上层项目承担
- AVCore 始终保持**纯 CLI 核心**

---

## 10. 风险与对策

| 风险 | 影响 | 对策 |
|------|------|------|
| Provider 限速 / 涨价 | 吞吐 / 成本 | 多 Provider + 路由表（§5.2） |
| Provider token 失效 / 错误 | 任务中断 | preflight + 401 自动重试 + 自动告警 |
| 数字人合规 | 法务 | 强制 consent + 可关闭"真实人物复刻"开关 |
| 训练漂移 | 用户体验 | 漂移自动评估 + 回退 + drift_report |
| 版本错乱 | 团队 | 版本不可变 + 切版本原子化 |
| Provider API 变更 | 中断 | provider.json 多版本兼容；token preflight catch change |
| 模型效果不稳 | 口碑 | canary 样本 + 评测集 + 漂移告警 |
| 跨机迁移 | 体验 | `avc export / import` tar.zst 包 |

---

## 11. 后续阅读

- 设计：[design.md](./design.md)
- **资产存储（含目录树与 SQLite 表示例）：**[storage.md](./storage.md) ⭐
- CLI / REPL 用法：[cli.md](./cli.md)
- 子模块详细设计：
  - [persona-modeling.md](./modules/persona-modeling.md)
  - [persona-evolution.md](./modules/persona-evolution.md)
  - [video-generation.md](./modules/video-generation.md)
  - [pipeline.md](./modules/pipeline.md)
  - [knowledge-aspect.md](./modules/knowledge-aspect.md)
- Provider / API 参考：[api/README.md](./api/README.md)
