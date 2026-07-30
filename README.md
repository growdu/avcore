# AVCore（AI Video Core）

> **极简内核**：一个人物角色模型（PersonaModel），持续完善多版本，用其中一个版本出一段视频。
> 一个 SQLite 文件 `~/.local/share/avc/avc.db` 就是全部状态。

---

## 这是什么

AVCore 是个开源的 Rust 单二进制 CLI：

```
任意设定 → PersonaModel v1 ──▶ refine / finetune ──▶ v2 / v3 / ... ──▶ 用某个版本出视频
              │                    │                       │
              ▼                    ▼                       ▼
         SQLite 1 行          SQLite 1 行             SQLite 1 行 + artifacts
```

默认示例 persona：**Yu / 数据库内核专家**（`archetype=db_kernel_expert`）。

**强约束**：AVCore **只调用 token 鉴权的商业 / 开源模型 API，不加载本地模型**。

**预设规模**：≤ 50 persona，单机运行。50×5 版本 ≈ 6 GB——单一 SQLite 完全够。

---

## 安装 / 起步

```bash
cargo install avc                                                # 计划中
avc init                                                         # 建 ~/.local/share/avc
avc config set provider.llm.openai.api_key "sk-..."
avc persona onboard yu --from ./yu.toml                          # [集成] 创建 + 上传资产
avc render run --persona yu --version 1 --topic "InnoDB Buffer Pool"  # [集成] 出片

avc shell                                          # 交互模式：可输自然语言
avc ask --yes "把 Yu 的 traits 改成严谨务实"          # 非交互 NL：管道里用
```

> `avc` 三种入口：**精确 CLI**（`avc persona list`）、**交互式 Shell**（`avc shell`，可输入自然语言）、**非交互式 NL**（`avc ask "..."`）。底层 = 原子命令 + 集成命令。详见 [`docs/cli.md`](docs/cli.md) 与 [`docs/shell.md`](docs/shell.md)。

---

## 文档

1. [设计](docs/design.md) · 做什么、流程
2. [架构](docs/architecture.md) · 怎么实现、模块划分
3. [存储](docs/storage.md) · 单一 SQLite 的 schema
4. [CLI](docs/cli.md) · 命令与用法
5. [Shell](docs/shell.md) · 交互式 Shell 与自然语言
6. [子模块](docs/modules/README.md)：
   - [人物角色模型生成](docs/modules/persona-modeling.md) — `persona-svc`（+ 可选知识维度）
   - [人物角色模型迭代与微调](docs/modules/persona-iteration.md) — `iterate-svc` + `finetune-svc`
   - [视频生成](docs/modules/video-generation.md) — `render-svc`
   - [工作流编排](docs/modules/pipeline.md) — `pipeline-svc`
6. [Provider trait](docs/api/README.md) — Rust crate API + Provider 字段

---

## 许可

待定（倾向 Apache-2.0 / MIT）。
