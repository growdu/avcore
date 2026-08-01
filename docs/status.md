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
| finetune start + publish | ✅ | start 前校验 base_version：缺失 → NotFound；非 pending/ready（如 building）→ Conflict；pending/ready 接受；Immediate 事务避免 deferred 锁升级 BUSY；target 与 finetune job 在同一事务创建，target 已存在（含并发重复）→ Conflict；publish 依据 drift 走 UPDATE ready 或 DELETE 回退 |
| job create / list / show | ✅ | render run 前校验 version：缺失 → NotFound；非 pending/ready → Conflict；pending/ready 接受 → INSERT jobs 行；status 字段可手改 |
| Provider trait + Mock | ✅ | `src/provider/mod.rs`（5 个 trait）+ `mock.rs`（返回占位 BLOB） |
| Shell 模式（原子透传） | ✅ | rustyline；`help / exit / clear / history` 内建 |
| ask 模式（NL 入口骨架） | ✅ | 无 LLM 时明确报错 exit 6；占位实现 |
| config set/get 往返 | ✅ | `config set provider.<dim>.<name>.<field>` 后，`config get` 同路径返回 `key = value`；`get` 仍对未知维度/字段返回 `AvcError::Arg`；set/get 对空 name 段对称拒绝（exit 2） |
| Phase 0 核心正确性加固（config 往返 + 版本状态守卫） | ✅ | 15 个集成测试：见下方测试矩阵 |

---

## Phase 1 — Provider 矩阵 + 持续 finetune + 漂移兜底

| 模块 | 状态 | 说明 |
|------|------|------|
| 真实 Provider 实现（kling / openai / elevenlabs / doubao 等） | ✅/⬜ | Phase 1.1 拆分如下 ↓ |
| `openai_compat` LLM 真实现（任意 OpenAI 兼容 chat 端点） | ✅ | `src/provider/real.rs::OpenAiCompatLlmProvider`；接任意 OpenAI 兼容 `/chat/completions`；通过 `base_url` + `extra_headers` 兼容 OpenAI / Anthropic 兼容 proxy / DeepSeek / 智谱 / 豆包 / Ollama 等；401/403→TokenAuth、429→RateLimited、非 2xx→ProviderUpstream（exit 码 5/10/11/12 映射见 `docs/cli.md` §6.5）；第一道集成测试 `ask_with_real_llm_round_trip` 用最小 HTTP 端点验证 request → reply 路径 |
| `openai_compat` Embed 真实现（任意 OpenAI 兼容 `/embeddings` 端点） | ✅ | `src/provider/real.rs::OpenAiCompatEmbedProvider`；同一模板（base_url + extra_headers），覆盖 OpenAI text-embedding-3-* / 阿里 DashScope / 智谱 / Cohere embed-v3 / Ollama nomic-embed 等；`avc provider test embed.<name>` 探针 |
| avatar / voice / video 真 Provider | ⬜ | Phase 1.1 续；按 `src/provider/real.rs` 模式复制 trait 实现 |
| Provider 路由表（provider.json + 注册） | ✅ | 已落地：`~/.config/avc/avc.toml` 的 `[provider.<dim>.<name>]` 段 + `extra_headers`，工厂 `make_llm(&Config, name)` / `make_embed(&Config, name)` 解析 |
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
tests/integration.rs           19 tests
├── version_and_help
├── init_idempotent_guard
├── persona_lifecycle_json
├── refine_changes_persist
├── finetune_creates_v2_then_publish
├── finetune_publish_failed_drifts_rollback
├── finetune_rejects_missing_base_version       [新增] Task 2: 缺失 base → NotFound
├── finetune_rejects_non_ready_base_version     [新增] Task 2: building → Conflict
├── finetune_rejects_duplicate_target_version   [新增] Task 1: 重复 target → Conflict，target/job 不重复写入
├── finetune_concurrent_starts_are_conflicts    [新增] 3×8 并发：每轮 1 成功 + 7 exit 4，无 BUSY/exit 20
├── render_rejects_missing_version              [新增] Task 3: 缺失 version → NotFound
├── render_rejects_non_ready_version            [新增] Task 3: building → Conflict
├── config_set_get_round_trip                   [新增] Task 1: 点号路径 round-trip
├── config_rejects_empty_provider_name          [新增] Task 1+: set/get 对空 name 对称拒绝
├── ask_without_llm_errors
├── ask_with_real_llm_round_trip                [新增] Phase 1.1: ask 真发请求到 OpenAI 兼容 LLM（最小 HTTP 端点）
├── provider_test_unknown_llm_name_says_not_configured   [新增] Phase 1.1: provider test 未配置
├── provider_test_unsupported_dim               [新增] Phase 1.1: avatar/voice/video 暂未实现
└── provider_test_embed_unknown                 [新增] Phase 1.1: 不存在的 embed provider
```

CI：

- `.github/workflows/rust.yml`：stable toolchain + `cargo build --locked --all-targets` + `cargo test --locked --all-targets` + `cargo clippy`（warning 不阻断）
- `.github/workflows/docs.yml`：mkdocs build --strict → GitHub Pages
