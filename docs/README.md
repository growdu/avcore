# AVCore 文档中心

> 面向开发者的纯后端 AI 数字人视频生成核心框架（AI Video Core）。聚焦**人物角色模型的生成、持续训练与视频消费**。

---

## 文档地图

```
docs/
├── README.md                        ← 你在这里
├── design.md                        设计文档：定位、领域模型、流程
├── architecture.md                  架构文档：技术选型、服务拆分、部署
├── modules/                         子模块详细设计
│   ├── README.md                    模块索引
│   ├── persona-modeling.md          [主] 人物角色模型生成（创建 v1）
│   ├── persona-evolution.md         [主] 人物角色模型完善演进（持续训练 + 版本管理）
│   ├── video-generation.md          [主] 视频生成（用模型出片）
│   ├── pipeline.md                  [主] 工作流编排（统一 DAG）
│   └── knowledge-aspect.md          [辅] 知识能力（可选的领域专家维度）
└── api/
    └── README.md                    API 端点、调用顺序、Webhook
```

---

## 快速导航

| 你想知道 | 看这里 |
|----------|--------|
| 框架能做什么 / 业务流程 | [design.md](./design.md) |
| 怎么搭起来 / 技术选型 | [architecture.md](./architecture.md) |
| 怎么创建一个角色模型 | [persona-modeling.md](./modules/persona-modeling.md) |
| 怎么持续训练这个模型 | [persona-evolution.md](./modules/persona-evolution.md) |
| 怎么用模型出视频 | [video-generation.md](./modules/video-generation.md) |
| 任务怎么编排 | [pipeline.md](./modules/pipeline.md) |
| 怎么接入领域专家知识 | [knowledge-aspect.md](./modules/knowledge-aspect.md) |
| 怎么调 API | [api/README.md](./api/README.md) |

---

## 框架一句话

> 开发者创建一个**人物角色模型**（不限专家 / 普通人物 / 虚拟员工），对模型**持续训练和演进**（多版本、保持身份一致），再用模型的某个版本来**生成成片视频**。

---

## 五大模块速览

| 模块 | 一句话 |
|------|--------|
| 人物角色模型生成 | 设定 + 样本 → PersonaModel v1（形象 + 声音 + 人设 + 可选知识） |
| 人物角色模型完善演进 | 持续追加样本 / 训练 / 出新版本 / 一致性兜底 |
| 视频生成 | 锁定 PersonaModel + version 出片 |
| 工作流编排 | 训练与渲染两条 DAG，统一节点 / 重试 / 观测 |
| 知识能力（可选） | 领域语料 + RAG；不接也能跑 |

---

## 阅读顺序建议

1. 先看 [design.md](./design.md) 了解全貌
2. 再看 [architecture.md](./architecture.md) 把握架构
3. 按需查阅各子模块文档（建模 → 演进 → 出片 是核心三步）
4. 集成前阅读 [api/README.md](./api/README.md)
