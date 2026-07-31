# Finetune Start Atomicity Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** 让 `finetune start` 原子创建目标版本和任务账本，并对同一 base version 的重复启动返回明确 Conflict。

**Architecture:** 保持现有单 SQLite + `Mutex<Connection>` 架构，不新增 migration 或索引。`start` 在一个 rusqlite transaction 中完成 base 校验、target 冲突检查、目标版本 INSERT 和 finetune_jobs INSERT；任一步失败即回滚。

**Tech Stack:** Rust 2021、rusqlite、现有 CLI 集成测试。

---

### Task 1: 重复 start 返回 Conflict 且不增写账本

**Objective:** 同一 persona/base version 已经预占 v(N+1) 后，再次 `finetune start` 必须 exit 4，而不是让多个 running job 共用同一 target。

**Files:**
- Modify: `src/svc/finetune.rs`
- Test: `tests/integration.rs`

**Step 1: Write failing test**

新增 `finetune_rejects_duplicate_target_version`：init、create persona，第一次 start base 1 成功；第二次 start base 1 应 exit 4。断言 v2 恰好 1 行、finetune_jobs 恰好 1 行，并检查 stderr 含 target/version 冲突线索。

**Step 2: Run test to verify failure**

Run: `cargo test --locked --test integration finetune_rejects_duplicate_target_version -- --exact --nocapture`
Expected: FAIL — 当前第二次 start exit 0，`INSERT OR IGNORE` 吞掉 v2 冲突并新增第二条 job。

**Step 3: Write minimal implementation**

在 `start` 中：
- 锁定 mutable connection，开启 transaction。
- transaction 内保持已有 base status 查询和 NotFound/Conflict 语义。
- 计算 target 后查询该 `(persona_model_id, target)` 是否已存在；存在即 `AvcError::Conflict`，信息含 persona、target version 和现有 status。
- 将 `INSERT OR IGNORE` 改为普通 `INSERT`。
- 在同一 transaction 内写 finetune_jobs，然后 commit。

不新增 schema/migration，不添加并发唯一索引，不改 CLI/publish。

**Step 4: Run test to verify pass**

Run exact test; expected PASS。

**Step 5: Run full suite**

Run: `cargo test --locked --all-targets`
Expected: 15 integration tests pass（14 + 跨进程并发回归）。

---

### Task 3: 跨进程并发 start 的 SQLITE_BUSY 语义

**Objective:** 多独立 CLI 同时 `finetune start <name> --base-version <N>` 必须
保证 1 个成功 + 其余 Conflict，绝不能出现 exit 20 (Db / SQLITE_BUSY)。

**Files:**
- Modify: `tests/integration.rs`
- Modify: `src/svc/finetune.rs`

**根因（已系统复现）:**

`start` 内部使用 `conn.transaction()`（rusqlite 默认 `Deferred`）。当 8 个独立
进程同时 start 时，多个事务可先完成读取，再在 `INSERT` 时竞争读→写锁升级；
该升级在 WAL 并发快照下可能返回 `SQLITE_BUSY` / `SQLITE_BUSY_SNAPSHOT`，继而进入
`AvcError::Db` → exit 20。rusqlite 0.31 打开连接时默认设置 5 秒 busy timeout，
但它不能消除这种 Deferred 升级冲突。终态数据库仍只 v2=1 / jobs=1，错误语义却
不是业务 Conflict。

**Step 1: Write failing test `finetune_concurrent_starts_are_conflicts`**

临时 XDG init + create persona yu；3 轮 × 8 进程并发 `finetune start yu
--base-version 1`（用 `std::sync::Barrier` 同时发起）；每轮断言：

- 成功恰好 1、其余全 exit 4、busy=0、other=0；
- 终态 v2=1、finetune_jobs=1。

在原 deferred 实现下，探针 10/10 中 9 次命中 exit20；测试在 3 轮下 RED 稳定
（最小 3/3 run 中第 0 轮即触发）。

**Step 2: 最小修复 `src/svc/finetune.rs`**

仅把 `start` 里的 `conn.transaction()?` 改为
`conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?`。
Immediate 模式下 BEGIN 直接拿写锁：

- 多个进程并发时只有一个先拿到写锁开始事务，其余由 rusqlite 默认 5 秒
  busy timeout 等待这个极短事务；
- 胜者提交后，等待方依次进入事务，并由 `target-version 已存在` 检查返回
  Conflict（exit 4）；
- 若数据库被其它长事务占用超过 busy timeout，仍保留真实 `AvcError::Db`
  语义，不把任意物理 BUSY 伪装成业务冲突。

**Step 3: 验收**

- `finetune_concurrent_starts_are_conflicts` 连续运行 10 次，全部 GREEN。
- `cargo test --locked --all-targets`：15/15 集成测试通过。
- `cargo build --locked --all-targets`：通过。

**Step 4: 边界与未做的事（明确记录）**

- 未在 `src/error.rs` 做全局 SQLITE_BUSY → Conflict 映射。原因：跨任意 SQL
  调用的"真实数据库忙"是物理资源问题，不等价于业务冲突。Immediate 已经把
  业务层冲突暴露出来（target-version 已存在 → Conflict），物理忙的场景只会
  出现在极端并发且 5 秒 busy timeout 都不够的情况下，不应被静默吃掉。
- 未引入额外 busy_timeout。当前事务为单条 `INSERT` + 一行 `INSERT`，极短；
  Immediate 已经把升级窗口关闭，5s busy_timeout 在外围连接默认即足够。
- 未改 CLI/schema/deps/publish。
- README/status 计数稍后统一，本计划内不改。

---

### Task 2: 状态文档与最终验证

**Objective:** 准确记录 `finetune start` 的事务和重复 target 防护。

**Files:**
- Modify: `docs/status.md`
- Modify: `README.md`（仅测试计数与一行状态）

**Step 1: Update status**

将测试计数更新为 15；在 finetune 状态中说明 target version 重复（含并发）时 Conflict，目标版本和任务账本在同一事务创建。

**Step 2: Verify**

Run:
- `cargo test --locked --all-targets`
- `cargo build --locked --all-targets`
- `python3 -m mkdocs build --strict`
- `git diff --check`

若 fmt/clippy 组件仍不可用，明确记录，不声称其通过。
