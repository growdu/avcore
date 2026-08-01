# Phase 2 · render vendor / artifacts export / feedback

> 接 Phase 1。本波把三件"产品体验门槛"推到可演示：
> (1) render DAG 节点真调 vendor CLI 出 mp4
> (2) artifacts BLOB 落 DB → 落 FS
> (3) 用户反馈入口 `avc job feedback` 把 looks-unlike 推进样本池

**Goal:** 用户能真实跑一条 persona → render → export 链路得到 `final.mp4` 文件；
并且能标 "不像" 让 persona `feedback` 样本计数增 1，触发下游迭代。

**Architecture:**

- `ProviderCfg` 增 `binary: Option<String>` 字段 → CLI 工具路径（Phase 1 mock，Phase 2 真 vendor）
- `CliVideoProvider` 改 spawn 流程：
  - submit: `binary submit --prompt @script.txt --ref-image avatar.png --ref-audio voice.wav`
  - poll:   `binary status --task-id <id>`
  - fetch:  `binary fetch --task-id <id> --out mp4`
  - timeout/error → ProviderUpstream/ProviderTimeout
- 重写 `Pipeline::run` 的 i2v+compose 节点：节点 i2v / compose 调 `make_video().render(...)` 替换 mock 占位
- 真实 BLOB 仍 `artifacts` 表 + 现在加 `avc job export <job_id> --out <dir>` 落 FS
- `persona_samples.kind = 'feedback'` 落库 schema 已存在（migrations/0001_init.sql），加 CLI `avc job feedback <job_id> --looks-unlike`

**Tech Stack:** 既有依赖零新增；vendor CLI 仅用 `std::process::Command`。

**非范围：**

- 不写真/音/视频 vendor SFT 端点（Phase 1 stub 保留）
- 不接对象存储 (S3 等) — export 只落 FS
- 不实现自动从 feedback 反向迭代（人手动 finetune）
- 不实现 batch / pack

---

## 任务拆分

### T1 · ProviderCfg.binary + CliVideoProvider spawn 真跑

**Files:** `src/config.rs`, `src/provider/real.rs`, `tests/integration.rs`

**Step 1:** `ProviderCfg` 加 `binary: Option<String>`（保 round-trip 兼容）

**Step 2:** `CliVideoProvider`：
- new(name, cfg) 保留
- render() 时若 cfg.binary.is_some()，spawn 三段：
  - submit: stdout capture task_id (`data: {"task_id":"..."}`)
  - poll task_id: stdout capture status，每 200ms 一次, 5 分钟 timeout
  - fetch task_id out path: 读 .mp4 文件并 base64 编码后返
- 若 binary 为 None：保留现有占位 mock 行为（Phase 1 行为不变）

**Step 3:** 加 unit tests：
- `cli_video_calls_binary_succeeds` — 用 mock shell script 作为 `binary`
- `cli_video_binary_subprocess_failure_returns_provider_upstream`
- `cli_video_binary_timeout_returns_provider_timeout`

**Step 4:** 加集成测试 `render_run_uses_custom_video_binary`：
- init XDG + 写 toml `[provider.video.kling] binary="path-to-sh"`
- 写 mock binary shell script（写 /tmp/poll-N 次返 done + 写真文件）
- 跑 `avc render run`，断言 DB artifacts final_video.byte_size > 100

### T2 · `avc job export <job_id> --out <dir>`

**Files:** `src/svc/render.rs`, `src/cli/job.rs`, `tests/integration.rs`

**Step 1:** `svc::render::export(db, job_id, out_dir)` → 读 `artifacts WHERE job_id=?` 每行写 `out_dir/<kind>__<id>.bin`

**Step 2:** `cli/job.rs` 增 `export <job_id> --out <dir>` subcommand；同时增 `list <persona>`、`show <job_id>`、`wait <job_id>` 这几个已在 docs/cli.md 中标了但没实现的子命令；show/wait 简化即可（show=列 artifacts；wait=tokio sleep N 秒或直到 succeeded/failed）

**Step 3:** 加集成测试 `job_export_writes_artifacts_to_fs`

### T3 · `avc job feedback <job_id> --looks-unlike`

**Files:** `src/svc/render.rs`, `src/cli/job.rs`, `tests/integration.rs`

**Step 1:** `svc::render::feedback(db, job_id, looks_unlike)` → INSERT persona_samples(kind='feedback', text='reason', source='feedback_pool', persona_model_id = (job's persona), version_id_at_collection = (job's persona_version), ...)

**Step 2:** CLI `feedback <job_id> --looks-unlike [--reason ...] [--video-out <job_id>=...]` — Phase 1 仅 `--looks-unlike` 一档，reason 可选

**Step 3:** 集成测试 `job_feedback_writes_sample_with_kind_feedback`

### T4 · docs sync

**Files:** `docs/status.md`

**Step 1:** 把 Phase 2 的 ⬜ → ✅。

---

## 验收

* `cargo test --locked --all-targets -- --test-threads=1` 全绿（目标 60+ tests）
* 端到端手验：
  1. `init`
  2. 写 toml + mock 脚本
  3. `persona create` + `render run`
  4. `artifacts byte_size > 100` (真 mp4)
  5. `job export --out /tmp/...` 出文件
  6. `job feedback --looks-unlike` → persona_samples kind=feedback count 增 1
