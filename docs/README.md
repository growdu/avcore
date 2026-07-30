# AVCore 文档中心

> AVCore（AI Video Core）——开源、纯后端的 AI 数字人核心框架。CLI + REPL，本地优先，**Rust 单二进制**。

---

## 文档地图

```
docs/
├── README.md                        ← 你在这里
├── design.md                        设计文档：定位、领域模型、流程
├── architecture.md                  架构：技术选型、代码组织、打包
├── storage.md                       人物形象资产落盘格式 ⭐ 核心
├── cli.md                           CLI / REPL 命令与用法
├── modules/                         子模块详细设计
│   ├── README.md                    模块索引
│   ├── persona-modeling.md          [主] 人物角色模型生成（v1）
│   ├── persona-evolution.md         [主] 人物角色模型完善演进（持续训练 + 版本管理）⭐
│   ├── video-generation.md          [主] 视频生成（用模型出片）
│   ├── pipeline.md                  [主] 工作流编排（统一 DAG）
│   └── knowledge-aspect.md          [辅] 知识能力（可选维度）
└── api/
    └── README.md                    Provider trait + Rust crate API
```

---

## 快速导航

| 你想知道 | 看这里 |
|----------|--------|
| 框架能做什么 / 业务流程 | [design.md](./design.md) |
| 怎么搭起来 / 技术选型 | [architecture.md](./architecture.md) |
| 资产怎么存（人物形象） | [storage.md](./storage.md) ⭐ |
| 怎么用命令跑 persona | [cli.md](./cli.md) |
| 怎么创建一个角色模型 | [persona-modeling.md](./modules/persona-modeling.md) |
| 怎么持续训练 | [persona-evolution.md](./modules/persona-evolution.md) |
| 怎么出片 | [video-generation.md](./modules/video-generation.md) |
| 任务怎么编排 | [pipeline.md](./modules/pipeline.md) |
| 怎么接领域知识 | [knowledge-aspect.md](./modules/knowledge-aspect.md) |
| Provider 怎么扩 / Rust 怎么调 | [api/README.md](./api/README.md) |

---

## 框架一句话

> 一个 Rust 单二进制 CLI：创建 PersonaModel → 持续训练出新版本 → 用某个版本出片。
> 一切资产落本地；不内嵌 web / SaaS / 计费 / dashboard。

---

## 五大模块速览

| 模块 | 一句话 |
|------|--------|
| 人物角色模型生成 | 设定 + 样本 → PersonaModel v1 |
| 人物角色模型完善演进 | 持续追加样本 / 训练 / 出新版本 / 一致性兜底 |
| 视频生成 | 锁定 PersonaModel + version 出片 |
| 工作流编排 | 训练与渲染两条 DAG，统一节点 / 重试 / 观测 |
| 知识能力（可选） | 领域语料 + RAG；不接也能跑 |

---

## 阅读顺序建议

1. 先 [design.md](./design.md) 了解全貌
2. 再 [architecture.md](./architecture.md) 看技术选型与代码结构
3. 然后 [storage.md](./storage.md) 看完就知道"形象到底怎么存"
4. [cli.md](./cli.md) 上手命令
5. 按需看子模块
6. 集成方看 [api/README.md](./api/README.md)
