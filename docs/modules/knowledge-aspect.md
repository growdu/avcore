# 模块设计：知识能力（Knowledge Aspect，可选）

> **领域专家**只是人物角色模型的一种可能。**只有当角色真的代表"懂某领域"时才接入**——比如讲师、客服、医生、律师、法律助手。普通主播、品牌虚拟代言人、虚拟员工不挂知识也能用。

> 本模块描述知识接入与检索增强生成（RAG）的细节，是 [persona-modeling.md](./persona-modeling.md) 与 [persona-evolution.md](./persona-evolution.md) 的可选配套能力。

---

## 1. 模块目标

**输入**：领域语料（文档 / 网页 / FAQ / 表格 / 代码段）
**输出**：`KnowledgeBinding`（语料索引 + 检索配置 + 风格偏好），挂载到 `PersonaModelVersion`

**边界**：
- 不做形象 / 声音 / 人设训练
- 不直接产出视频；它只让 persona 在被驱动时"显得懂这个领域"

---

## 2. 为什么是"可选能力"

强调这一点的几个理由：

1. **不是所有角色都需要领域**——主播、虚拟代言人、Vlogger、品牌 IP 形象，主要靠"形象 + 风格"，不靠"专业知识"
2. **同一模型可分期激活**——v1 是"无知识版"，v2 才接入某垂直语料
3. **知识是 persona 的能力维度，不是独立实体**——在旧版本里被做成"专家"独立实体，反而割裂了"角色"与"知识"的关系

因此在 AVCore 里，`KnowledgeCorpus` 与 `KnowledgeBinding` 是 `PersonaModelVersion` 的可选子结构，没有独立顶级实体。

---

## 3. 数据契约

```python
@dataclass
class KnowledgeCorpus:
    id: str
    name: str
    source_type: str                 # upload / url / api / faq
    language: str = "zh"
    chunk_count: int = 0
    index_version: int = 0
    created_at: datetime

@dataclass
class CorpusChunk:
    id: str
    corpus_id: str
    content: str
    embedding: list[float] | None
    meta: dict                       # 来源、位置、标签
    token_count: int

@dataclass
class KnowledgeBinding:
    corpus_ids: list[str]
    domain: str | None               # "高中物理" / "保险产品" / "Python 教程"
    grounding_mode: str = "loose"    # strict / loose / hybrid
    retrieval: RetrievalConfig
    style: ExpertStyle

@dataclass
class ExpertStyle:
    terminology: list[str]           # 偏好术语
    sentence_style: str              # 简洁 / 详尽 / 学术 / 通俗
    must_mention: list[str] = []
    must_avoid: list[str] = []
    compliance: list[str] = []

@dataclass
class RetrievalConfig:
    top_k: int = 6
    score_threshold: float = 0.7
    rerank: bool = True
    hybrid: bool = True              # 向量 + BM25 混合
```

---

## 4. 语料接入

### 4.1 来源类型
| 类型 | 说明 |
|------|------|
| `upload` | 用户上传 PDF / DOCX / TXT / MD |
| `url` | 抓取 URL（sitemap / 单页） |
| `api` | 业务系统通过 API 注入 |
| `faq` | 问答对（结构化） |

### 4.2 流水线
```
原始文档 ──▶ 解析（PDF/DOCX/HTML）──▶ 清洗（去模板/去导航）
          ──▶ 切分（按段落/语义/长度）──▶ 元数据补全
          ──▶ 向量化（embed）──▶ 写入向量库
```

### 4.3 切分策略
- 默认：按段落 + 滑动窗口（chunk_size=500, overlap=80）
- FAQ：直接作为独立 chunk
- 表格：转 Markdown 后切分
- 代码块：独立 chunk，保留上下文

### 4.4 增量更新
- 追加：`POST /v1/corpora/{id}/chunks`
- 重建：`POST /v1/corpora/{id}/reindex`
- 弃用：通过 chunk 级 `deprecated=true` 标，不删（用于回滚与审计）

---

## 5. 检索增强生成（RAG）

### 5.1 检索流程
```
Query ──▶ Embedding ──▶ 向量召回 TopK*4
                     ──▶ BM25 召回 TopK*2
                     ──▶ Reranker 重排
                     ──▶ 阈值过滤
                     ──▶ TopK chunks
```

### 5.2 Grounding 模式
- `strict`：回答必须基于语料，否则拒答
- `loose`：允许 LLM 自由发挥，语料仅作参考（最常用）
- `hybrid`：优先用语料，语料不足时回退

### 5.3 Prompt 拼接
```text
[系统] 你是{persona_descriptor}，也是{domain}领域专家，仅基于下列资料回答。
[资料]
1. {chunk1}
2. {chunk2}
...
[用户问题] {query}
[要求] 若资料不足，明确告知；引用时标 [1] [2]。
```

### 5.4 风格融合
`KnowledgeBinding.style` 与 `PersonaDescriptor` 同时进 prompt：

```python
def build_prompt(persona: PersonaDescriptor, knowledge: KnowledgeBinding | None):
    parts = [
        f"你是名叫 {persona.name} 的 {persona.archetype}",
        f"性格：{', '.join(persona.traits)}",
        f"语气：{persona.tone}",
    ]
    if knowledge:
        parts.append(f"同时你是 {knowledge.domain} 领域专家：")
        parts.append(f"术语偏好：{knowledge.style.terminology}")
        parts.append(f"句风：{knowledge.style.sentence_style}")
    return "\n".join(parts)
```

---

## 6. 与版本演进的关系

知识是 persona 的一个**可热替换维度**：

- 同一角色模型 v1 不带知识，v2 接入法律语料，v3 换成医学语料
- 知识维度"训练"实际是**重建索引**而非微调 LLM（参见 [persona-evolution.md §4.4](./persona-evolution.md#44)）
- 切换语料不影响视觉 / 声音 / 人设

---

## 7. 接口

```http
POST   /v1/corpora                       创建语料
GET    /v1/corpora/{id}                  查询
POST   /v1/corpora/{id}/chunks           追加 chunks
POST   /v1/corpora/{id}/reindex          重建索引
POST   /v1/corpora/{id}/search           检索（调试 / 联调用）
DELETE /v1/corpora/{id}                  删除

# 绑定 / 解绑到某 persona model 版本
POST   /v1/persona-models/{id}/knowledge
GET    /v1/persona-models/{id}/knowledge
DELETE /v1/persona-models/{id}/knowledge

POST   /v1/persona-models/{id}/knowledge/ask         试运行问答
```

---

## 8. 评测与质量

| 指标 | 说明 | 目标 |
|------|------|------|
| 召回率 | 测试集问题召回相关 chunk 的比例 | ≥ 0.85 |
| 引用准确率 | 输出事实是否能在引用 chunk 中找到 | ≥ 0.90 |
| 拒答率 | grounding=strict 时未在语料中正确拒答 | ≥ 0.95 |
| 端到端问答 P95 | 检索 + LLM | ≤ 3s |

评测集：人工标注 50~200 道典型问题，定期回归。

---

## 9. 关键指标

- 语料切分 + 向量化吞吐：≥ 100 chunks/s
- 单语料最大规模：1 亿 chunks（按需分片）
- 检索 P95：≤ 300ms（不含 LLM）
- 端到端问答 P95：≤ 3s

---

## 10. 上下游

- **上游**：客户业务系统 / 运营手动上传
- **下游**：
  - [persona-modeling.md](./persona-modeling.md)（创建时绑定）
  - [persona-evolution.md](./persona-evolution.md)（再训练时重建索引）
  - [video-generation.md](./video-generation.md)（脚本生成时召回）
