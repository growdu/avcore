# CLI 设计

> AVCore 的 CLI 单一根命令 `avc`，子命令 = `<noun> <verb>`。所有命令归为两类：**原子** 与 **集成**。两类并存，职责清晰。

---

## 1. 设计原则

### 原则一：原子化（Atomic）

> **一条命令 = 一个资源 = 一个动作。**

- 单一职责；可被 shell 组合
- 幂等（能重跑）；可加 `--dry-run` 看清将做什么
- 输出**结构化**：`--json` 永远有效，机器可读
- 失败时不破坏未涉及的状态

### 原则二：集成化（Integrated / Workflow）

> **一条命令 = 一条典型工作流，封装 N 个原子。**

- 满足"用户最常用的 80% 路径"
- 内部依然走原子（不是黑盒魔法）：`--dry-run` 展开为原子列表
- 任何集成都能"降级"为手工跑的原子序列
- 集成命令的命名不与原子命令同名（避免歧义）

### 一句话区分

> 能用一行说清的，就是**集成**；需要几句说明的，就是**原子**。

| | 原子 | 集成 |
|---|------|------|
| 例 | `persona create`, `persona set-traits`, `finetune start` | `persona onboard`, `persona refine`, `persona finetune`, `render run` |
| 调用频率 | 单次步骤 | 80% 工作流 |
| 参数粒度 | 明确指出一个动作 | 接受"模板 / YAML / topic" |
| 失败行为 | 仅该步失败，不影响其他 | 默认自动回退；可用 `--no-rollback` 关闭 |
| 可见性 | `--dry-run` 打印下一步 | `--dry-run` 展开为原子步骤清单 |

---

## 2. 资源树

```
avc
├── root        系统级（init / doctor / config / backup / ...）
├── persona     角色管理
├── sample      训练样本
├── iterate     refine 任务账本（只读 + cancel）
├── finetune    finetune 任务账本（只读 + cancel）
├── job         渲染任务账本
├── render      出片工作流
├── corpus      知识语料（可选维度）
└── provider    Provider 注册与诊断
```

资源命名**与 SQLite 表一一对应**。一个原子命令 = 表的一次操作。

---

## 3. 完整命令表

> 命令格式：`avc <noun> <verb> [--flags]`。  
> 标记 **`[A]`** = 原子；标记 **`[I]`** = 集成。

### 3.1 root（系统级）

| 命令 | 类型 | 说明 |
|------|------|------|
| `avc init` | `[A]` | 初始化 `~/.local/share/avc/avc.db` + `~/.config/avc/avc.toml` |
| `avc doctor` | `[I]` | 集成：`verify` + token preflight + 网络探测 |
| `avc verify [--persona <id>]` | `[A]` | 重算 / 比对 SHA256 |
| `avc backup --out <path>` | `[A]` | WAL checkpoint + atomic copy |
| `avc restore --from <path>` | `[A]` | 替换 `avc.db` |
| `avc export --persona <id> --out <tar>` | `[A]` | 单 persona 导出 |
| `avc import <tar>` | `[A]` | 单 persona 导入 |
| `avc prune [--archive-older-than <days>]` | `[I]` | 集成：扫描 archived + 物理清理 |
| `avc config get <key>` / `set <key> <val>` | `[A]` | 读写 `avc.toml` |
| `avc version` | `[A]` | 打印版本 |

### 3.2 persona

#### 原子（最小操作集）

| 命令 | 作用 |
|------|------|
| `avc persona create --name <n> --archetype <a>` | 预占 `persona_models` 行 + 新版本号 status=`pending` |
| `avc persona show <name>` | 概要 |
| `avc persona list [--status active\|archived]` | 列表 |
| `avc persona versions <name>` | 历史版本 |
| `avc persona attach-avatar <name> --version <v> --ref <img>` | 写 `avatar_*` 列 |
| `avc persona attach-voice <name> --version <v> --ref <wav>` | 写 `voice_*` 列 |
| `avc persona attach-persona <name> --version <v>` | 写 `persona_descriptor_json` |
| `avc persona attach-knowledge <name> --version <v> --corpus <id>` | 写 `knowledge_binding_json` |
| `avc persona set-traits <name> --version <v> --traits <list>` | refine：改 `persona_descriptor_json.traits` |
| `avc persona set-catchphrase <name> --version <v> --add <s>` / `--remove <s>` | refine：改 catchphrases |
| `avc persona set-render <name> --version <v> --resolution 1080p ...` | refine：改 `manifest_json.render_options` |
| `avc persona commit <name> --version <v>` | pending → ready，触发 anchor 抽取并落行 |
| `avc persona promote <name> --to <v>` | 改 `current_version` |
| `avc persona demote <name> --version <v>` | 该版本 status=`deprecated` |
| `avc persona archive <name>` | 软删除 |
| `avc persona delete <name> --confirm` | 硬删除（要求 `--confirm`，默认拒绝） |
| `avc persona current <name> [--set <v>]` | 读 / 设当前版本 |
| `avc persona inspect <name> --version <v>` | 结构化展示整行 |
| `avc persona dump <name> --version <v> --out <dir>` | 一次性导出可读目录 |

#### 集成（典型工作流）

| 命令 | 内部等价于 |
|------|----------|
| `avc persona onboard --name <n> --from <yaml>` | `create` + `attach-avatar` + `attach-voice` + `attach-persona` (+ 可选 `attach-knowledge`) + `commit` |
| `avc persona refine --name <n> --from <yaml>` | `set-traits` + `set-catchphrase` + `set-render` + `corpus attach/detach`（按 yaml diff；通常不调 Provider） |
| `avc persona finetune --name <n> --scope <avatar\|voice\|persona> [--with-feedback] [--threshold <n>]` | 收集样本 + `finetune start` + drift 评估 + 达标 → `commit` + `promote`，不达标 → `DELETE` 整行事务回退 |

> `--with-feedback` 自动把该 persona 最近标记 `looks_unlike` 的 feedback 样本纳入训练池。

### 3.3 sample

| 命令 | 类型 | 作用 |
|------|------|------|
| `avc sample add <persona> --kind image\|audio\|behavior_text\|feedback --uri <p>\|--text <s> --consent <p>` | `[A]` | 入训练池 |
| `avc sample list <persona> [--kind ...]` | `[A]` | 列出 |
| `avc sample show <id>` | `[A]` | 详情 |
| `avc sample remove <id>` | `[A]` | 删除 |

### 3.4 iterate（refine 任务账本）

| 命令 | 类型 | 作用 |
|------|------|------|
| `avc iterate list --persona <n>` | `[A]` | 列出该 persona 的 iterate 任务 |
| `avc iterate show <id>` | `[A]` | 任务详情（含 `changes_json`） |
| `avc iterate cancel <id>` | `[A]` | 取消 queued |

### 3.5 finetune（SFT 任务账本）

| 命令 | 类型 | 作用 |
|------|------|------|
| `avc finetune start <persona> --scope ... --base-version <v> [--threshold <n>]` | `[A]` | 启动 finetune 任务（INSERT 新版本行 + 调 Provider SFT 端点） |
| `avc finetune list --persona <n>` | `[A]` | 列出该 persona 的 finetune 任务 |
| `avc finetune show <id>` | `[A]` | 任务详情 |
| `avc finetune report <id> --json` | `[A]` | drift_report_json 结构化输出 |
| `avc finetune cancel <id>` | `[A]` | 取消 queued / running |

### 3.6 job

| 命令 | 类型 | 作用 |
|------|------|------|
| `avc job list --persona <n>` | `[A]` | 列出该 persona 的渲染任务 |
| `avc job show <id> [--watch]` | `[A]` | 任务详情 |
| `avc job wait <id> --until <status>` | `[A]` | 阻塞到目标状态 |
| `avc job cancel <id>` | `[A]` | 取消 |
| `avc job export <id> --all\|--kind <k> --out <dir>` | `[A]` | 拷 BLOB 到 FS |
| `avc job feedback <id> --looks_unlike` | `[A]` | 把"不像"标记写入 `persona_samples(kind=feedback)`，供下次 finetune `--with-feedback` 用 |

### 3.7 render

**原子**：

```bash
avc render script  --persona yu --version 2 --topic "..." --out script.json
avc render script edit script.json --patch '{"op":"replace","path":"/scenes/0/duration_ms","value":9000}'
avc render video --from-script script.json --quiet    # 返 job_id
```

**集成**：

```bash
avc render run --persona yu --version 2 --topic "InnoDB Buffer Pool" --duration 60 --resolution 1080p
# 内部 = render script + render video

avc render pack --persona yu --topics-file ./daily_topics.txt
# 对每行 topic 跑一次 render run
```

### 3.8 corpus

| 命令 | 类型 | 作用 |
|------|------|------|
| `avc corpus create --name <n> --source <path>` | `[A]` | 切 chunk + 调 embed API |
| `avc corpus chunks <id>` | `[A]` | 列 chunk |
| `avc corpus search <id> --query <q> --topk 5` | `[A]` | embed cosine top-K |
| `avc corpus attach <persona> --version <v> --corpus <id>` | `[A]` | 写 `knowledge_binding_json`（refine 路径） |
| `avc corpus detach <persona> --version <v>` | `[A]` | 清空 `knowledge_binding_json` |
| `avc corpus reindex <id>` | `[A]` | 重跑 embed |
| `avc corpus delete <id> --confirm` | `[A]` | 硬删除 |

### 3.9 provider

| 命令 | 类型 | 作用 |
|------|------|------|
| `avc provider list` | `[A]` | 已注册 provider |
| `avc provider show <name>` | `[A]` | 元数据 + endpoint + auth scheme |
| `avc provider test <name>` | `[A]` | 一次轻量 ping（校验 token + 网络） |
| `avc provider config <name> --set-key` | `[A]` | 写 token 到 `avc.toml`（不入 DB） |

---

## 4. 典型 shell 组合

集成命令把"大多数情况"一行搞定；原子命令 + shell 把"边角情况"完全覆盖。

```bash
# A. 完整闭环：创建 → 迭代 → 出片
NAME=yu
avc persona onboard $NAME --from ./yu.toml

# 改 prompt / 知识（纯数据，常做）
avc persona refine $NAME --from ./yu.v2.toml

# 加样本 + 微调（花 token，慢）
avc persona finetune $NAME --scope voice --base-version 2 --with-feedback

# 4. 出片
v=$(avc persona show $NAME --json | jq -r .current_version)
jid=$(avc render run --persona "$NAME" --version "$v" --topic "$TOPIC" --json | jq -r .jobid)
avc job wait "$jid" --until succeeded

# 5. 导出
avc job export "$jid" --all --out "./out/$TOPIC/"
```

---

## 5. 输出约定

### 5.1 默认输出

人类可读，TTY 下加颜色。非 TTY 退化为纯文本。

### 5.2 `--json`

所有命令都接受 `--json`，输出**稳定** JSON：

```bash
avc persona show yu --json
{
  "id": "pm_01H...",
  "name": "yu",
  "current_version": 2,
  "versions": [1, 2],
  "status": "active"
}
```

`--json` + `jq` 是构建脚本的推荐路径。

### 5.3 `--quiet`

只输出最关键 ID / 退出码，便于脚本里赋值：

```bash
jid=$(avc render run --persona yu --version 2 --topic "..." --quiet)
```

### 5.4 进度流

长任务支持 `--watch`：默认轮询 / 输出进度，Ctrl+C 干净退出。

### 5.5 退出码

| code | 含义 |
|------|------|
| 0 | ok |
| 1 | 通用失败 |
| 2 | 参数错 |
| 3 | 资源不存在 |
| 4 | 状态冲突 |
| 5 | token 鉴权失败 |
| 6 | token 未配置 |
| 10 | Provider 限速 |
| 11 | Provider 上游错 |
| 12 | Provider 超时 |

### 5.6 错误格式

```
error[E0501]: provider_unauthenticated
  provider: provider.avatar.kling
  hint: avc config set provider.avatar.kling.api_key ...
  doc:   https://avc.dev/errors/E0501
```

---

## 6. 集成命令的"展开"规则

每个集成命令都必须做两件事：

1. 接受 `--dry-run`：打印**将执行**的原子列表，不真执行
2. 实际执行时，所有变更都体现为 SQLite 的 INSERT / UPDATE，长任务进度写到 `job_steps` 与 `iterate_jobs` / `finetune_jobs`

### 6.1 `avc persona refine yu --from ./yu.v2.toml --dry-run` 输出

```
plan (no changes made):

  1. set-traits     yu --version 1 --traits 严谨,务实            (atomic)
  2. set-catchphrase yu --version 1 --add "我们直接看源码"        (atomic)
  3. set-render     yu --version 1 --resolution 1080p           (atomic)
  4. corpus attach  yu --version 1 --corpus db-internals         (atomic)
  no Provider SFT calls.  no new version.  no drift eval.
```

### 6.2 `avc persona finetune yu --scope voice --dry-run` 输出

```
plan (no changes made):

  1. sample add yu --kind audio --uri ./feedback_*.wav   (atomic)
  2. sample add yu --kind audio --uri ./new_*.wav         (atomic, --with-feedback resolved)
  3. finetune start yu --scope voice --base-version 1    (atomic)
  4.   ↳ publish_or_rollback branch
  5. persona commit yu --version <v>   if drift ok        (atomic)
  6. persona promote yu --to <v>      if drift ok        (atomic)
```

集成命令 = **原子列表 + 默认值 + 顺序**。可观察、可回放。

---

## 7. 命令矩阵总览

```
                         atomic        integrated
                    ───────────────  ──────────────
root                init/verify/...   doctor/prune/config
persona             create           onboard
                    attach-*         refine          ← 80% 路径（纯数据）
                    set-traits/...   finetune        ← 少数路径（调 SFT）
                    commit
                    promote/demote
                    archive/delete
sample              add/list/remove
iterate             list/show/cancel             (start 由 refine 触发)
finetune            start/list/show/report/cancel
job                 list/show/wait/cancel/export/feedback
render              script/video     run / pack
corpus              create/chunks/search/attach/detach/reindex
provider            list/show/test/config
```

只统计**真正保留**的（去掉"已删除 / 即将做"）。合计：

- 原子：**约 50** 条
- 集成：**约 9** 条
- 总计：**59** 条左右

---

## 8. 与 SQLite 操作的对应

每条原子命令原则上对应一行 SQL：

| 原子命令 | 主要 SQL |
|---------|---------|
| `persona create` | `INSERT persona_models` + `INSERT persona_versions(status=pending)` |
| `persona attach-avatar` | `UPDATE persona_versions SET avatar_primary=..., avatar_primary_sha256=... WHERE ...` |
| `persona commit` | `UPDATE persona_versions SET status='ready' WHERE ...` |
| `persona set-traits` | `UPDATE persona_versions SET persona_descriptor_json=... WHERE pm_id=? AND version=N`（refine 路径） |
| `persona refine` | `BEGIN; UPDATE persona_versions ... COMMIT;` 多步（refine 路径） |
| `finetune start` | `BEGIN; INSERT finetune_jobs; INSERT persona_versions(version=N+1, status=building); ... COMMIT;` |
| `persona promote` | `UPDATE persona_models SET current_version=... WHERE id=...` |
| `persona archive` | `UPDATE persona_models SET status='archived'` |
| `sample add` | `INSERT persona_samples` |
| `job feedback` | `INSERT persona_samples(kind='feedback')` |
| `job export` | `SELECT content FROM artifacts WHERE ...` |

> 集成命令是 `BEGIN; ... COMMIT;` 内多步原子。失败 → `ROLLBACK`，状态不变。

---

## 9. 不在 CLI 的事

这些**不该**用 CLI 暴露（让 Rust crate 处理）：

- `BLOB` 直接读写（用 `inspect / dump` 而不是 cat）
- 内部 DAG 配置（pipeline 引擎读 YAML，不开放 edit）
- schema migration（自动跑，不由用户触发）
- Provider 内部路由表（API 不稳定，未来拆）

---

## 10. 速查

```bash
# 镜像某次发布的快照
avc export --persona yu --out yu.tar.zst
# 在另一台机器
avc import yu.tar.zst

# 删除一个归档超过 30 天的 persona
avc prune --archive-older-than 30d

# 一次性看 schema
sqlite3 ~/.local/share/avc/avc.db ".schema persona_versions"

# 改 prompt / 人设 / 知识（纯数据，常见）
avc persona refine yu --from ./yu.v2.toml

# 调 Provider SFT 重新训声音（少数路径）
avc persona finetune yu --scope voice --base-version 1

# 强制回滚到 v1
avc persona promote yu --to 1
```
