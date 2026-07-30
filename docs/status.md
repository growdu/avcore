# 实现状态（Implementation Status）

> 跟踪 docs/ 与代码的真实落地。本页在 Phase 0 / Phase 1 之间漂移是常态。

---

## Phase 0 — 单 Provider × 1 角色 × 1 视频跑通（**当前**）

| 模块 | 状态 | 说明 |
|------|------|------|
| CLI 三入口路由 | ✅ | `avc <atom>` / `avc shell` / `avc ask` 派发在 `main.rs`，argv 处理兼容 basename |
| 单一 SQLite + migrations | ✅ | `src/db/` + `migrations/0001_init.sql`；启动幂等迁移 |
| persona CRUD | ✅ | `persona create / list / show / versions` 走原子 SQLite 操作 |
| iterate apply (refine) | ✅ | 纯 SQL UPDATE；不调 Provider；同版本号升级；写 iterate_jobs 账本 |
| finetune start + publish | ✅ | start 预占 v(N+1)；publish 依据 drift 走 UPDATE ready 或 DELETE 回退 |
| job create / list / show | ✅ | render run → INSERT jobs 行；status 字段可手改 |
| Provider trait + Mock | ✅ | `src/provider/mod.rs`（5 个 trait）+ `mock.rs`（返回占位 BLOB） |
| Shell 模式（原子透传） | ✅ | rustyline；`help / exit / clear / history` 内建 |
| ask 模式（NL 入口骨架） | ✅ | 无 LLM 时明确报错 exit 6；占位实现 |
| 集成测试 | ✅ | 7 个：version / init 幂等 / persona CRUD / refine 落库 / finetune 双路径 / ask 错误 |

---

## Phase 1 — Provider 矩阵 + 持续 finetune + 漂移兜底

| 模块 | 状态 | 说明 |
|------|------|------|
| 真实 Provider 实现（kling / openai / elevenlabs / doubao 等） | ⬜ | Phase 1.1 |
| Provider 路由表（provider.json + 注册） | ⬜ | 加载 `~/.config/avc/avc.toml` 的 `[provider.*.*]` 段 |
| avatar / voice SFT 节点真调 Provider | ⬜ | 接 avatar.create / voice.clone / voice.finetune |
| drift_eval 用 Provider 返回的 embedding 真算 | ⬜ | 当前用 Mock 写死 0.9 |
| DAG 引擎真调度 | ⬜ | 当前 pipeline-svc 仅 stub；Phase 1.2 |
| LLM chat 真解析 NL → plan | ⬜ | 当前 ask 模式无 LLM 时报错；Phase 1.3 |
| Shell 内 NL 解析 | ⬜ | Phase 1.3 |
| corpus 切 chunk + embed | ⬜ | Phase 1.4 |

---

## Phase 2 — 多 persona 矩阵 + 真实出片

| 模块 | 状态 |
|------|------|
| video.render DAG 节点（script_gen / tts / img_gen / i2v / compose） | ⬜ |
| artifacts BLOB 落库 + export | ⬜ |
| feedback 路径（`avc job feedback`） | ⬜ |
| render pack（topics-file → 批跑） | ⬜ |

---

## Phase 3+（按需扩展，不在主线）

- 多用户 / 跨机迁移（与本框架无关，独立项目）
- Web UI（不属于内核）
- Provider 健康检查 / 限速策略增强
- 升级阈值到 100+ persona 的 side-file 拆分

---

## 测试矩阵

```
tests/integration.rs            7 tests
├── version_and_help
├── init_idempotent_guard
├── persona_lifecycle_json
├── refine_changes_persist
├── finetune_creates_v2_then_publish
├── finetune_publish_failed_drifts_rollback
└── ask_without_llm_errors
```

CI：

- `.github/workflows/rust.yml`：stable toolchain + `cargo build --locked --all-targets` + `cargo test --locked --all-targets` + `cargo clippy`（warning 不阻断）
- `.github/workflows/docs.yml`：mkdocs build --strict → GitHub Pages
