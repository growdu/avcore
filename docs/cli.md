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
| 例 | `persona create`, `persona attach-avatar`, `persona commit` | `persona onboard`, `persona evolve`, `render run` |
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
├── training    训练任务账本（只读 + cancel）
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
| `avc persona list [--status active|archived]` | 列表 |
| `avc persona versions <name>` | 历史版本 |
| `avc persona attach-avatar <name> --version <v> --ref <img>` | 写 `avatar_*` 列 |
| `avc persona attach-voice <name> --version <v> --ref <wav>` | 写 `voice_*` 列 |
| `avc persona attach-persona <name> --version <v>` | 写 `persona_descriptor_json` |
| `avc persona attach-knowledge <name> --version <v> --corpus <id>` | 写 `knowledge_binding_json` |
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
| `avc persona evolve <name> --scope <avatar|voice|persona> [--with-feedback] [--threshold <n>]` | 收集样本 + `training` + drift 评估 + 达标 → `commit` + `promote`，不达标 → `DELETE` 整行事务回退 |

> `--with-feedback` 自动把该 persona 最近标记 `looks_unlike` 的 feedback 样本纳入训练池。

### 3.3 sample

| 命令 | 类型 | 作用 |
|------|------|------|
| `avc sample add <persona> --kind image|audio|behavior_text|feedback --uri <p>\|--text <s> --consent <p>` | `[A]` | 入训练池 |
| `avc sample list <persona> [--kind]` | `[A]` | 列表 |
| `avc sample show <sid>` | `[A]` | 详情 |
| `avc sample remove <sid>` | `[A]` | 删除（仅删样本，不删 persona） |
| `avc sample consign <sid>` | `[A]` | 标金丝雀（必须不漂移） |

### 3.4 training（只读）

| 命令 | 类型 | 作用 |
|------|------|------|
| `avc training list [--persona <n>]` | `[A]` | 历史训练任务 |
| `avc training show <tjid>` | `[A]` | 详情 + 已完成节点 |
| `avc training report <tjid> [--json]` | `[A]` | 漂移评估报告 |
| `avc training cancel <tjid>` | `[A]` | 取消 |

> **训练任务**真正的"启动"在 persona.集成 `evolve` 里；这里只做查询。

### 3.5 job（渲染任务账本）

| 命令 | 类型 | 作用 |
|------|------|------|
| `avc job list [--persona <n>]` | `[A]` | 历史渲染任务 |
| `avc job show <jobid>` | `[A]` | 详情 + 进度 |
| `avc job wait <jobid> [--until <status>]` | `[A]` | 阻塞到结束（可设超时） |
| `avc job cancel <jobid>` | `[A]` | 取消运行中 |
| `avc job export <jobid> [--kind final_video|cover|subtitle|meta|all] --out <path>` | `[A]` | BLOB → FS |
| `avc job retry <jobid>` | `[A]` | 重跑失败节点 |
| `avc job rerender-scene <jobid> --idx <i>` | `[A]` | 重渲指定分镜 |
| `avc job feedback <jobid> --signal looks_unlike|thumbs_up\|wrong_voice\|... [--note]` | `[A]` | 写 `persona_samples(kind=feedback)` |

### 3.6 render

#### 原子

| 命令 | 作用 |
|------|------|
| `avc render script --persona <p> --version <v> --topic <t> --out <script.json>` | 只出分镜，不渲染 |
| `avc render script edit <file> --patch '<json-patch>'` | 编辑脚本（保留 diff） |
| `avc render video --from-script <script.json>\|--script-id <sid>` | 拿已有脚本渲染 |

#### 集成

| 命令 | 内部等价 |
|------|---------|
| `avc render run --persona <p> --version <v> --topic <t> [...]` | `script` + `video`（默认 80% 走这条） |
| `avc render pack --persona <p> --topics-file <path>` | 对每行 topic 跑一次 `render run` |

### 3.7 corpus（可选：知识语料）

| 命令 | 类型 | 作用 |
|------|------|------|
| `avc corpus create --name <n> --source-type upload\|url\|faq --uri <p>` | `[A]` | 新建语料 |
| `avc corpus chunks add <corpus> --from <jsonl>` | `[A]` | 追加 chunks（自动 embed） |
| `avc corpus chunks list <corpus>` | `[A]` | 列表 |
| `avc corpus search <corpus> --query <q>` | `[A]` | 远端 embed + cosine |
| `avc corpus reindex <corpus>` | `[A]` | 清空 `deprecated` 之外的全重 embed |

> `corpus bind` 等同 `persona attach-knowledge --corpus <id>`，所以放在 persona 那边。

### 3.8 provider

| 命令 | 类型 | 作用 |
|------|------|------|
| `avc provider list` | `[A]` | 所有 Provider |
| `avc provider show <name>` | `[A]` | 配置详情 + token 是否配 |
| `avc provider test <name>` | `[A]` | preflight：网络 + 鉴权 + 远端一次最小调用 |
| `avc provider config <name> --set KEY=VAL` | `[A]` | 改 provider.json 字段（不含 token） |

---

## 4. 组合示例（用原子搭流水线）

下面展示：一切"集成命令"都能用原子重新实现。这印证**原子化原则**——CLI 是壳，shell 是胶水。

### 4.1 受控 6 步创建 persona

```bash
avc persona create yu --archetype db_kernel_expert
# → persona_id=pm_xxx  version=1  status=pending

avc persona attach-avatar yu --version 1 --ref ./ref_*.png --style 写实
avc persona attach-voice yu --version 1 --ref ./sample.wav --lang zh
avc persona attach-persona yu --version 1 \
  --traits 严谨,务实 --tone 严谨 --catchphrase "我们直接看源码"
avc persona commit yu --version 1
# → status=ready
```

### 4.2 追加样本 + 训练

```bash
# 手动追样本
avc sample add yu --kind audio --uri ./new.wav --text "..." --consent ./auth.pdf

# 启动训练（集成）
avc persona evolve yu --scope voice --threshold 0.85
# 内部 = train + drift_eval + promote
```

或者纯原子版：

```bash
avc sample add yu --kind audio --uri ./new.wav --text "..." --consent ./auth.pdf
tj=$(avc training start yu --scope voice --base-version 1 | jq -r .tjid)  # 假设 add 命令式
avc training wait "$tj" --until succeeded
v=$(avc training show "$tj" --json | jq -r .result_version)
avc persona commit yu --version "$v"
avc persona promote yu --to "$v"
```

### 4.3 出片 + 反馈回灌

```bash
# 常规（集成）
avc render run --persona yu --version 2 --topic "InnoDB Buffer Pool" --duration 60
# 内部 = script + video

# 显式分步（原子）
avc render script --persona yu --version 2 --topic "..." --out script.json
avc render script edit script.json --patch '{"op":"replace","path":"/scenes/0/duration_ms","value":9000}'
jid=$(avc render video --from-script script.json --json | jq -r .jobid)

# 阻塞等结果
avc job wait "$jid" --until succeeded
avc job export "$jid" --kind final_video --out ./final.mp4

# 反馈
avc job feedback "$jid" --signal looks_unlike --note "侧脸不像本人"
# 内部触发 persona evolve --with-feedback 时被消费
```

### 4.4 批量出片

```bash
# 单条不行？批量跑
avc render pack --persona yu --topics-file ./daily_topics.txt --json | tee pack.log
```

### 4.5 完整运维脚本（示例）

```bash
#!/usr/bin/env bash
# evolve-and-render.sh —— "给 yu 加样本、训练、出片、再反馈" 的一条流水线

set -euo pipefail

NAME=yu
TOPIC="$1"

# 1. 收最近反馈到样本池
avc job feedback scan "$NAME" --since 24h  # 辅助原子（未来可选）

# 2. 训练（集成）
avc persona evolve "$NAME" --scope voice --with-feedback --threshold 0.85

# 3. 解析最新版本
v=$(avc persona show "$NAME" --json | jq -r .current_version)

# 4. 出片
jid=$(avc render run --persona "$NAME" --version "$v" --topic "$TOPIC" --json | jq -r .jobid)
avc job wait "$jid" --until succeeded

# 5. 导出
avc job export "$jid" --all --out "./out/$TOPIC/"
```

集成命令把"大多数情况"一行搞定；原子命令+shell 把"边角情况"完全覆盖。

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
2. 实际执行时，所有变更都体现为 SQLite 的 INSERT / UPDATE，长任务进度写到 `job_steps` 与 `training_jobs`

`avc persona evolve yu --scope voice --dry-run` 应当输出：

```
plan (no changes made):

  1. sample add yu --kind audio --uri ./feedback_*.wav   (atomic)
  2. sample add yu --kind audio --uri ./new_*.wav         (atomic, --with-feedback resolved)
  3. training start yu --scope voice --base-version 1    (atomic)
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
                    attach-avatar    evolve
                    attach-voice
                    attach-persona
                    commit
                    promote/demote
                    archive/delete
sample              add/list/remove
training            list/show/report/cancel     (start 由 evolve 触发)
job                 list/show/wait/cancel/export/feedback
render              script/video     run / pack
corpus              create/chunks/search/reindex
provider            list/show/test/config
```

只统计**真正保留**的（去掉"已删除 / 即将做"）。合计：

- 原子：**约 38** 条
- 集成：**约 8** 条
- 总计：**46** 条左右

---

## 8. 与 SQLite 操作的对应

每条原子命令原则上对应一行 SQL：

| 原子命令 | 主要 SQL |
|---------|---------|
| `persona create` | `INSERT persona_models` + `INSERT persona_versions(status=pending)` |
| `persona attach-avatar` | `UPDATE persona_versions SET avatar_primary=..., avatar_primary_sha256=... WHERE ...` |
| `persona commit` | `UPDATE persona_versions SET status='ready' WHERE ...` |
| `persona evolve` | `BEGIN; ... COMMIT;` 多步 |
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

# 重新跑训练从某版本
avc persona evolve yu --scope voice --base-version 1

# 强制回滚到 v1
avc persona promote yu --to 1
```
