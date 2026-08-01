# Changelog

All notable changes to AVCore (avc) are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/) — Phase 1 = `0.1.0`.

---

## [Unreleased]

### Planned
- avatar / voice SFT 节点真接 vendor SFT 端点（kling-API / ElevenLabs-clone）—— 现为 stub
- 真 vendor CLI 替换 mock（kling-cli / doubao-cli 等）—— 接口已固定，替换 binary 即可
- `export` 接 S3 / 对象存储（当前仅落本地 FS）

---

## [0.2.0] - 2026-08-01 — Phase 2 渲染体验闭环 (vendor + export + feedback + pack)

### Added
- `CliVideoProvider` 真 spawn 三段式（`submit / status / fetch`）；stdout 兼容 KV / JSON / SSE 三种 flavor；超时 5 min；Phase 1 占位 mp4 行为零回归
- `Pipeline::run` `video` 节点真调 `CliVideoProvider::render`（XDG-aware 加载 config）
- `avc job export <job_id> --out <dir>` — `artifacts` BLOB 落 FS `<kind>__<name>__<id>.bin`
- `avc job show <job_id> --artifacts` JSON 列 5 条 artifact
- `avc job feedback <job_id> --looks-unlike [--reason <text>]` — 写 `persona_samples(kind='feedback', source='user_feedback')`，开启"持续微调"飞轮
- `avc render pack <persona> --topics-file <path> [--version <n>]` — batch 批跑 N topics → N jobs，失败不中断 + 返 `(job_ids, errors)` + 任一失败 exit 4 供 CI 探测
- `svc::render::pack` 服务函数（topics-file 解析：跳过空行 + `#` 注释）
- `CHANGELOG.md` 入库（Keep a Changelog 风格）
- 单测 5 个 + 集成 7 个（CliVideoProvider spawn × 3 / job export / job feedback × 2 / render pack × 3）

### Changed
- `ProviderCfg` 增 `binary: Option<String>` 字段（vendor CLI 路径，向后兼容 None）
- `docs/status.md` 三条 ⬜ → ✅（render vendor / artifacts export / feedback 路径 / render pack）

### Known Limitations
- vendor CLI 仍需用户自己写二进制（mock shell 脚本可用）；`kling-cli` / `doubao-cli` 等真实二进制为 Phase 2.5
- avatar / voice SFT 节点仍为 stub（vendor SFT 端点未接）
- `export` 仅落本地 FS，不接 S3 等对象存储

---

## [0.1.0] - 2026-07-31 — Phase 1 完成

### Added
- Provider 路由表：`make_llm / make_embed / make_avatar / make_voice / make_video` 工厂，按 `[provider.<dim>.<name>]` 选实例
- `OpenAiCompatLlmProvider` — 接 `/v1/chat/completions`（OpenAI / DashScope / 智谱 / Ollama）
- `OpenAiCompatEmbedProvider` — 接 `/v1/embeddings`
- `OpenAiCompatAvatarProvider` — 接 `/v1/images/generations`（dall-e-3 / wanx / CogView）
- `OpenAiCompatVoiceProvider` — 接 `/audio/speech`（tts-1）
- `CliVideoProvider` 接口固定 + 占位 mp4 BLOB（Phase 2 接入 vendor 留口子）
- `ProviderCfg` 增 `base_url` + `extra_headers` (BTreeMap) — 兼容非 OpenAI 服务（Anthropic 走 `/v1/messages` 需 headers）
- `avc provider test {llm,embed,avatar,voice,video}.<name>` 探针
- `svc::drift::cosine_similarity` + `eval_voice_from_db` — 真算 finetune drift
- `svc::corpus::{split_into_chunks, create_from_file, search_async}` — 切 chunk + embed + search 真全实现
- `svc::pipeline::run` — Kahn 拓扑排序 → 顺序执行节点 → `job_steps` 落 status/progress/outputs_json；`artifacts` BLOB 落库（base64 decode + sha256）
- `svc::persona` 全套 + `persona_sample` 落库
- `ask::dispatch_nl` — LLM 真发 chat completions → plan JSON → 白名单原子（list/show/versions + set-traits/set-catchphrase/set-render/commit/promote）
- `shell::dispatch` — NL → 启发式分类（KNOWN_NOUNS）→ 原子 vs ask::dispatch_nl
- Ask NL plan argv 隐藏 bug 修复（`s.cmd` 原 "persona list" 字符串未 split_whitespace 注入）
- `avc render run` 真跑 DAG 5 节点（script_gen / tts / img_gen / i2v / compose）出 5 个 artifacts
- 单测 28 个 + 集成 27 个

### Changed
- `docs/status.md` Phase 1 全部 7 项 ✅
- `docs/api/README.md` 同步真实现
- `docs/cli.md` 完整 CLI 表面
- `docs/shell.md` 标 NL 入口实现

### Security
- `avc.toml` 加固：保存后 0600 权限；`api_key` 与 SQLite 物理分离
- Provider 错误统一：`ProviderUpstream`/`ProviderTimeout`/`ProviderRateLimit` 区分

---

## [0.0.0] - 2026-07-30 — Phase 0 骨架

### Added
- `avc init` SQLite DB 初始化
- `persona` CRUD 骨架
- `provider` trait 定义 + `MockProvider` 占位
- `cli` 四入口（persona / sample / iterate / finetune / job / render / corpus / provider / config / doctor / shell）
- `docs/{design,architecture,storage,cli,shell,modules}` 全套
- `docs/api/README.md` MVP 接口表
- migrations/0001_init.sql — 7 张表 + 索引

### Known Limitations
- 所有 Provider 走 Mock
- DAG 引擎未跑通（仅 schema）
- NL 入口未接
