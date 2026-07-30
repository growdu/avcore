# 模块设计：知识能力（Knowledge Aspect，可选）

> **领域专家只是人物角色模型的一种可能**。只有当角色真的"懂某领域"时才接入——讲师、客服、医生、律师、法律助手。普通主播、品牌虚拟代言人、虚拟员工**不挂知识也能跑**。
>
> 本文档描述知识接入与 RAG 细节，是 [persona-modeling.md](./persona-modeling.md) 与 [persona-evolution.md](./persona-evolution.md) 的可选配套能力。

---

## 1. 模块目标

**输入**：领域语料（文档 / 网页 / FAQ / 表格 / 代码段）
**输出**：`KnowledgeBinding` —— 挂载到 `PersonaModelVersion` 的可选结构

**边界**：
- 不做形象 / 声音 / 人设训练
- 不直接产出视频；只让 persona 在被驱动时"显得懂这个领域"
- 不强制组件；用户可纯跑 persona 不挂知识

---

## 2. 为什么是"可选能力"

1. **不是所有角色都需要领域**——主播、虚拟代言人、Vlogger、品牌 IP，主要靠形象 + 风格
2. **同一个模型可分期激活**——v1 无知识，v2 才接某垂直语料
3. **知识是 persona 的能力维度，不是独立实体**——在旧版本里被做成"专家"独立实体，反而割裂了关系

因此在 AVCore 里，`KnowledgeCorpus` 与 `KnowledgeBinding` 是 `PersonaModelVersion` 的可选子结构。

---

## 3. 落盘布局（与 storage.md §3 一致）

```
personas/pm_xxx/vN/knowledge/
├── corpora/
│   └── corpus_01H.../
│       ├── chunks.parquet       # 或 sqlite 表
│       ├── embed.bin
│       └── index.faiss          # 或 sqlite-vss
└── binding.json                 # KnowledgeBinding 元数据
```

chunks 落地两种风格（按语料规模）：
- 小语料（< 100 MB）：直接进 SQLite `corpus_chunks` 表
- 大语料：parquet + FAISS 文件 + `knowledge_dir` 路径索引

---

## 4. 数据契约

```rust
struct KnowledgeCorpus {
    id: String,
    name: String,
    source_type: SourceType,     // upload/url/api/faq
    language: String,
    chunk_count: u64,
    index_version: u64,
    storage: CorpusStorage,      // sqlite | parquet+faiss
    created_at: DateTime<Utc>,
}

struct CorpusChunk {
    id: String,
    corpus_id: String,
    ordinal: u32,
    content: String,
    token_count: u32,
    deprecated: bool,             // 不删，但置 deprecated=true 时检索权重=0
    meta: Value,
}

struct KnowledgeBinding {
    corpus_ids: Vec<String>,
    domain: Option<String>,
    grounding_mode: GroundingMode, // strict / loose / hybrid
    retrieval: RetrievalConfig,
    style: ExpertStyle,
}

struct RetrievalConfig {
    top_k: u32,                 // 默认 6
    score_threshold: f32,       // 默认 0.7
    rerank: bool,
    hybrid: bool,               // 向量 + BM25 混合
}

struct ExpertStyle {
    terminology: Vec<String>,
    sentence_style: String,     // concise/detailed/academic/accessible
    must_mention: Vec<String>,
    must_avoid: Vec<String>,
}
```

---

## 5. 语料接入流水线

```
原始文档 ──▶ 解析（PDF/DOCX/HTML/MD）──▶ 清洗（去模板/导航）
          ──▶ 切分（按段落/语义/长度）──▶ 元数据补全
          ──▶ 向量化（embed）──▶ 写入 SQLite 或 parquet+faiss
          ──▶ index_version++，binding.json 重新写入
```

切分：
- 默认段落 + 滑动（chunk_size=500, overlap=80）
- FAQ → 独立 chunk
- 表格 → 转 markdown 切分
- 代码块 → 独立 chunk，保留上下文

增量：
- `avc corpus chunks add corpus_xxx --from chunks.jsonl`
- `avc corpus reindex corpus_xxx`
- chunk 级 `deprecated=true` 不删，留审计

---

## 6. RAG 流程

```
query ──▶ embed ──▶ vector recall (top_k * 4)
                    + BM25 recall (top_k * 2)
                    ──▶ rerank
                    ──▶ threshold filter
                    ──▶ top_k chunks
```

Grounding 模式：
- `strict`：只能基于语料回答
- `loose`：可自由发挥，语料只是参考（默认）
- `hybrid`：优先语料，缺则回退

Prompt 拼装：

```text
[系统] 你是{persona_descriptor}，也是{domain}专家。仅基于下列资料回答。
[资料]
1. {chunk_1}
2. {chunk_2}
...
[用户问题] {query}
[要求] 资料不足时明确告知。
```

---

## 7. 与版本演进的关系

知识是 persona 的一个**可热替换维度**：

- 同一模型 v1 不带知识，v2 接法律语料，v3 换医学语料
- 知识"训练"实际是**重建索引**，不是微调 LLM（见 [`persona-evolution.md §4.4`](./persona-evolution.md)）
- 切换语料不影响视觉 / 声音 / 人设

绑定/解绑命令：

```bash
avc persona knowledge bind yu --corpus corpus_xxx --domain "数据库内核"
avc persona knowledge unbind yu
```

---

## 8. CLI 用法

```bash
# 创建语料
avc corpus new --name "数据库内核" --source-type upload --uri ./physics.md

# 追加 chunk
avc corpus chunks add corpus_xxx --from ./chunks.jsonl

# 检索（试运行）
avc corpus search corpus_xxx --query "InnoDB Buffer Pool 替换算法"

# 重建索引（在绑定/解绑失败时用）
avc corpus reindex corpus_xxx

# 试运行问答
avc persona knowledge ask yu --query "..."

# 绑 / 解
avc persona knowledge bind yu --corpus corpus_xxx --domain "数据库内核"
avc persona knowledge unbind yu
```

---

## 9. Provider（全部 token 鉴权 API）

| Provider | 说明 |
|----------|------|
| `openai_embed` | OpenAI text-embedding-3-large / small |
| `volcengine_embed` | 火山引擎 embedding |
| `alibaba_embed` | 阿里通义 embedding |
| `cohere_embed` | Cohere embed（多语支持好） |
| `cohere_rerank` | Cohere rerank |
| `voyage_rerank` | Voyage AI rerank |

切换 = 修改 `provider.json` 的 `embed` / `rerank` / `splitter` 字段。**所有 embedding / rerank 都来自远端 API**，本地不持有 encoder。

---

## 10. 评测与质量

```bash
avc corpus eval corpus_xxx --eval-set ./qa.jsonl
```

| 指标 | 目标 |
|------|------|
| 召回率 | ≥ 0.85 |
| 引用准确率 | ≥ 0.90 |
| 拒答率（strict 模式） | ≥ 0.95 |
| 检索 P95 | ≤ 300 ms（不含 LLM） |
| 端到端问答 P95 | ≤ 3 s |

评测集 = 人工标注 50~200 道典型问题，回归测试。

---

## 11. 关键指标

- 语料切分 + 向量化吞吐：≥ 100 chunks/s
- 单语料最大规模：1 亿 chunks（按需分片）
- 检索 P95：≤ 300 ms

---

## 12. 上下游

- **上游**：业务系统 / 运营手动上传
- **下游**：
  - [persona-modeling.md](./persona-modeling.md)（创建时绑定）
  - [persona-evolution.md](./persona-evolution.md)（再训练时重建索引）
  - [video-generation.md](./video-generation.md)（脚本生成时召回）
