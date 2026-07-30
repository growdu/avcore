# AVCore 架构文档（Architecture Document）

> 回答"用什么技术、怎么组织代码、怎么打包、怎么演进"。配套设计文档 [`design.md`](./design.md)。

---

## 1. 一句话总结

AVCore = **Rust 单二进制 CLI + 本地 SQLite + 本地文件系统 + 统一 DAG Pipeline + 一组 trait 化的 Provider 适配器**。
不暴露 HTTP / GRPC 服务，不做 SaaS 控制台，不内嵌计费 / 可观测性 dashboard——把这些都交给外部系统。

---

## 2. 顶层形态

```
┌──────────────────────────────────────────────────────────────────┐
│  avc  (Rust 单二进制)                                              │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │  CLI (clap)       REPL (rustyline + completer)              │ │
│  └─────────────────────────────────────────────────────────────┘ │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │  Core Services                                               │ │
│  │   persona-svc  evolution-svc  script-svc  asset-svc           │ │
│  │   pipeline-svc(DAG)  task-svc  job-svc                       │ │
│  └─────────────────────────────────────────────────────────────┘ │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │  Provider Adapters (trait + 动态加载)                          │ │
│  │   avatar / voice / llm / video / knowledge / storage         │ │
│  └─────────────────────────────────────────────────────────────┘ │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │  Storage Layer                                               │ │
│  │   SQLite (rusqlite) + 文件系统 (tokio fs)                     │ │
│  └─────────────────────────────────────────────────────────────┘ │
└─────────┬─────────────────────────┬────────────────────────────┘
          │                         │
          ▼                         ▼
  ~/.local/share/avc/         Provider endpoints (HTTP/gRPC)
  ├── avc.db                  ├── 自托管 SDXL / CosyVoice ...
  ├── personas/pm_*/vN/...    └── 商用 API（Kling / 即梦 / 豆包 ...）
  ├── media/jobs/...
  └── cache/...
```

进程模型：**单进程多任务**——长任务走 tokio task，不起子进程。需要隔离 Provider 时可 fork 子进程（仅当 Provider SDK 必须独立运行时）。

---

## 3. Cargo Workspace

```
avcore/
├── Cargo.toml                     # workspace
├── crates/
│   ├── avc/                       # 二进制入口：CLI + REPL
│   ├── core/                      # 领域类型 + 服务 trait
│   │   └── src/{persona, evolution, script, pipeline, asset, task}.rs
│   ├── pipeline/                  # DAG 解析 / 调度 / 节点执行器
│   ├── storage/                   # SQLite + 文件系统布局（参见 storage.md）
│   │   └── src/{db.rs, fs.rs, migrations.rs}
│   ├── providers/                 # trait 定义
│   │   └── src/{avatar, voice, llm, video, knowledge}.rs
│   ├── providers-impl/            # 具体实现（一个 Provider 一个文件）
│   │   └── src/{sdxl, cosyvoice, kling, openai_compat, ...}.rs
│   ├── renderer/                  # 视频渲染高层逻辑（合成/字幕/水印等）
│   └── eval/                      # 漂移评估、评测集、canary 样本
├── assets/
│   └── providers/                 # 每个 provider 的 provider.json 示例
├── docs/                          # 本文档站
└── site/                          # mkdocs 构建产物
```

### 关键依赖

| Crate | 用途 |
|-------|------|
| `tokio` (full) | 异步运行时 |
| `clap` | CLI 解析 |
| `rustyline` | REPL |
| `rusqlite` (bundled) | SQLite |
| `reqwest` (rustls) | Provider HTTP 调用 |
| `serde` / `serde_json` | 序列化 |
| `tracing` + `tracing-subscriber` | 日志（可选 OTel 导出） |
| `thiserror` / `anyhow` | 错误 |
| `prometheus` / `tracing-opentelemetry` | 可选 |
| `zstd` / `tokio-tar` | import/export 打包 |

---

## 4. 技术选型

| 维度 | 选型 | 理由 |
|------|------|------|
| 主语言 | **Rust** | 启动快、类型强、零外部依赖、单二进制；AI 主力语言虽在 Python，但 AVCore 不重算法重编排 |
| 异步运行时 | tokio | 生态最广、Provider HTTP/WS 通杀 |
| HTTP 客户端 | reqwest (rustls) | 跨平台无需 OpenSSL |
| DB | SQLite (rusqlite, bundled) | 单文件、零运维、足够撑单租户 / 单团队 |
| 对象存储 | 本地文件系统 → S3/OSS 通过 trait | 默认本地，可选迁移 |
| CLI 框架 | clap v4 (derive) | 工业标准、文档自动生成 |
| REPL | rustyline | 多行 + 历史 |
| 日志 | tracing | 结构化、可选 OTel |
| 可观测性 | 仅 tracing 日志；不内置 dashboard | 用户自行接 OTel collector |
| Provider 集成 | trait + 动态子进程加载（plugins 后续） | 主仓不绑模型厂商 |

> **故意不引入** Python 即使它是 AI 主力——AVCore 是**编排层**，算法侧已经在各家 Provider 内部，AVCore 只需要 HTTP 通它们。

---

## 5. 数据模型（精简版）

```sql
-- 顶层 persona
CREATE TABLE persona_models (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  archetype     TEXT,
  description   TEXT,
  current_version INTEGER NOT NULL,
  status        TEXT NOT NULL,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);

-- 版本（不可变快照，账本）
CREATE TABLE persona_versions (
  persona_model_id TEXT NOT NULL,
  version          INTEGER NOT NULL,
  parent_version   INTEGER,
  dir_path         TEXT NOT NULL,
  status           TEXT NOT NULL,
  training_job_id  TEXT,
  manifest_json    TEXT NOT NULL,
  created_at       TEXT NOT NULL,
  PRIMARY KEY (persona_model_id, version)
);

-- 训练样本池
CREATE TABLE persona_samples (
  id                       TEXT PRIMARY KEY,
  persona_model_id         TEXT NOT NULL,
  kind                     TEXT NOT NULL,
  uri_or_text              TEXT,
  version_id_at_collection INTEGER,
  consent_proof            TEXT,
  tags_json                TEXT,
  quality_score            REAL,
  created_at               TEXT NOT NULL
);

-- 训练任务
CREATE TABLE training_jobs (
  id                   TEXT PRIMARY KEY,
  persona_model_id     TEXT NOT NULL,
  base_version         INTEGER NOT NULL,
  target_version       INTEGER,
  scope_json           TEXT NOT NULL,
  config_json          TEXT,
  status               TEXT NOT NULL,
  result_version       INTEGER,
  drift_report_json    TEXT,
  started_at           TEXT,
  finished_at          TEXT
);

-- 渲染任务（绑定版本，不漂移）
CREATE TABLE jobs (
  id                  TEXT PRIMARY KEY,
  script_id           TEXT,
  persona_model_id    TEXT NOT NULL,
  persona_version     INTEGER NOT NULL,
  status              TEXT NOT NULL,
  options_json        TEXT,
  artifacts_json      TEXT,
  created_at          TEXT,
  finished_at         TEXT
);

-- 知识语料（可选）
CREATE TABLE knowledge_corpora ( id, name, source_type, language, chunk_count, index_version, ... );
CREATE TABLE corpus_chunks ( id, corpus_id, ordinal, content, token_count, deprecated, meta_json );
```

完整布局见 [`storage.md §8`](./storage.md)。

---

## 6. 模块边界

| 模块 | 谁负责 | 不负责 |
|------|--------|--------|
| [persona-modeling](./modules/persona-modeling.md) | v1 创建 | 训练 / 渲染 |
| [persona-evolution](./modules/persona-evolution.md) | 训练 / 版本 / 漂移兜底 | v1 创建、渲染 |
| [video-generation](./modules/video-generation.md) | 出片 | 训练、人设设计 |
| [pipeline](./modules/pipeline.md) | DAG 节点 / 调度 / 重试 / 断点 | 具体 Provider |
| [knowledge-aspect](./modules/knowledge-aspect.md) | 语料 / RAG | 形象 / 声音 / 人设 |

---

## 7. Provider 抽象

```rust
#[async_trait]
pub trait AvatarProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn create(&self, spec: &AvatarSpec) -> Result<Avatar>;
    async fn render(&self, avatar: &Avatar, prompt: &str, mot: &Motion) -> Result<Media>;
    async fn finetune(&self, base: &Avatar, samples: &[Sample], cfg: &TrainCfg) -> Result<Avatar>;
}

pub trait VoiceProvider {
    async fn clone(&self, samples: &[Audio]) -> Result<Voice>;
    async fn synth(&self, voice: &Voice, text: &str, ssml: &Ssml) -> Result<Audio>;
    async fn finetune(&self, base: &Voice, samples: &[Sample], cfg: &TrainCfg) -> Result<Voice>;
}

pub trait LlmProvider { /* chat / sft */ }
pub trait VideoProvider { /* render(i2v) */ }
pub trait KnowledgeProvider { /* chunk / embed / search */ }
```

每个 Provider 一份 `provider.json`：

```json
{
  "name": "kling_avatar",
  "kind": "avatar",
  "version": "v1.2",
  "config_schema": { ... },
  "endpoint": "https://api.kling.ai/...",
  "limits": { "max_refs": 6, "max_lora_size_mb": 250 }
}
```

新增 Provider = 新建 trait 实现 + 一个 `provider.json`；主仓无需修改。

---

## 8. 单二进制设计的好处

| 关心 | 体现 |
|------|------|
| 启动 | 亚秒级，REPL 交互不卡 |
| 部署 | `cargo install avc` 完事；无 Docker 也能跑 |
| 升级 | `avc self update`（后续） |
| CI | 只测一个 binary |
| 静态分发 | `--target x86_64-unknown-linux-musl` |

---

## 9. DAG Pipeline

统一的 DAG 模型同时支撑训练与渲染。节点元类型见 [`modules/pipeline.md`](./modules/pipeline.md)；节点类型新增例子：

| 节点类型 | 用于 |
|----------|------|
| `script_gen`, `tts`, `img_gen`, `i2v`, `compose`, `encode` | 渲染 DAG |
| `sample_filter`, `persona_train_avatar`, `persona_train_voice`, `anchor_extract`, `drift_eval`, `publish_or_rollback` | 训练 DAG |
| `corpus_chunk`, `corpus_embed`, `corpus_index` | 知识重建 |

调度：
- 内存 + tokio task 即可起步（千级并发内）
- 节点失败重试 + 节点结果落盘 → 进程重启后续跑
- 节点完成事件写 SQLite `node_steps` 表

---

## 10. 失败模式与恢复

| 场景 | 行为 |
|------|------|
| Provider 限速 | 切备 / 退避重试 / 失败入 `drift_report` |
| 网络中断 | 节点 retry，超过阈值标 `failed` |
| 磁盘满 | 写入前预检查；缺空间直接 abort |
| 资产 sha 不匹配 | `asset_corrupted`，禁止用此版本渲染 |
| 进程崩溃 | 启动时扫描 `node_steps`，把 `running → pending` 续跑 |
| 版本漂移 | 训练任务自动回退到 base version + 报告 |

---

## 11. 打包与分发

- 二进制：musl 静态链接，~ 30 MB
- 镜像（可选）：`Dockerfile` 用 distroless 装二进制
- Homebrew / scoop / cargo / apt 等由后续 CI 扩展；Phase 0 主打 `cargo install`

---

## 12. 关键技术决策（ADR 摘要）

| 编号 | 决策 | 备选 | 理由 |
|------|------|------|------|
| ADR-001 | **Rust + 单二进制** 为主仓 | Python 服务 / Go / Node | 启动快、类型强、与"CLI 优先"对齐 |
| ADR-002 | **SQLite + 本地文件系统** 起手 | Postgres / MinIO | 零运维、可拷走、单用户足够 |
| ADR-003 | **DAG 节点编排引擎自研** | Temporal / Argo | 起步要轻、可演进 |
| ADR-004 | **Provider 通过 trait 抽象** + 内置实现 | 配置化插件框架 | 内置足够简单；插件框架可后续加 |
| ADR-005 | **PersonaModelVersion 不可变** | 可变 + 软删 | 历史视频必须稳定 |
| ADR-006 | **训练任务独占（同一 persona 不并发）** | 并行训练 | 防止版本冲突 |
| ADR-007 | **不内嵌计费 / 可观测性 dashboard** | 内嵌 SaaS 化 | 与开源核心定位冲突 |
| ADR-008 | **CLI + REPL 双形态** | 仅 CLI / 仅 REPL | 自动化 + 探索各有需求 |
| ADR-009 | **Provider 通过 HTTP/gRPC，不在本仓跑模型** | 内置本地 GPU | 主仓是编排层，模型在 Provider |

---

## 13. 演进路线

### Phase 0 — 最小闭环（4 周）
- `avc persona new` 跑通：v1 生成（一个 avatar provider + 一个 voice provider）
- `avc render video` 跑通：1 条视频（脚本 + tts + i2v + compose）
- 不做版本管理（先用 `current_version = 1`）
- 验收：`avc persona new Lily → avc render video --persona lily --topic hi` 出一个能看的 mp4

### Phase 1 — Provider 矩阵 + 持续训练（8 周）
- 形象：sdxl / kling avatar / heygen / flux lora
- 声音：cosyvoice / gpt-sovits / volc tts
- LLM：openai 兼容接口
- 视频：kling / cogvideox / animatediff
- 多版本 + 漂移评估 + 切版本 + 强制回滚
- 反馈回灌（手动 + 自动）

### Phase 2 — 可选插件能力（4 周）
- `avc storage plugin install s3`（对象存储备份）
- OpenTelemetry 可选导出（接 collector）
- 训练并行（同 persona 多 base / 多 worker）
- 评测集 / canary 样本管理

### Phase 3 — 平台化扩展（不属本仓范围）
- Web 控制台、模板市场、A/B 实验、多租户 SaaS —— 由独立上层项目承担
- AVCore 始终保持**纯 CLI 核心**

---

## 14. 风险与对策

| 风险 | 影响 | 对策 |
|------|------|------|
| Provider 限速 / 涨价 | 吞吐 / 成本 | 多 Provider + 路由表 |
| 数字人合规 | 法务 | 强制 consent + 可关闭"真实人物复刻"开关 |
| 训练漂移 | 用户体验 | 漂移自动评估 + 回退 + drift 报告 |
| 版本错乱 | 团队 | 版本不可变 + 切版本原子化 |
| 训练 GPU 成本 | 团队 | 本地默认 CPU friendly 训练 / 可选 GPU 加速 |
| 模型效果不稳 | 口碑 | canary 样本 + 评测集 + 漂移告警 |
| 跨机迁移 | 体验 | `avc export / import` tar.zst 包 |
| 进程崩溃丢失任务 | 体验 | 节点级落盘 + 重启续跑 |

---

## 15. 后续阅读

- 设计：[design.md](./design.md)
- 资产存储：[storage.md](./storage.md) ⭐
- CLI / REPL 用法：[cli.md](./cli.md)
- 子模块详细设计：[modules/README.md](./modules/README.md)
- Provider / API 参考：[api/README.md](./api/README.md)
