# AVCore 文档中心

> 极简内核：一个 SQLite，一个 token，一段视频。

---

## 文档地图

```
docs/
├── README.md                  ← 你在这里
├── design.md                  设计
├── architecture.md            架构
├── storage.md                 单一 SQLite schema
├── cli.md                     CLI / REPL
├── modules/                   子模块
│   ├── persona-modeling.md
│   ├── persona-evolution.md
│   ├── video-generation.md
│   └── pipeline.md
└── api/README.md              Provider trait
```

---

## 阅读路径

| 你想知道 | 看这里 |
|----------|--------|
| 总览 | [design.md](./design.md) |
| 架构 + 技术选型 | [architecture.md](./architecture.md) |
| 数据怎么存 | [storage.md](./storage.md) |
| 命令行怎么用 | [cli.md](./cli.md) |
| 怎么创建一个角色 | [modules/persona-modeling.md](./modules/persona-modeling.md) |
| 怎么持续训练 | [modules/persona-evolution.md](./modules/persona-evolution.md) |
| 怎么出片 | [modules/video-generation.md](./modules/video-generation.md) |
| DAG 怎么跑 | [modules/pipeline.md](./modules/pipeline.md) |
| 怎么写 Provider | [api/README.md](./api/README.md) |
