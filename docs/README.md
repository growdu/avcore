# AVCore 文档中心

> 面向开发者的纯后端 AI 视频生成核心框架（AI Video Core）。

---

## 文档地图

```
docs/
├── README.md                  ← 你在这里
├── design.md                  设计文档：做什么、业务流、用户故事
├── architecture.md            架构文档：技术选型、服务拆分、部署
├── roadmap.md                 演进路线（可选）
├── modules/                   子模块详细设计
│   ├── character-modeling.md  人物形象建模
│   ├── character-cultivation.md  角色养成
│   ├── expert-cultivation.md  专家养成
│   ├── video-generation.md    视频生成
│   └── pipeline.md            工作流编排
└── api/                       API 文档
    └── README.md              端点一览、调用顺序、Webhook
```

---

## 快速导航

| 你想知道 | 看这里 |
|----------|--------|
| 框架能做什么 / 业务流程 | [design.md](./design.md) |
| 怎么搭起来 / 技术选型 | [architecture.md](./architecture.md) |
| 人物形象怎么建模 | [character-modeling.md](./modules/character-modeling.md) |
| 角色怎么养成 | [character-cultivation.md](./modules/character-cultivation.md) |
| 专家怎么养成 | [expert-cultivation.md](./modules/expert-cultivation.md) |
| 视频怎么生成 | [video-generation.md](./modules/video-generation.md) |
| 任务怎么编排 | [pipeline.md](./modules/pipeline.md) |
| 怎么调 API | [api/README.md](./api/README.md) |

---

## 框架一句话

> 开发者上传角色设定 + 声音样本 → 框架自动生成可复用的数字人形象与声音；
> 灌入领域知识后让角色成为"专家"；
> 给定主题即可一键产出"该专家讲解"的成片视频。

---

## 四大模块速览

| 模块 | 一句话 |
|------|--------|
| 人物形象建模 | 用"设定 + 声音样本"造一个数字人 |
| 角色养成 | 让数字人有性格、有口吻、有人设 |
| 专家养成 | 让数字人在某个领域是"行家" |
| 视频生成 | 把角色 + 脚本 + 音视频素材拼成成片 |
| 工作流编排 | 串起全流程，支持重试 / 断点 / 观测 |

---

## 阅读顺序建议

1. 先看 [design.md](./design.md) 了解全貌
2. 再看 [architecture.md](./architecture.md) 把握架构
3. 按需查阅各子模块文档
4. 集成前阅读 [api/README.md](./api/README.md)
