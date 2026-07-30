# AVCore（AI Video Core）

> **开源、纯后端的 AI 数字人核心框架**。聚焦三件事：**流程编排 + 人物形象生成 + 视频生成**。用 CLI 或 REPL 在本地驱动一切。

[![status](https://img.shields.io/badge/status-design-blue)](#)
[![docs](https://img.shields.io/badge/docs-EN-blue)](./docs/README.md)

---

## 这是什么

AVCore 是一个**开源核心框架**，让个人开发者也能在本地或服务器上"造一个数字人，让它持续变好，再用它产视频"。

它刻意保持简单：

- **CLI + REPL** —— 不是 SaaS，不带 Web 控制台
- **本地优先** —— 默认 SQLite + 本地文件系统，不强求对象存储 / K8s
- **可插拔 Provider** —— 形象、声音、LLM、视频、知识检索都通过统一的 Provider 接口暴露主流模型
- **框架即编排器** —— 你写 DAG 描述，框架负责执行、重试、断点续跑
- **一切为了"完善一个角色"** —— 形象资产的保存、版本管理、跨版本一致性是核心关注

---

## 核心特性

- **Rust 单二进制** —— 启动亚秒级、零外部依赖、可静态链接
- **本地优先持久化** —— `~/.local/share/avc/` 内 SQLite + 文件资产，无需数据库服务器
- **统一 Provider 抽象** —— 形象 / 声音 / LLM / 视频 / 知识（可选）全部可替换
- **角色资产不可变快照** —— 每代版本是固化目录，运维错误也不破坏存量视频
- **持续训练优先** —— 追加样本即出 v2，框架检测漂移并自动回退
- **DAG 编排** —— 训练和渲染共用一套节点模型

---

## 它不是什么

- ❌ 不是 SaaS / Web 控制台
- ❌ 不内置计费 / 配额 / 多租户
- ❌ 不内置可观测性 dashboard（你可以接 OpenTelemetry，但不是默认）
- ❌ 不内置审核策略（你挂自己的）
- ❌ 不会自动开新模型（创建 PersonaModel 必须由人触发）

> 这些都是**插件**或**外部系统**的事，不是核心框架的事。

---

## 它能做什么

```
任意设定（专家 / 虚拟主播 / 真实人物复刻 / 虚拟员工 ...）
       │
       ▼
┌──────────────────────┐    ┌─────────────────────────┐
│  1. 人物角色模型生成  │ →  │   PersonaModel v1       │
│  （一次性创建）       │    │  形象 + 声音 + 人设 + 可选知识 │
└──────────────────────┘    └────────────┬────────────┘
                                         │
           持续追加样本 / 用户反馈回灌         │
                                         ▼
┌──────────────────────┐    ┌─────────────────────────┐
│  2. 角色完善演进      │ →  │   PersonaModel v2 / v3  │
│  （持续训练 + 版本）  │    │  含 Identity Anchor 跨版本一致性 │
└──────────────────────┘    └────────────┬────────────┘
                                         │
                                         ▼
                              ┌──────────────────────┐
                              │  3. 视频生成          │ → final.mp4
                              │  （锁定 version 出片）│
                              └──────────────────────┘
```

---

## 安装与运行（规划）

```bash
# 安装
cargo install avc              # 即将开放

# 或从源码
git clone https://github.com/growdu/avcore
cd avcore && cargo build --release

# 开始
avc init                                  # 初始化 ~/.local/share/avc/
avc persona new "Lily" --from samples/    # 创建 persona v1
avc persona show lily                     # 查看
avc persona evolve lily --add voice.wav   # 追加样本，再训练
avc render video --persona lily --topic "..."
```

也可进入交互式：

```bash
avc repl
> persona new "Lily" --from ./samples
> persona evolve lily --add ./new-voice.wav
> render video --persona lily --topic "..."
> exit
```

---

## 文档

**👉 完整文档见 [`docs/`](./docs/README.md)**

推荐阅读：
1. [设计文档](./docs/design.md) — 做什么、领域模型、流程
2. [架构文档](./docs/architecture.md) — 怎么做、技术选型、代码组织
3. [CLI / REPL 用法](./docs/cli.md) — 命令与例子
4. [人物形象存储格式](./docs/storage.md) — 资产怎么落盘 ⭐ 重点
5. [子模块索引](./docs/modules/README.md)
   - [人物角色模型生成](./docs/modules/persona-modeling.md) — 创建 v1
   - [人物角色模型完善演进](./docs/modules/persona-evolution.md) — 持续训练 + 版本管理
   - [视频生成](./docs/modules/video-generation.md) — 出片
   - [工作流编排](./docs/modules/pipeline.md) — 统一 DAG
   - [知识能力（可选）](./docs/modules/knowledge-aspect.md)
6. [API/Provider 参考](./docs/api/README.md) — 各 Provider 的配置字段

---

## 项目状态

当前阶段：**设计 / 文档**。代码骨架尚未生成。

路线图（详见架构文档第 14 节）：
- **Phase 0** — 最小闭环：`avc persona new → avc render video` 跑通 1 个 persona → 1 条视频（4 周）
- **Phase 1** — Provider 矩阵 + 持续训练（8 周）
- **Phase 2** — 多 Provider 路由、对象存储可选、OpenTelemetry 可选
- **Phase 3** — 平台化扩展（控制台 / 多租户等，由外部项目承担）

---

## 许可

待定（倾向 Apache-2.0 / MIT，方便个人和商用接入）。
