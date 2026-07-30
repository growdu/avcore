# AVCore 架构文档（Architecture Document）

> 回答"用什么技术、怎么组织、怎么部署、怎么演进"。配套设计文档 [`design.md`](./design.md)。

---

## 1. 架构总览

### 1.1 一句话总结

AVCore = **API 网关 + 业务服务 + AI Provider 适配层 + 工作流引擎 + 任务调度 + 资产存储**，
由一条 **DAG Pipeline** 串起"造角 → 养成 → 拍戏"全流程。

### 1.2 逻辑视图

```
                        ┌──────────────────────────────────────┐
                        │            API Gateway              │
                        │  REST / gRPC / WebSocket / Webhook  │
                        └──────────────────┬───────────────────┘
                                           │
        ┌─────────────┬────────────┬───────┴───────┬──────────────┐
        ▼             ▼            ▼               ▼              ▼
   ┌─────────┐  ┌──────────┐ ┌──────────┐  ┌──────────┐  ┌──────────────┐
   │Character│  │ Knowledge│ │ Script   │  │ Pipeline │  │   Asset      │
   │ Service │  │ Service  │ │ Service  │  │ Service  │  │   Service    │
   └────┬────┘  └────┬─────┘ └────┬─────┘  └────┬─────┘  └──────┬───────┘
        │            │            │             │               │
        └────────────┴────────────┴──────┬──────┴───────────────┘
                                        │
                              ┌─────────▼─────────┐
                              │   AI Provider 层  │  ←  抽象所有模型
                              │ Avatar/Voice/LLM  │
                              │    /Video/RAG     │
                              └─────────┬─────────┘
                                        │
        ┌───────────────────────────────┼────────────────────────────────┐
        │                               │                                │
   ┌────▼─────┐  ┌───────────────┐ ┌────▼──────┐  ┌───────────────┐ ┌────▼────┐
   │ Object   │  │ Vector DB     │ │  Task     │  │  Model        │ │ Cache/  │
   │ Storage  │  │ (Milvus/      │ │  Queue    │  │  Gateway      │ │ Redis   │
   │ (S3/OSS) │  │  pgvector)    │ │(Redis/Kafka)│ (内部 LLM 路由)│ │         │
   └──────────┘  └───────────────┘ └───────────┘  └───────────────┘ └─────────┘
```

### 1.3 物理部署视图

```
                ┌────────────── 控制面 Control Plane ──────────────┐
                │  API Gateway, Console/Admin, Auth, Metering     │
                └───────────────────────┬──────────────────────────┘
                                        │
                ┌───────────────────────▼──────────────────────────┐
                │             业务服务（Stateless）                │
                │   character-svc, knowledge-svc, script-svc,     │
                │   pipeline-svc, asset-svc                       │
                └───────────────────────┬──────────────────────────┘
                                        │
        ┌───────────────────────────────┼────────────────────────────────┐
        │                  任务执行面 Worker Plane                        │
        │   ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌────────────┐  │
        │   │ TTS Worker │ │ Avatar/    │ │ Video      │ │  RAG /     │  │
        │   │            │ │ Image Wrk  │ │ Worker     │ │  Embed Wrk  │  │
        │   └────────────┘ └────────────┘ └────────────┘ └────────────┘  │
        └─────────────────────────────────────────────────────────────────┘
                                        │
        ┌───────────────────────────────▼────────────────────────────────┐
        │                数据面 Data Plane                                │
        │  Postgres / Vector DB / Object Storage / Message Queue          │
        └─────────────────────────────────────────────────────────────────┘
```

---

## 2. 技术选型

| 层 | 选型 | 理由 |
|----|------|------|
| API 网关 | Kong / APISIX / 自研 Nginx+Lua | 限流、鉴权、租户路由 |
| 业务服务 | **Python（FastAPI）** + 关键路径 Go/Rust | AI 生态丰富；性能瓶颈用 Go/Rust 单点优化 |
| 异步任务 | **Celery + Redis**（小规模）/ **Temporal / Argo**（大规模） | 渐进式升级 |
| 消息队列 | Redis Streams → Kafka | 后期吞吐扩展 |
| 数据库 | PostgreSQL（元数据）+ pgvector（小规模向量）/ Milvus / Qdrant（大规模） | 一库多用起步 |
| 对象存储 | S3 兼容（MinIO / 阿里 OSS / AWS S3） | 存形象 / 声音 / 视频 / 临时帧 |
| 缓存 | Redis | 会话 / 限流 / 任务状态 |
| 工作流 | **自研轻量 DAG**（前期）+ Temporal（后期） | 控制粒度 |
| 监控 | Prometheus + Grafana + Loki + OpenTelemetry | 可观测 |
| 部署 | Docker / Kubernetes + Helm | 标准 |
| CI | GitHub Actions / GitLab CI | 标准 |
| 鉴权 | OAuth2 / OIDC + API Key | 标准化 |

---

## 3. 服务拆分

> 建议采用 **模块化单体起步 → 按瓶颈拆微服务** 的演进路径。

| 服务 | 职责 | 关键接口 |
|------|------|----------|
| **character-svc** | 角色 CRUD、形象 / 声音绑定、版本化 | `/characters`, `/characters/{id}` |
| **asset-svc** | 形象 / 声音 / BGM / 模板的元数据与引用 | `/assets`, `/assets/upload` |
| **knowledge-svc** | 语料管理、切分、向量化、检索 | `/corpora`, `/corpora/{id}/search` |
| **expert-svc** | 专家设定、绑定到角色、风格管理 | `/experts`, `/characters/{id}/expert` |
| **script-svc** | 脚本模板、LLM 生成分镜、脚本编辑 | `/scripts`, `/scripts/{id}/render` |
| **pipeline-svc** | 编排 DAG、任务编排与重试 | `/jobs`, `/jobs/{id}` |
| **render-svc** | 视频渲染后期合成（ffmpeg、字幕、转场） | 内网 gRPC |
| **meter-svc** | 计费、配额、审计 | 内网 |
| **notify-svc** | Webhook / WebSocket 进度推送 | `/webhooks`, `/ws` |
| **admin-svc** | 租户、Provider 配置、模型路由 | `/admin/*` |

---

## 4. 核心数据模型

### 4.1 元数据表（PostgreSQL）

```sql
-- 角色
CREATE TABLE characters (
    id              UUID PRIMARY KEY,
    tenant_id       UUID NOT NULL,
    name            TEXT NOT NULL,
    persona_id      UUID,
    avatar_id       UUID,
    voice_id        UUID,
    expert_id       UUID,
    status          TEXT NOT NULL,    -- draft/ready/failed
    version         INT NOT NULL DEFAULT 1,
    meta            JSONB,
    created_at      TIMESTAMPTZ,
    updated_at      TIMESTAMPTZ
);

-- 资产
CREATE TABLE assets (
    id              UUID PRIMARY KEY,
    tenant_id       UUID NOT NULL,
    type            TEXT NOT NULL,    -- avatar / voice / bgm / template / image
    provider        TEXT NOT NULL,    -- sdxl / kling / cosyvoice ...
    uri             TEXT NOT NULL,    -- 对象存储地址
    ref_id          TEXT,             -- 厂商侧 ID
    meta            JSONB,
    created_at      TIMESTAMPTZ
);

-- 知识语料
CREATE TABLE corpora (
    id              UUID PRIMARY KEY,
    tenant_id       UUID NOT NULL,
    name            TEXT NOT NULL,
    source          TEXT,             -- url/upload/api
    chunk_count     INT,
    index_version   INT,
    created_at      TIMESTAMPTZ
);

CREATE TABLE corpus_chunks (
    id              UUID PRIMARY KEY,
    corpus_id       UUID REFERENCES corpora(id),
    content         TEXT,
    embedding       VECTOR(1536),     -- pgvector
    meta            JSONB
);

-- 脚本
CREATE TABLE scripts (
    id              UUID PRIMARY KEY,
    tenant_id       UUID NOT NULL,
    character_id    UUID,
    template_id     UUID,
    topic           TEXT,
    scenes          JSONB,            -- Scene[]
    duration_ms     INT,
    created_at      TIMESTAMPTZ
);

-- 任务
CREATE TABLE jobs (
    id              UUID PRIMARY KEY,
    tenant_id       UUID NOT NULL,
    script_id       UUID,
    character_id    UUID,
    status          TEXT NOT NULL,    -- queued/running/succeeded/failed
    progress        INT DEFAULT 0,
    artifacts       JSONB,            -- 视频 URL / 封面 / 字幕
    error           JSONB,
    cost_points     INT,
    created_at      TIMESTAMPTZ,
    finished_at     TIMESTAMPTZ
);

-- 任务步骤（用于断点续跑 / 调试）
CREATE TABLE job_steps (
    id              UUID PRIMARY KEY,
    job_id          UUID REFERENCES jobs(id),
    name            TEXT,
    status          TEXT,
    input           JSONB,
    output          JSONB,
    started_at      TIMESTAMPTZ,
    finished_at     TIMESTAMPTZ,
    attempt         INT DEFAULT 1
);
```

### 4.2 资产存储布局

```
oss://{tenant_id}/
├── characters/{character_id}/
│   ├── avatar/{version}/
│   │   ├── portrait.png
│   │   ├── lora.safetensors
│   │   └── meta.json
│   ├── voice/{version}/
│   │   ├── sample.wav
│   │   ├── ref.pt
│   │   └── meta.json
│   └── jobs/{job_id}/
│       ├── scenes/{idx}.mp4
│       ├── audio/{idx}.wav
│       └── final.mp4
└── shared/
    ├── bgm/{id}.mp3
    └── templates/{id}.json
```

---

## 5. AI Provider 适配层

### 5.1 抽象接口

```python
# providers/base.py
from typing import Protocol, runtime_checkable

@runtime_checkable
class AvatarProvider(Protocol):
    name: str
    def create_avatar(self, spec: "AvatarSpec") -> "Avatar": ...
    def render_image(self, avatar: "Avatar", prompt: str, **kw) -> "Media": ...
    def render_video(self, avatar: "Avatar", audio: "Audio", motion: "Motion") -> "Clip": ...

@runtime_checkable
class VoiceProvider(Protocol):
    name: str
    def clone_voice(self, samples: list["Audio"], name: str) -> "Voice": ...
    def synthesize(self, voice: "Voice", text: str, ssml: "SSML"=None) -> "Audio": ...

@runtime_checkable
class LLMProvider(Protocol):
    name: str
    def chat(self, messages: list, tools: list = None, **kw) -> "LLMResponse": ...
    def embed(self, texts: list[str]) -> list[list[float]]: ...

@runtime_checkable
class VideoProvider(Protocol):
    name: str
    def render(self, scene: "Scene", avatar: "Avatar", audio: "Audio") -> "Clip": ...
```

### 5.2 Provider 路由

通过 **Model Gateway** 统一对外：

- 路由策略：按租户配置 / 成本 / 延迟 / 质量分
- 降级：A 厂商失败 → 自动切 B 厂商
- 限速：每租户每 Provider 配额
- 缓存：相同输入可命中语义缓存

### 5.3 Provider 实现清单（示例）

| 类型 | Provider | 用途 |
|------|----------|------|
| Avatar | SDXL + IP-Adapter / Flux | 文生图 + 形象一致性 |
| Avatar | HunyuanDiT / Qwen-Image | 中文场景形象 |
| Avatar | Kling Avatar / HeyGen / D-ID | 商用数字人 |
| Voice | CosyVoice / GPT-SoVITS / F5-TTS | 声音克隆 |
| Voice | 火山 / 阿里 / 微软 TTS | 商用音色 |
| LLM | GPT-4o / Claude / Qwen / DeepSeek | 脚本生成 / RAG |
| Video | Kling / 可灵 / CogVideoX / AnimateDiff | 镜头渲染 |
| Video | Sora / Veo / Hailuo | 高级模型 |
| Compose | ffmpeg + OpenCV | 拼接 / 转场 / 字幕 |

---

## 6. 工作流引擎（Pipeline）

### 6.1 DAG 描述

一次视频生成任务被建模为 DAG：

```
                  ┌──────────┐
                  │  script  │
                  └─────┬────┘
                        │ Script
        ┌───────────────┼───────────────┐
        ▼               ▼               ▼
  ┌─────────┐     ┌─────────┐     ┌──────────┐
  │ tts     │     │ bgm_sel │     │ img_gen  │
  └────┬────┘     └────┬────┘     └─────┬────┘
       │ Audio[]       │ BGM            │ Image[]
       └───────┬───────┴───────┬────────┘
               ▼               ▼
          ┌──────────┐   ┌──────────┐
          │  i2v     │   │  i2v     │  …per scene
          └─────┬────┘   └─────┬────┘
                │ Clip[]       │
                └──────┬───────┘
                       ▼
                 ┌──────────┐
                 │ compose  │
                 └─────┬────┘
                       ▼
                 ┌──────────┐
                 │ finalize │  → final.mp4
                 └──────────┘
```

### 6.2 引擎能力

- **DAG 解析**：JSON / YAML 描述
- **节点执行**：统一 Executor 接口
- **失败重试**：节点级 + 任务级
- **断点续跑**：每个节点执行前后持久化中间结果
- **并发控制**：可标注节点 fanout / 串行
- **可观测**：每节点产生 span / log / metric
- **人机协同**：脚本节点支持"人工编辑后再继续"

### 6.3 节点清单

| 节点 | 描述 | 耗时（P50） | 可并发 |
|------|------|------------|--------|
| `script_gen` | LLM 生成分镜 | 2s | 否 |
| `tts` | 旁白合成 | 3s/段 | 是 |
| `bgm_select` | BGM 匹配 | 0.5s | 是 |
| `img_gen` | 关键帧生成 | 5s/段 | 是 |
| `i2v` | 图生视频 | 30s/段 | 是 |
| `lipsync` | 口型同步 | 8s/段 | 是 |
| `compose` | 多镜头拼接 | 5s | 否 |
| `subtitle` | 字幕烧录 | 2s | 否 |
| `finalize` | 转封装 / 封面 | 1s | 否 |

---

## 7. 任务调度与并发

### 7.1 队列分层

```
inbound (HTTP) ─▶ control queue ─▶ per-tenant queue ─▶ worker pool
                                          │
                                          ▼
                                  GPU scheduler
                                  (per model pool)
```

### 7.2 GPU 调度

- 内部维护 **GPU 池**：按模型类型分组（SDXL 池 / Kling 池 / TTS 池 / 视频池）
- 调度器：FIFO + 优先级 + 抢占（可中断的低优任务）
- 弹性：K8s + 自定义 CRD，按队列积压自动扩缩

### 7.3 限流与配额

- 租户级：QPS / 并发任务数 / 月配额
- Provider 级：每厂商限速（防厂商配额超限）
- 用户级：API Key 维度

---

## 8. 数据架构

```
                ┌──────────────┐
                │ PostgreSQL   │  元数据 / 业务数据
                └──────┬───────┘
                       │
       ┌───────────────┼───────────────┐
       ▼               ▼               ▼
  ┌─────────┐    ┌──────────┐    ┌──────────┐
  │ pgvector│    │ Milvus / │    │ Object   │
  │  (RAG)  │    │ Qdrant   │    │ Storage  │
  └─────────┘    └──────────┘    └──────────┘
```

- **PostgreSQL**：业务主库，租户、角色、脚本、任务、计费
- **pgvector**：早期 RAG，与主库同库降低复杂度
- **Milvus / Qdrant**：向量规模 > 1 亿时迁移
- **对象存储**：所有大文件（图片、声音、视频、模型权重）
- **Redis**：会话、限流、Leaderboard、任务状态缓存

---

## 9. API 形态

详细 API 见 [`api/README.md`](./api/README.md)。概览：

```http
POST   /v1/characters              创建角色
GET    /v1/characters/{id}         查询角色
POST   /v1/characters/{id}/avatar  创建 / 更新形象
POST   /v1/characters/{id}/voice   创建 / 更新声音

POST   /v1/corpora                 创建语料
POST   /v1/corpora/{id}/chunks     追加 chunks
POST   /v1/corpora/{id}/search     检索

POST   /v1/scripts                 生成脚本
PUT    /v1/scripts/{id}            编辑脚本

POST   /v1/jobs                    创建视频生成任务
GET    /v1/jobs/{id}               查询任务
GET    /v1/jobs/{id}/steps         任务步骤
POST   /v1/jobs/{id}/retry         重试

POST   /v1/webhooks                注册回调
WS     /v1/ws/jobs                 实时进度
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

---

## 11. 可观测性

| 维度 | 工具 | 关键指标 |
|------|------|----------|
| Trace | OpenTelemetry → Tempo / Jaeger | 全链路 DAG span |
| Metric | Prometheus | QPS / 延迟 / 错误率 / GPU 利用率 / 队列积压 |
| Log | Loki / ELK | 结构化 JSON 日志 |
| Event | 业务事件流 | 任务状态变更、计费事件 |
| 告警 | Alertmanager | GPU 利用率、SLO 违反、Provider 错误率 |

每个 Job 自动注入 `trace_id` / `tenant_id` / `character_id`，便于横向排查。

---

## 12. 部署架构

### 12.1 K8s 拓扑

```
Namespace: avcore
├── api-gateway          (Deployment × 3, HPA)
├── character-svc        (Deployment × 2)
├── knowledge-svc        (Deployment × 2)
├── script-svc           (Deployment × 2)
├── pipeline-svc         (Deployment × 3, Leader Election)
├── render-svc           (Deployment × 2)
├── notify-svc           (Deployment × 2)
├── workers/
│   ├── tts-pool         (Deployment, GPU)
│   ├── img-pool         (Deployment, GPU)
│   ├── video-pool       (Deployment, GPU)
│   └── rag-pool         (Deployment, CPU)
├── postgres             (StatefulSet 或托管)
├── redis                (StatefulSet 或托管)
├── minio                (StatefulSet 或对接 OSS)
└── milvus               (Helm, 独立集群)
```

### 12.2 多环境

- `dev` / `staging` / `prod`，按 namespace 隔离
- 模型版本通过 ConfigMap / Admin API 灰度

### 12.3 灰度与回滚

- API 层：按租户灰度（Header 路由）
- 模型层：按租户配置 Provider 优先级
- 代码层：标准 K8s 滚动升级 / Argo Rollouts

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

---

## 14. 演进路线

### Phase 0 — 验证（4 周）
- 单体服务 + 单一视频 Provider
- 1 个角色 → 1 条视频的最小闭环

### Phase 1 — 多 Provider + 知识（8 周）
- Provider 抽象落地 ≥ 3 个
- 知识语料 + RAG 接入
- 任务系统、Webhook、可观测

### Phase 2 — 高可用 + 性能（8 周）
- 拆微服务、K8s 化
- 队列分层、GPU 调度
- 模型路由、降级、缓存

### Phase 3 — 平台化（持续）
- 模板市场、A/B 实验
- 多租户 SaaS、计费 / 配额
- 数字员工 / 直播等扩展场景

---

## 15. 风险与对策

| 风险 | 影响 | 对策 |
|------|------|------|
| 模型厂商限速 / 涨价 | 吞吐 / 成本 | 多 Provider + 路由 |
| 长链路失败 | 用户体验 | 节点级断点续跑 + 重试 |
| 数字人 / 声音合规 | 法务 | 强制授权 + 审核 + 水印 |
| GPU 成本失控 | 毛利 | 弹性 + 缓存 + 限速 |
| 模型效果不稳 | 口碑 | 评测体系 + 人工抽检 + 反馈闭环 |

---

## 16. 后续阅读

- 子模块详细设计：[docs/modules/](./modules/README.md)
- API 详细说明：[docs/api/](./api/README.md)
- 设计文档：[design.md](./design.md)
