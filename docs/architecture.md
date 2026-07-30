# AVCore 架构文档（Architecture Document）

> 回答"用什么技术、怎么组织、怎么部署、怎么演进"。配套设计文档 [`design.md`](./design.md)。

---

## 1. 架构总览

### 1.1 一句话总结

AVCore = **API 网关 + 业务服务（persona / training / script / render / notify / asset）+ AI Provider 适配层 + 统一 DAG 工作流引擎 + 任务调度 + 资产存储**，
由一条 **DAG Pipeline** 同时支撑"模型生成 / 模型持续训练 / 视频渲染"三条链路。

### 1.2 逻辑视图

```
                        ┌──────────────────────────────────────┐
                        │            API Gateway              │
                        │  REST / gRPC / WebSocket / Webhook  │
                        └──────────────────┬───────────────────┘
                                           │
        ┌─────────────┬────────────┬───────┴───────┬──────────────┬───────────┐
        ▼             ▼            ▼               ▼              ▼           ▼
   ┌─────────┐  ┌─────────────┐ ┌──────────┐ ┌──────────┐  ┌──────────┐ ┌──────────────┐
   │ Persona │  │  Evolution  │ │ Script   │ │ Pipeline │  │   Asset  │ │   Notify     │
   │ Service │  │   Service   │ │ Service  │ │ Service  │  │ Service  │ │   Service    │
   │(v1 init)│  │(再训练)    │ │(分镜)   │ │(统一 DAG)│  │          │ │ (Webhook)    │
   └────┬────┘  └────┬────────┘ └────┬─────┘ └────┬─────┘  └────┬─────┘ └──────┬───────┘
        │            │               │             │             │              │
        └────────────┴───────────────┴──────┬──────┴─────────────┴──────────────┘
                                            │
                                ┌───────────▼────────────┐
                                │   AI Provider 层       │ ← 抽象所有模型
                                │ Avatar/Voice/LLM       │
                                │ /Video/RAG/Knowledge   │
                                └────────────┬───────────┘
                                             │
        ┌────────────────────────┬───────────┼─────────────────────────┬──────────────────┐
        │                        │           │                         │                  │
   ┌────▼─────┐  ┌────────────────▼─┐ ┌───────▼────────┐  ┌──────────────┐  ┌──────▼──────┐
   │ Object   │  │ Vector DB         │ │  Task          │ │ Model        │ │ Cache/      │
   │ Storage  │  │ (pgvector/Milvus) │ │  Queue         │ │ Gateway      │ │ Redis       │
   │ (S3/OSS) │  │ + 样本库          │ │ (Redis/Kafka)  │ │(内部 LLM 路由)│ │             │
   └──────────┘  └───────────────────┘ └────────────────┘ └──────────────┘  └─────────────┘
```

服务职责：

| 服务 | 职责 |
|------|------|
| `persona-svc` | 创建 `PersonaModel + v1`（形象/声音/人设/知识） |
| `evolution-svc` | 训练任务管理、样本治理、版本发布、漂移评估、一致性兜底 |
| `script-svc` | 根据 persona + topic 生成分镜；支持编辑 |
| `pipeline-svc` | 统一 DAG 调度（视频渲染 + 训练都跑这层） |
| `asset-svc` | 形象 / 声音 / BGM / 模板 等资产的统一管理 |
| `notify-svc` | Webhook / WebSocket 触达 |

### 1.3 物理部署视图

```
              ┌──────────────── Control Plane ────────────────┐
              │  API Gateway, Console/Admin, Auth, Metering  │
              └───────────────────────┬──────────────────────┘
                                      │
              ┌───────────────────────▼──────────────────────────┐
              │           业务服务（Stateless）                   │
              │   persona-svc, evolution-svc, script-svc,        │
              │   pipeline-svc, asset-svc, notify-svc           │
              └───────────────────────┬──────────────────────────┘
                                      │
   ┌──────────────────────────────────┼──────────────────────────────────────┐
   │                       Worker Plane（按 DAG 节点类型分池）                │
   │   ┌───────────┐ ┌───────────┐ ┌────────────┐ ┌────────────┐ ┌───────┐  │
   │   │ llm-pool  │ │ tts-pool  │ │ img-pool   │ │video-pool  │ │train- │  │
   │   │ (CPU/GPU) │ │ (GPU)     │ │ (GPU)      │ │ (GPU)      │ │pool   │  │
   │   │           │ │           │ │            │ │            │ │(GPU)  │  │
   │   └───────────┘ └───────────┘ └────────────┘ └────────────┘ └───────┘  │
   │   ┌───────────┐ ┌───────────┐ ┌────────────┐ ┌────────────┐             │
   │   │ rag-pool  │ │ lipsync   │ │ compose    │ │ eval-pool  │             │
   │   │ (CPU)     │ │ (GPU)     │ │ (CPU)      │ │ (GPU)      │             │
   │   └───────────┘ └───────────┘ └────────────┘ └────────────┘             │
   └─────────────────────────────────────────────────────────────────────────┘
                                      │
        ┌─────────────────────────────┼─────────────────────────────┐
        │                             │                             │
   ┌────▼─────┐  ┌───────────────┐ ┌───▼────────────┐  ┌──────────▼────────┐
   │ Postgres │  │ Vector DB     │ │ Object Storage │  │ Redis / Kafka    │
   │ 元数据/步骤│  │ (语料/记忆)   │ │ (资产/产物)    │  │ (队列/缓存/锁)    │
   └──────────┘  └───────────────┘ └────────────────┘  └───────────────────┘
```

---

## 2. 技术选型

| 维度 | 选型 | 理由 |
|------|------|------|
| 主语言 | Python（业务）+ Rust/Go（热路径） | AI 生态最丰富 |
| Web 框架 | FastAPI | 异步、生态、自动 OpenAPI |
| 任务队列 | Celery / Dramatiq → 可演进 Temporal | 起步轻、后期可换 |
| 元数据库 | PostgreSQL | JSONB 支持好、单库起步后期再分 |
| 向量检索 | pgvector → Milvus（>1 亿迁移） | 渐进式复杂度 |
| 资产存储 | S3 / OSS | 标准 |
| 训练框架 | Transformers / PEFT / LoRA + 自研编排 | SFT 与偏好对齐走业界方案 |
| 模型路由 | 自研 Model Gateway，OpenAI-compatible | 让集成方零修改接入 |
| GPU 调度 | K8s + Volcano / 简单标签选择 | 多租户公平 |

---

## 3. 数据模型（精简版）

### 3.1 主表
```
persona_models(id, name, archetype, current_version_id, status, ...)
persona_versions(id, persona_model_id, version, parent_version_id,
                 avatar_id, voice_id, persona_id, knowledge_id,
                 identity_anchor_id, metrics_id, status, ...)
avatars(id, provider, primary_image, face_id, lora_uri, ...)
voices(id, provider, voice_id, sample_uri, language, ...)
personas(id, traits, tone, catchphrases, taboos, scenario_prompts, ...)
knowledge_bindings(id, persona_version_id, corpus_ids, domain, grounding_mode, ...)
identity_anchors(id, face_emb, voice_emb, style_emb, ...)
persona_samples(id, persona_model_id, kind, uri/text, version_id_at_collection, ...)
training_jobs(id, persona_model_id, base_version_id, target_version, scope[],
              config, status, result_version_id, metrics, ...)
scripts(id, persona_model_id, persona_version_id, topic, scenes[], ...)
jobs(id, script_id, persona_version_id, status, options, artifacts, ...)
job_steps(id, job_id, node_id, status, attempt, outputs, artifacts, error, ...)
```

### 3.2 不可变性约束
- `persona_versions` 中所有资产都是**不可变快照**：一旦写入，永不更新
- 老资产可"标 deprecated" 不可删
- 所有跨版本数据都用 `parent_version_id` 关联，构成多版本树

---

## 4. 模块边界再确认

| 模块 | 谁负责 | 不负责 |
|------|--------|--------|
| [persona-modeling](./modules/persona-modeling.md) | v1 创建 | v1 之后的事 |
| [persona-evolution](./modules/persona-evolution.md) | 训练 / 版本 / 一致性兜底 | v1 创建、脚本、渲染 |
| [video-generation](./modules/video-generation.md) | 用 PersonaModelVersion 出片 | 训练剧本、模型微调 |
| [pipeline](./modules/pipeline.md) | 节点编排 / 调度 / 重试 | 具体模型调用 |
| [knowledge-aspect](./modules/knowledge-aspect.md) | 语料 / 检索 | 形象 / 声音 / 人设 / 训练 |

---

## 5. 端到端调用链

### 5.1 视频生成
```
Client  →  API Gateway  →  script-svc  (build script)
                          →  pipeline-svc (submit run)
                              →  worker pools: llm / tts / img / video / compose
                          →  notify-svc (webhook / ws)
Client  ←  artifacts
```

### 5.2 持续训练
```
Client  →  evolution-svc (submit training_job)
                          →  pipeline-svc (persona.train.v1 DAG)
                              →  worker pools: train / eval
                          →  publish v(N+1) or rollback
Client  ←  training_report
```

---

## 6. 多租户与版本配额

- 每个 `PersonaModel` 属于一个 tenant
- tenant 可配置：同时训练任务数、最大月度训练时长、最大版本数
- `train-pool` 按 tenant 配额调度（fair-share）

---

## 7. Provider 抽象

每个能力点（形象 / 声音 / LLM / 视频 / 知识）以 `Provider` 协议暴露，新增模型只需实现：

```python
class PersonaTrainAvatarProvider(Protocol):
    def finetune(self, base_avatar: Avatar, samples: list[Sample], config: dict) -> Avatar: ...
    def eval_consistency(self, base: Avatar, candidate: Avatar, anchors: list[Sample]) -> float: ...
```

注册到 `Model Gateway` 即可被 pipeline 发现与路由。

---

## 8. 数据流

### 8.1 视频渲染
```
topic + persona_version ──▶ LLM (build script, RAG-augmented) ──▶ Script
                                                       │
                                                       ▼
                                 TTS（voice.synthesize）───┐
                                                       ▼
                                 KeyFrame（avatar.render）─┐
                                                       ▼
                                 i2v（video.render）      │
                                                       ▼
                                 Lipsync (optional)       │
                                                       ▼
                                 Compose（bgm/sub/wmark）──▶ final.mp4
                                                            (meta 含 persona_version_id)
```

### 8.2 训练流水线
```
samples + base_version ──▶ Filter (quality, dedup) ──▶ Train (per scope)
                                                       │
                                                       ▼
                                          Identity Anchor (extract)
                                                       │
                                                       ▼
                                          Drift Eval (vs base) ──▶ branch
                                                       │
                                          ┌────────────┴────────────┐
                                          ▼                         ▼
                                   publish v(N+1)             rollback + drift_report
```

---

## 9. API 形态

完整 API 见 [`api/README.md`](./api/README.md)。概览：

```http
# PersonaModel 顶层
POST   /v1/persona-models                       创建 (异步)
GET    /v1/persona-models/{id}                  查询
GET    /v1/persona-models/{id}/versions         版本列表
PUT    /v1/persona-models/{id}/current-version  切换默认版本

# 训练
POST   /v1/persona-models/{id}/samples
POST   /v1/persona-models/{id}/training-jobs
GET    /v1/training-jobs/{jid}/report

# 视觉 / 声音 / 人设 / 知识
POST   /v1/persona-models/{id}/avatars
POST   /v1/persona-models/{id}/voices
POST   /v1/persona-models/{id}/persona
POST   /v1/corpora
POST   /v1/persona-models/{id}/knowledge

# 脚本与视频
POST   /v1/scripts
PUT    /v1/scripts/{id}
POST   /v1/jobs
POST   /v1/jobs/{id}/feedback

# Webhook / WS
POST   /v1/webhooks
WS     /v1/ws/jobs
WS     /v1/ws/training-jobs
```

---

## 10. 安全架构

- **认证**：API Key（HMAC） / OAuth2 / OIDC
- **授权**：RBAC + 租户隔离
- **传输**：全链路 TLS，内部 mTLS
- **存储**：对象存储 SSE-KMS 加密；DB 字段级加密（声音授权文件）
- **审计**：所有写操作 + 模型调用入审计日志
- **审核**：文本前置审核 + 输出审核（命中策略 → 拦截 / 打码）
- **反滥用**：行为风控、人机验证
- **凭据管理**：HashiCorp Vault / 云厂商 KMS
- **真实人物复刻**：默认禁止；开启需走额外合规审核

---

## 11. 可观测性

| 维度 | 工具 | 关键指标 |
|------|------|----------|
| Trace | OpenTelemetry → Tempo / Jaeger | 全链路 DAG span（按 node_id 切分） |
| Metric | Prometheus | QPS / 延迟 / 错误率 / GPU 利用率 / 队列积压 / 漂移分 |
| Log | Loki / ELK | 结构化 JSON 日志 |
| Event | 业务事件流 | 任务状态变更、训练发布、版本停用、漂移告警 |
| 告警 | Alertmanager | GPU 利用率、SLO 违反、Provider 错误率、漂移超阈值 |

每个 Job 自动注入 `trace_id` / `tenant_id` / `persona_model_id` / `persona_version_id`。

---

## 12. 部署架构

### 12.1 K8s 拓扑
```
Namespace: avcore
├── api-gateway              (Deployment × 3, HPA)
├── persona-svc              (Deployment × 2)
├── evolution-svc            (Deployment × 2)
├── script-svc               (Deployment × 2)
├── pipeline-svc             (Deployment × 3, Leader Election)
├── notify-svc               (Deployment × 2)
├── workers/
│   ├── llm-pool
│   ├── tts-pool             (GPU)
│   ├── img-pool             (GPU)
│   ├── video-pool           (GPU)
│   ├── train-pool           (GPU)         ← 持续训练
│   ├── eval-pool            (GPU)         ← 漂移评估
│   ├── rag-pool             (CPU)
│   ├── lipsync-pool         (GPU)
│   └── compose-pool         (CPU)
├── postgres                 (StatefulSet 或托管)
├── redis                    (StatefulSet 或托管)
├── minio                    (StatefulSet 或对接 OSS)
└── milvus / pgvector        (Helm 或内置)
```

### 12.2 多环境
- `dev` / `staging` / `prod`，按 namespace 隔离
- 模型版本通过 ConfigMap / Admin API 灰度

### 12.3 灰度与回滚
- **API 层**：按租户灰度（Header 路由）
- **模型层**：按租户配置 Provider 优先级
- **PersonaModel 版本**：切 `current_version` 即可让新任务走新版本；老任务仍绑老版本不受影响
- **代码层**：标准 K8s 滚动升级 / Argo Rollouts

---

## 13. 关键技术决策（ADR 摘要）

| 编号 | 决策 | 备选 | 理由 |
|------|------|------|------|
| ADR-001 | 元数据用 PostgreSQL 单库起步 | 多库分库 | 早期降低运维，热点数据后期再分 |
| ADR-002 | 向量检索 pgvector 起步，>1 亿迁移 Milvus | 起步 Milvus | 渐进式复杂度 |
| ADR-003 | Python 为主，Go/Rust 仅用于热路径 | 全 Go | AI 生态 |
| ADR-004 | 自研轻量 DAG，再评估 Temporal | 直接 Temporal | 早期避免重 SDK |
| ADR-005 | Provider 抽象通过 Protocol / Interface | 强耦合调用 | 多厂商 + 灰度 |
| ADR-006 | 任务状态可由前端轮询 / WS / Webhook | 仅 Webhook | 适配不同集成方 |
| ADR-007 | 渲染任务支持节点级断点续跑 | 任务级 | 长链路降本 |
| ADR-008 | PersonaModelVersion 不可变 | 可变 + 软标记 | 历史视频必须锁定版本 |
| ADR-009 | 训练任务独占（同一 persona 不并发） | 并行训练 | 防止版本错乱 |

---

## 14. 演进路线

### Phase 0 — 最小闭环（4 周）
- `persona-svc` + `evolution-svc` 雏形：1 个 persona → v1 → 1 条视频 → 手动反馈
- 单体服务 + 单一视频 Provider
- 不做版本管理（先跑通）

### Phase 1 — 多 Provider + 版本机制（8 周）
- 实现 v1 / v2 不可变快照 + 切版本
- Provider 抽象落地 ≥ 3 个
- 知识语料 + RAG 接入（knowledge-aspect）
- 训练 DAG 跑通：含漂移评估
- 任务系统、Webhook、可观测

### Phase 2 — 高可用 + 持续运营（8 周）
- 拆微服务、K8s 化
- 队列分层、GPU 调度、训练独立池
- 模型路由、降级、缓存
- A/B 流量分配、强制回滚
- 反馈闭环自动化

### Phase 3 — 平台化（持续）
- 模板市场、A/B 实验
- 多租户 SaaS、计费 / 配额
- 数字员工 / 直播等扩展场景
- 实时交互数字人（可对话）

---

## 15. 风险与对策

| 风险 | 影响 | 对策 |
|------|------|------|
| 模型厂商限速 / 涨价 | 吞吐 / 成本 | 多 Provider + 路由 |
| 长链路失败 | 用户体验 | 节点级断点续跑 + 重试 |
| 数字人 / 声音合规 | 法务 | 强制授权 + 审核 + 水印 + 真实人物复刻额外审核 |
| 训练漂移 | 用户体验 | 漂移自动评估 + 回退 + 告警 |
| 版本管理混乱 | 团队 | PersonaModelVersion 不可变 + 切版本原子化 |
| 训练 GPU 成本失控 | 毛利 | 弹性 + 配额 + 限额 + 预估提示 |
| 模型效果不稳 | 口碑 | 评测体系 + 人工抽检 + 反馈闭环 |
| 历史视频与新版 persona 不一致 | 体感 | 历史视频固定 version_id，不跟随默认漂移 |

---

## 16. 后续阅读

- 子模块详细设计：[docs/modules/](./modules/README.md)
- API 详细说明：[docs/api/](./api/README.md)
- 设计文档：[design.md](./design.md)
