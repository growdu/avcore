# Phase 0 Correctness Hardening Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** 修复已确认的配置读取和悬挂版本引用问题，让 Phase 0 的核心命令在进入真实 Provider 开发前具备可靠数据边界。

**Architecture:** 保持现有 CLI → svc → SQLite 分层，不新增依赖、不改 schema。配置读取在 `Config` 上提供点号路径查询；finetune/render 在写任务前验证 persona version 存在且状态可用。

**Tech Stack:** Rust 2021、rusqlite、serde/toml、现有 CLI 集成测试。

---

### Task 1: 修复 config set/get 往返

**Objective:** `config set provider.<dim>.<name>.<field> <value>` 后，`config get` 能按同一点号路径返回该值，而不是误报 unset。

**Files:**
- Modify: `src/config.rs`
- Modify: `src/cli/root.rs`
- Test: `tests/integration.rs`

**Step 1: Write failing test**

新增 `config_set_get_round_trip`：在临时 XDG 目录 init，set `provider.llm.openai.api_key=sk-test`，随后 get；断言 exit 0、stdout 含完整 key 和 `sk-test`，且不含 `(unset)`。

**Step 2: Run test to verify failure**

Run: `cargo test --locked --test integration config_set_get_round_trip -- --exact --nocapture`
Expected: FAIL — 当前 `config get` 输出 `(unset) provider.llm.openai.api_key`。

**Step 3: Write minimal implementation**

在 `Config` 增加只读 `get(&self, key: &str) -> AvcResult<Option<String>>`，只支持与现有 setter 相同的 `provider.<dim>.<name>.{api_key|model|endpoint}` 范围；未知维度/字段返回 `AvcError::Arg`，未设置值返回 `Ok(None)`。`cmd_config get` 调用它并输出 `key = value` 或 `(unset) key`。不要打印调试信息，不扩大到 shell/safety 配置。

**Step 4: Run test to verify pass**

Run: `cargo test --locked --test integration config_set_get_round_trip -- --exact --nocapture`
Expected: PASS。

**Step 5: Run full suite**

Run: `cargo test --locked --all-targets`
Expected: 8 tests pass。

---

### Task 2: 阻止 finetune 从不存在或不可用版本分叉

**Objective:** `finetune start` 仅允许从该 persona 已存在且版本状态可用（`pending` 或 `ready`）的版本分叉，避免创建幽灵 v100；其它 status（如 `building`）被拒绝。

**Files:**
- Modify: `src/svc/finetune.rs`
- Test: `tests/integration.rs`

**Step 1: Write failing tests**

新增：
- `finetune_rejects_missing_base_version`：persona 仅有 v1，start base 99；断言 exit 3，且 DB 中无 v100、无 finetune_jobs。
- `finetune_rejects_non_ready_base_version`：先 start base 1 创建 building v2，再以 base 2 start；断言 exit 4，且无 v3。

**Step 2: Run tests to verify failure**

Run each exact test. Expected: 当前命令成功并创建幽灵版本，因此 FAIL。

**Step 3: Write minimal implementation**

在同一数据库连接内、任何 INSERT 前查询 `(persona_model_id, base_version)` 的 status：
- 无行 → `AvcError::NotFound("persona '<name>' version <n>")`
- status 为 `pending` 或 `ready` → 保持现有 start 行为（Phase 0 尚未实现 `persona commit`，初始 v1 为 `pending`）
- 其它 status → `AvcError::Conflict(...)`

不要在本任务引入并发 job 唯一索引或迁移。

**Step 4: Run tests to verify pass**

Run the two exact tests. Expected: PASS。

**Step 5: Run full suite**

Run: `cargo test --locked --all-targets`
Expected: 10 tests pass。

---

### Task 3: 阻止 render 创建悬挂或不可用版本任务

**Objective:** `render run` 只为存在且版本状态可用的 persona version 创建 job；Phase 0 尚未实现 `persona commit`，初始 v1 为 `pending`，与 `ready` 一起被接受；其它 status（`building` 等）被拒绝。

**Files:**
- Modify: `src/svc/render.rs`
- Test: `tests/integration.rs`

**Step 1: Write failing tests**

新增：
- `render_rejects_missing_version`：persona 仅有 v1，render version 99；断言 exit 3，jobs 计数为 0。
- `render_rejects_non_ready_version`：用 finetune start 创建 building v2，render v2；断言 exit 4，jobs 计数为 0。

**Step 2: Run tests to verify failure**

Run each exact test. Expected: 当前均创建 queued job，因此 FAIL。

**Step 3: Write minimal implementation**

在 `create_job` 中、INSERT jobs 前查询指定版本 status：
- 无行 → `AvcError::NotFound("persona '<name>' version <n>")`
- status 为 `pending` 或 `ready` → 保持现有行为（Phase 0 尚未实现 `persona commit`，初始 v1 为 `pending`）
- 其它 status → `AvcError::Conflict(...)`（信息含 version/status）

topic/options 扩展不在本任务范围。

**Step 4: Run tests to verify pass**

Run exact tests. Expected: PASS。

**Step 5: Run full suite**

Run: `cargo test --locked --all-targets`
Expected: 12 tests pass。

---

### Task 4: 文档同步与最终验证

**Objective:** 文档准确反映 Phase 0 加固状态和当前真实边界。

**Files:**
- Modify: `docs/status.md`
- Modify: `README.md`（仅修复当前状态计数/明显语法边界，如需要）

**Step 1: Update status**

在 Phase 0 状态中说明 finetune/render 均会校验 ready version；测试矩阵更新实际数量和新增负路径类别。不要宣称真实 Provider、DAG 或 NL 已实现。

**Step 2: Verify source and docs**

Run:
- `cargo test --locked --all-targets`
- `cargo build --locked --all-targets`
- `cargo clippy --locked --all-targets -- -D warnings`（若 clippy 组件不可用，明确记录环境阻塞）
- `python -m mkdocs build --strict`（若依赖不可用，用 `python -m pip` 环境或明确记录）

Expected: 所有可运行检查通过，无静默跳过。

**Step 3: Review diff**

确认修改仅覆盖上述 correctness 边界和相应文档，未触碰未跟踪 `.git-backup-*`，未新增运行时依赖、未改 schema。
