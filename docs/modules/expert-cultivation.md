# 模块设计：专家养成（Expert Cultivation）

> 让角色具备某个垂直领域的"专业能力"，通过 RAG 把领域知识灌入 LLM。

---

## 1. 模块目标

输入：领域语料（文档、网页、FAQ、术语表）
输出：`Expert` + `KnowledgeCorpus`，绑定到 `Character`

边界：**不做**视觉形象、**不做**声音克隆、**不直接生成视频**。

---

## 2. 数据契约

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
    embedding: list[float] | None    # 1536-d
    meta: dict                       # 来源、位置、标签
    token_count: int

@dataclass
class Expert:
    id: str
    character_id: str
    domain: str                      # "高中物理" / "保险产品" / "Python 教程"
    corpus_ids: list[str]            # 关联语料
    style: ExpertStyle
    retrieval: RetrievalConfig
    grounding_mode: str = "strict"   # strict / loose（是否强制基于语料）
    meta: dict

@dataclass
class ExpertStyle:
    terminology: list[str]           # 偏好术语
    sentence_style: str              # 简洁 / 详尽 / 学术 / 通俗
    must_mention: list[str] = []     # 必含关键词
    must_avoid: list[str] = []       # 禁用词
    compliance: list[str] = []       # 合规边界

@dataclass
class RetrievalConfig:
    top_k: int = 6
    score_threshold: float = 0.7
    rerank: bool = True
    hybrid: bool = True              # 向量 + BM25 混合
```

---

## 3. 语料接入

### 3.1 来源类型
| 类型 | 说明 |
|------|------|
| `upload` | 用户上传 PDF / DOCX / TXT / MD |
| `url` | 抓取 URL（sitemap / 单页） |
| `api` | 业务系统通过 API 注入 |
| `faq` | 问答对（结构化） |

### 3.2 流水线
```
原始文档 ──▶ 解析（PDF/DOCX/HTML）──▶ 清洗（去模板/去导航）
          ──▶ 切分（按段落/语义/长度）──▶ 元数据补全
          ──▶ 向量化（embed）──▶ 写入向量库
```

### 3.3 切分策略
- 默认：按段落 + 滑动窗口（chunk_size=500, overlap=80）
- FAQ：直接作为独立 chunk
- 表格：转 Markdown 后切分
- 代码块：独立 chunk，保留上下文

### 3.4 增量更新
- 支持追加：`POST /v1/corpora/{id}/chunks`
- 支持重建：`POST /v1/corpora/{id}/reindex`
- 版本化：`index_version` 字段

---

## 4. 检索增强生成（RAG）

### 4.1 检索流程
```
Query ──▶ Embedding ──▶ 向量召回 TopK*4
                     ──▶ BM25 召回 TopK*2
                     ──▶ Reranker 重排
                     ──▶ 阈值过滤
                     ──▶ TopK chunks
```

### 4.2 Grounding 模式
- `strict`：回答必须基于语料，否则拒答
- `loose`：允许 LLM 自由发挥，语料仅作参考
- `hybrid`：优先用语料，语料不足时回退

### 4.3 Prompt 拼接
```text
[系统] 你是某领域专家，请仅基于下列资料回答。
[资料]
1. {chunk1}
2. {chunk2}
...
[用户问题] {query}
[要求] 若资料不足，明确告知"未在资料中找到"；引用时标 [1] [2]。
```

---

## 5. 接口

```http
POST   /v1/corpora                       创建语料
GET    /v1/corpora/{id}                  查询
POST   /v1/corpora/{id}/chunks           追加 chunks
POST   /v1/corpora/{id}/reindex          重建索引
DELETE /v1/corpora/{id}                  删除

POST   /v1/corpora/{id}/search           检索（调试 / 联调用）

POST   /v1/experts                       创建专家
GET    /v1/experts/{id}                  查询
PUT    /v1/experts/{id}                  更新
POST   /v1/experts/{id}/ask              专家问答（试运行）
```

---

## 6. 与其他模块协作

```
Expert + Persona ──▶ Script 生成的 system prompt（含语料召回）
                 ──▶ 对话场景的领域问答
                 ──▶ 脚本中的"专业知识点"自动校对
```

### 6.1 在 Script 生成中的位置
```
[1] LLM 收到主题 → 
[2] 触发 Expert 检索，召回相关 chunks →
[3] 注入 prompt，让 LLM 基于 chunks 生成分镜 →
[4] 输出 Script 时附带引用 [chunk_id]
```

### 6.2 引用与可追溯
- 生成的脚本中每个事实点附 `[chunk_id]`
- 视频字幕中可选择性展示引用编号（学术 / 培训场景）

---

## 7. 评测与质量

| 指标 | 说明 | 目标 |
|------|------|------|
| 召回率 | 测试集问题召回相关 chunk 的比例 | ≥ 0.85 |
| 引用准确率 | 输出事实是否能在引用 chunk 中找到 | ≥ 0.90 |
| 拒答率 | grounding 模式下未在语料中时正确拒答 | ≥ 0.95 |
| 时延 | 单次问答 P95 | ≤ 3s |

评测集：人工标注 50~200 道典型问题，定期回归。

---

## 8. 关键指标

- 语料切分 + 向量化吞吐：≥ 100 chunks/s
- 单语料最大规模：1 亿 chunks（按需分片）
- 检索 P95：≤ 300ms（不含 LLM）
- 端到端问答 P95：≤ 3s
