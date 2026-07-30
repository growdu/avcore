# 人物形象资产存储格式（Persona Asset Storage）

> **核心关注**：AVCore 把"一个人物角色模型的某个版本"以**不可变目录**的形式落到本地文件系统中——这既是运行时的真相来源，也是版本可回滚的基础。本文件描述这个目录布局，以及每种资产的存储约定。

> 这份规范有约束力。所有 Provider 实现必须按这个布局写入，反序列化时也按这个读取。

---

## 1. 总原则

- **一个 PersonaModelVersion = 一个不可变目录**
- **本框架仅存元数据 + 产物引用 + 用户上传的原始素材**；**不下载、不缓存模型权重**
- 所有"模型"在 Provider 端（远端 API），本框架只持有 `model_id / voice_id / face_id / knowledge_corpus_id` 等引用
- 文件写入采用**原子替换**（先写临时文件，再 `rename`）
- 元数据同时存 JSON（人类可读 / 调试友好）和 SQLite（索引 / 查询友好）
- 大文件（图、音、上传样本）作为原始 blob 落盘；小配置走 JSON
- 任何 Provider 返回的"模型权重 / LoRA 文件 / checkpoint"只存**引用**（URL / model_id），**不下载到本机**

---

## 2. 默认根目录

| 平台 | 默认路径 |
|------|----------|
| Linux / macOS | `~/.local/share/avc/`（`XDG_DATA_HOME` 优先） |
| Windows | `%LOCALAPPDATA%\avc\` |

子目录：
```
~/.local/share/avc/
├── personas/                  # 人物角色模型资产（关键）
│   └── {persona_model_id}/
│       └── v{N}/
│           ├── manifest.json          # 版本元数据（强 schema）
│           ├── avatar/
│           ├── voice/
│           ├── persona.json
│           ├── knowledge/             # 可选
│           └── identity_anchor.json
├── media/                     # 视频/封面/字幕等输出
│   └── jobs/{job_id}/
├── cache/                     # 临时缓存（Provider 下载 / 切分中间产物）
├── logs/                      # 任务日志（按日滚动）
├── avc.db                     # SQLite 主库（元数据 + 索引）
└── avc.toml                   # 用户级配置（provider keys 等，权限 0600）
```

> 上层可通过 `AVC_HOME` 环境变量指向别处。Provider 子目录里的资源是**只读快照**——任何写入都意味着一个**新版本**。

---

## 3. 版本目录布局（以 `v3` 为例）

```
~/.local/share/avc/personas/pm_01H..._v3/
├── manifest.json
├── avatar/
│   ├── primary.png            # 主形象图（1024px 长边，PNG）
│   ├── views/                 # 多视角（4~8 张）
│   │   ├── view_front.png
│   │   ├── view_side_l.png
│   │   ├── view_side_r.png
│   │   └── ...
│   ├── ref/                   # 上传的参考图原图（仅用于审计与重训）
│   │   └── ref_001.png
│   ├── lora/                  # 可选；远端 avatar LoRA 引用
│   │   └── ref.json           # { model_id, provider, trained_at, base_model, ... }
│   ├── face.json              # 通用 face_id / instantid / ip-adapter 锚点
│   └── provider.json          # 哪个 provider 生成的 + provider 版本
├── voice/
│   ├── sample.wav             # 用户上传的高质量样本（≥ 30s 干净人声），原始材料
│   ├── transcript.json        # sample 对应文本（用于训练对齐）
│   ├── embed.bin              # Provider 返回的 speaker embedding（一致性度量用，非本地模型权重）
│   └── ref.json               # 远端 voice_id / provider / created_at
├── persona.json               # 人设（详见 §5）
├── knowledge/                 # 可选——只有该版本绑定了知识时存在
│   ├── corpora/
│   │   └── corpus_01/
│   │       ├── chunks.parquet
│   │       ├── embed.bin
│   │       └── index.faiss    # 或 sqlite-vss / pgvector 文件
│   └── binding.json           # KnowledgeBinding 元数据
└── identity_anchor.json       # 跨版本一致性锚点
```

---

## 4. manifest.json（核心元数据）

每个版本根目录都有一个 `manifest.json`。这是 persona version 的真实身份。

```json
{
  "schema_version": 1,
  "persona_model_id": "pm_01H...",
  "persona_version": 3,
  "parent_version": 2,
  "created_at": "2026-07-30T08:12:00Z",
  "training_job_id": "tj_01H...",
  "status": "ready",
  "assets": {
    "avatar": {
      "format": "png",
      "sha256": "...",
      "byte_size": 1842300
    },
    "voice": {
      "sample_format": "wav",
      "sha256": "...",
      "byte_size": 9201000
    },
    "persona": { "schema_version": 1 },
    "knowledge": null
  },
  "providers": {
    "avatar": { "name": "sdxl_ip_adapter", "version": "v2.3" },
    "voice":  { "name": "cosyvoice", "version": "v0.6" }
  },
  "metrics": {
    "identity_consistency_vs_parent": 0.92,
    "style_consistency_vs_parent": 0.88,
    "quality_score": 0.84
  },
  "tags": ["neutral", "teach"],
  "notes": ""
}
```

---

## 5. persona.json

```json
{
  "schema_version": 1,
  "name": "Lily",
  "archetype": "mentor",
  "description": "温和、严谨的物理讲师",
  "traits": ["耐心", "严谨", "幽默"],
  "tone": "温和",
  "catchphrases": ["来，我们一步步看"],
  "taboos": ["绝对化表述", "医学诊断"],
  "scenario_prompts": {
    "teach": "请用通俗语言讲解，避免未定义术语",
    "marketing": "请突出价值、节奏感，结尾给出明确 CTA"
  },
  "formality": 0.6,
  "temperature": 0.7,
  "response_length": "medium",
  "language": "zh"
}
```

> 这份文件同时被 LLM 调用读取（system prompt 组装）和训练读取（SFT 数据）。

---

## 6. identity_anchor.json

跨版本一致性是 persona 演进的命门，**锚点特征必须独立存**，方便后续比对：

```json
{
  "schema_version": 1,
  "computed_at": "2026-07-30T08:12:30Z",
  "model_versions": {
    "face_encoder": "arcface-r100",
    "voice_encoder": "wespeaker"
  },
  "embeddings": {
    "face":  { "dim": 512, "uri": "../avatar/face_emb.bin", "sha256": "..." },
    "voice": { "dim": 512, "uri": "../voice/embed.bin",    "sha256": "..." },
    "style": { "dim": 768, "uri": "style_emb.bin",         "sha256": "..." }
  },
  "anchor_samples": ["sample_canary_001.png", "sample_canary_002.wav"]
}
```

演进评估逻辑：

```
new_anchor = extract(new_version)
old_anchor = load(parent_version/identity_anchor.json)
cos = cosine(new_anchor.face, old_anchor.face)
if cos < threshold: drift_detected → rollback
```

---

## 7. 已知要点

### 7.1 不可变性
- 一旦 `vN` 完成，`./personas/pm_xxx_vN/` 内任何文件**禁止修改**
- 重训只产出 `v(N+1)`，原版完整保留
- 强制手段：CI / 校验脚本以 `sha256` 比对，发现变更就拒绝

### 7.2 删除策略
- **永不物理删除**历史版本
- 仅"停用"：把 `manifest.status` 改为 `deprecated`
- 整个 persona 归档：`avc persona archive lily`，整个目录树加 `.archive` 后缀，30 天后由 `avc prune` 物理清理

### 7.3 大对象与加密
- 形象参考图 / LoRA / 声音样本属于"敏感个人数据"
- 默认**本地明文**，权限 `0600 / 0700`
- 可选加密目录：把 `~/.local/share/avc/` 整体放到 `gocryptfs` / `eCryptfs` 卷
- 框架**不**自实现加密（避免发明轮子）

### 7.4 资产规模估算
| 资产 | 单 persona 单版本 |
|------|-----------------|
| 主形象 PNG（1024px，由 Provider 生成 / 本地缓存） | ~2 MB |
| 多视角 ×6 | ~12 MB |
| LoRA 引用（**仅 JSON**，不下载权重） | < 1 KB |
| 声音样本（30~60s wav 48k，用户上传的原始材料） | ~10 MB |
| embed.bin × 数个（一致性度量用特征向量） | < 1 MB |
| JSON 配置 | KB 级 |
| **典型单版本总量** | **30–60 MB** |

1000 个 persona × 5 版本典型空间占用 ~ 300 GB；用户上传素材可由对象存储 plugin 接管。
> 由于 LoRA 权重**不下载到本机**，单版本体积从 80–250 MB 降至 30–60 MB。这是与"仅 API"对齐的直接好处。

---

## 8. SQLite 主库：`avc.db`

> 元数据 + 索引存 SQLite；大文件走文件系统。两边通过 hash 关联。

### 8.1 关键表（精简）

```sql
CREATE TABLE persona_models (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  archetype     TEXT,
  description   TEXT,
  current_version INTEGER NOT NULL,
  status        TEXT NOT NULL,        -- active / archived
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);

CREATE TABLE persona_versions (
  persona_model_id TEXT NOT NULL,
  version          INTEGER NOT NULL,
  parent_version   INTEGER,
  dir_path         TEXT NOT NULL,    -- 相对 ~/.local/share/avc
  status           TEXT NOT NULL,
  created_at       TEXT NOT NULL,
  training_job_id  TEXT,
  metrics_json     TEXT,
  PRIMARY KEY (persona_model_id, version),
  FOREIGN KEY (persona_model_id) REFERENCES persona_models(id)
);

CREATE TABLE training_jobs (
  id                   TEXT PRIMARY KEY,
  persona_model_id     TEXT NOT NULL,
  base_version         INTEGER NOT NULL,
  scope_json           TEXT NOT NULL,
  config_json          TEXT,
  status               TEXT NOT NULL,
  result_version       INTEGER,
  drift_report_json    TEXT,
  started_at           TEXT,
  finished_at          TEXT
);

CREATE TABLE persona_samples (
  id                       TEXT PRIMARY KEY,
  persona_model_id         TEXT NOT NULL,
  kind                     TEXT NOT NULL,  -- image / audio / behavior_text / feedback
  uri_or_text              TEXT,
  version_id_at_collection INTEGER,
  consent_proof            TEXT,
  tags_json                TEXT,
  quality_score            REAL,
  created_at               TEXT NOT NULL
);

CREATE TABLE jobs (                -- 渲染任务
  id                  TEXT PRIMARY KEY,
  script_id           TEXT,
  persona_model_id    TEXT NOT NULL,
  persona_version     INTEGER NOT NULL,    -- 锁定版本，永不漂移
  status              TEXT NOT NULL,
  options_json        TEXT,
  artifacts_json      TEXT,
  created_at          TEXT,
  finished_at         TEXT
);

CREATE TABLE knowledge_corpora (
  id                  TEXT PRIMARY KEY,
  name                TEXT NOT NULL,
  source_type         TEXT NOT NULL,
  language            TEXT,
  chunk_count         INTEGER,
  index_version       INTEGER DEFAULT 0,
  created_at          TEXT
);

CREATE TABLE corpus_chunks (
  id           TEXT PRIMARY KEY,
  corpus_id    TEXT NOT NULL,
  ordinal      INTEGER NOT NULL,
  content      TEXT NOT NULL,
  token_count  INTEGER NOT NULL,
  deprecated   INTEGER DEFAULT 0,  -- 0/1
  meta_json    TEXT,
  FOREIGN KEY (corpus_id) REFERENCES knowledge_corpora(id)
);

CREATE TABLE audit_log (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  ts            TEXT NOT NULL,
  actor         TEXT,           -- user / system
  action        TEXT NOT NULL,
  target_kind   TEXT,
  target_id     TEXT,
  detail_json   TEXT
);
```

> 知识语料内容（`content` / `embeddings`）也允许放文件系统，但默认**行存储** SQLite——小语料（< 100MB chunk）足够高效；超大再迁出去。

### 8.2 迁移
- 单文件 `avc.db` + 版本号 `schema_version`（顶层 PRAGMA user_version）
- 升级时跑 `avc migrate`，就地迁移 SQLite schema 与目录布局

### 8.3 备份
- 整个 `~/.local/share/avc/` 就是一个完整的有状态 snapshot
- 备份 = 拷目录 + 拷 SQLite
- 可选 `avc export` 把整个 persona 打包成 `tar.zst`，便于跨机

---

## 9. 不在框架内的存储

- **远程对象存储**（S3 / OSS）：通过 trait 抽象成 Provider 即可
- **Postgres / Milvus**：当前**不需要**；未来导入时只换 `avc.db` + Storage Provider

---

## 10. 文件被破坏时怎么办

- 每次访问 `personas/pm_xxx_vN/avatar/primary.png` 时校验 `sha256`，比对 manifest
- 不匹配 → 抛 `asset_corrupted`，禁止渲染（CI 必要时重新生成）
- 提供 `avc verify` 命令以**只读**遍历所有版本与产物，校验 sha256

---

## 11. 总结：为什么"目录即版本"

| 做法 | 好处 |
|------|------|
| 目录即版本 | 拷目录 = 拷一个完整 persona；rsync / restic / object storage 都能直接用 |
| 不放数据库的 blob | SQLite 只存元数据；崩溃后用文件系统可手动恢复 |
| 全 JSON 配置 | 调试时 `cat persona.json` 即看懂；git friendly（除了二进制外） |
| 不可变 | 误操作只可能出新版本，原版安全；运维风险最低 |
