# 子模块详细设计

> 以 **人物角色模型（PersonaModel）** 为顶层抽象：模型既可以是技术专家，也可以是形象鲜明的虚构人物 / 真实人物复刻 / 虚拟员工。模型会被**持续训练**、**多版本管理**，最后被消费用于出片。

## 模块索引

> 4 个主模块 + 1 个横切关注点。

| # | 模块 | 一句话 | 文档 |
|---|------|--------|------|
| 1 | 人物角色模型生成 (Persona Modeling) | 从设定 + 样本创建 PersonaModel v1 | [persona-modeling.md](./persona-modeling.md) |
| 2 | 人物角色模型完善演进 (Persona Evolution) | 持续训练 / 微调 / 版本管理 / 一致性保障 | [persona-evolution.md](./persona-evolution.md) |
| 3 | 视频生成 (Video Generation) | 锁定 PersonaModel + 版本 出成片 | [video-generation.md](./video-generation.md) |
| 4 | 工作流编排 (Pipeline) | 把训练与渲染统一拆成 DAG | [pipeline.md](./pipeline.md) |
| ★ | 知识能力 (Knowledge Aspect, 可选) | 领域专家需要的语料 / RAG，可按需绑定 | [knowledge-aspect.md](./knowledge-aspect.md) |

## 概念关系（高层）

```
                      ┌────────────────────────────────────┐
                      │           PersonaModel             │
                      │  ├── v1  (snapshot, immutable)     │
                      │  │     ├── avatar                  │
                      │  │     ├── voice                   │
                      │  │     ├── persona_descriptor      │
                      │  │     ├── knowledge (可选)         │
                      │  │     └── identity_anchor         │
                      │  ├── v2  (再训练后的快照)            │
                      │  ├── v3  ...                       │
                      │  └── training_jobs[]                │
                      └────────────────┬───────────────────┘
                                       │
                              锁定某个 version
                                       │
                                       ▼
                               ┌──────────────┐
                               │   Video Job  │
                               └──────────────┘
```

## 选读路径

| 你是 | 推荐先看 |
|------|----------|
| 集成方开发者 | video-generation.md → pipeline.md |
| 模型 / 算法工程师 | persona-modeling.md → persona-evolution.md → knowledge-aspect.md |
| 后端 / 平台工程师 | pipeline.md → persona-evolution.md |
| 产品 / 业务方 | persona-modeling.md → persona-evolution.md |
