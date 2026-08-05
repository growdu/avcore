# Changelog

All notable changes to AVCore (avc) are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/) — 当前 alpha 阶段（0.x → 1.0）。

---

## [Unreleased] — MiniMax 多模态 Provider 适配

### Added
- **MiniMax 多模态 Provider 适配**（avatar / voice / video 三维）：
  - 端点：`api.minimaxi.com` 专有 API（非 OpenAI 兼容）
  - 配置：`[provider.<dim>.<n>_minimax]` 段——名字后缀固定 `_minimax` 触发工厂路由
  - 实测可用模型：
    - `image-01`（图像，`/v1/image_generation`）
    - `speech-01-turbo`（TTS，`/v1/t2a_v2`）
    - `video-01`（视频，`/v1/video_generation` 3 段式）
  - 视频异步 3 段式：submit → poll `/v1/query/video_generation?task_id=...` → retrieve `/v1/files/retrieve?file_id=...` → 下载 mp4
  - **3 个 `#[ignore]` 真实 API 集成测试**（需 `MINIMAX_API_KEY` env）：avatar / voice / video 端到端
  - **9 个 mock 单元测试 + 工厂路由测试**（avatar / voice / video 各 3 个）
- **错误翻译**：401 → `TokenAuth`、429 → `RateLimited`、5xx → `ProviderUpstream`、
  `base_resp.status_code=2013` → `Arg`、其它非 0 → `ProviderUpstream`
- **工厂路由**：`make_avatar` / `make_voice` / `make_video` 按名字 `_minimax` 后缀路由到
  MiniMax provider（与 OpenAI 兼容路径共存）
- **公共 helper**：`auth_header` / `decode_hex_audio` / `handle_response`（MiniMax audio 字段
  是 HEX 编码非 base64，helper 自动解码）
- **`docs/cli.md` §12 新章节**：`MiniMax Provider (minimaxi.com)` 含配置段 / 用法 / 协议差异表
  / 重要陷阱 / 端到端示例

### Known Limitations
- **`provider.embed`**：MiniMax 没暴露 OpenAI 兼容 embedding endpoint（实测 `/v1/embeddings` 404）——
  沿用 OpenAI `text-embedding-3-small`
- **voice clone**：需 `file_id` / `audio_url` 复杂 schema，v1 留 placeholder（`Err` 提示用 vendor CLI）
- **avatar / voice finetune**：v1 留 placeholder（`Err` 提示用 vendor CLI；走 `avc finetune start --scope avatar|voice` 不接 MiniMax）
- **video I2V-01 / live**：需 `first_frame_image` 字段，v1 暂不支持（仅 T2V-01 / video-01）
- **video 每日 3 条配额**（用户层 plan 决定；撞到 429 报 `AvcError::RateLimited` exit 10）
- **`voice_id` 写死 `male-qn-qingse`**：MiniMax API 没暴露 list voices endpoint，v1 不可选声线

### Planned
- 升级阈值到 100+ persona 的 side-file 拆分（与本框架无关，独立项目）

---

## [0.3.4] - 2026-08-03 — Phase 3 daemon 模式 + Provider 健康/限速

### Added
- **后台 daemon**：`avc daemon start|stop|status|logs`（fork 子进程执行 `avc _run`、pid 写到 `~/.local/share/avc/avc.pid`、HTTP loopback 端口 7891、tracing 写 `~/.local/share/avc/avc.log`）。
- **Provider 健康探活**：5 维（`llm / embed / avatar / voice / video`）主动 ping loop + 被动 hook 旁路记录（不改变主流程）。PingLoop 间隔 60s（可配）。
- **`provider_health` / `provider_rate_limit` / `daemon_meta`** 三张表（migration 0003，记录最近一次健康 / 限速 / daemon 状态）。
- **`avc provider status [--dim <llm|embed|avatar|voice|video>] [--json]`** — 健康查询（默认 text 输出最近 1 条/provider；--dim 单维过滤；--json 稳定结构）。
- **`avc provider rate-limit [--json]`** — 限速查询（in_cooldown / hit_count_24h）。
- **`avc.toml` 新增 `[daemon]` 段**：`enabled`（默认 true）/ `port`（7891）/ `bind`（仅 `127.0.0.1`，不允许 `0.0.0.0`）/ `ping_interval_s`（60）/ `log_level`（tracing filter）/ `auto_record_hook`（true）。
- **axum 0.7 HTTP endpoints**（loopback only）：`GET /health/all` — 健康全表；`GET /limits/all` — 限速全表；`GET /version` — daemon 元数据。
- **27 个新测试**（10 单元：daemon 守护进程 / config 校验 / record_hook 旁路；17 集成：start/stop/status/logs 4 个 + provider status/rate-limit 5 个 + HTTP 端点 4 个 + record_hook 4 个）。**累计 89 单测 + 93 集成 = 182 个测试全过**。

### Changed
- **`OpenAiCompat*` Provider 错误分支旁路记录** `provider_health` / `provider_rate_limit`
  （`rate_limited` / `unauthenticated` / `timeout` / `upstream_error`），不改变主流程返回值。
- `Cargo.toml` 增 `axum = "0.7"` + `tower` 依赖（HTTP loopback 用）。
- `Config` 加 `daemon: Option<DaemonCfg>` 段。

### Known Limitations
- 沿用 v0.3.2 的三条限制。

---

## [0.3.3] - 2026-08-03 — Phase 2.5.3 测试覆盖补齐 + iterate/finetune list 守卫

### Added
- **集成 10 个新增**（iterate / persona CLI 覆盖补齐）：
  - `iterate_list_returns_apply_records` — 空 list 返 `[]`、2 次 apply 后返 2 行、`--json` 形态稳定
  - `iterate_list_unknown_persona_errors` — 未知 persona 走 NotFound（exit 3）
  - `iterate_apply_set_knowledge_merges_binding` — 验证 `knowledge_binding_json` 二次 set 是 merge 不是覆盖
  - `iterate_apply_set_manifest_merges_render_options` — 验证 `manifest_json.render_options.fps` 二次 set 覆盖、resolution 保留、style 新增
  - `iterate_apply_three_sections_together` — `--set-persona/--set-knowledge/--set-manifest` 一次给齐三列都落库
  - `iterate_apply_missing_version_errors` — 缺 `--version` → Arg（exit 2）
  - `iterate_apply_unknown_persona_errors` — 未知 persona → NotFound（exit 3）
  - `persona_show_returns_full_row_json` — `--json` 形态（name / archetype / current_version / status）
  - `persona_show_unknown_errors` — NotFound 守卫
  - `persona_versions_lists_after_finetune` — v1 → finetune start → v2 自动建好，`persona versions` 看到 `[1, 2]`
  - `iterate_show_parses_changes_json_as_object` — show 返 JSON object 而非 raw string，changes 字段可直接索引
  - `persona_list_filters_by_status` — 默认 / `--status pending` / `archived` / 未知状态四场景都返稳定结构
- **累计 79 单测 + 76 集成 = 155 个测试全过**。

### Changed
- `iterate list <persona>` 现在先调 `persona::get_persona` 验存在——未知 → NotFound（exit 3），
  与 `iterate apply` / `persona show` 守卫一致。**breaking 行为变化**：未知 persona 不再返 `[]`。
- `finetune list <persona>` 同样加固：未知 → NotFound（exit 3）。CLI 错误码契约统一。

### Known Limitations
- 无新增（沿用 v0.3.2 的三条）。

---

## [0.3.2] - 2026-08-03 — Phase 2.5.2 examples 模板（vendor CLI + avc.toml + README）

### Added
- **`examples/vendor-cli/kling-video.sh`** — CliVideoProvider 三段式 mock 模板
  （`submit / status / fetch`，KV-flavor stdout，缺 ref 显式 exit 3 失败）。
- **`examples/vendor-cli/kling-avatar-fin.sh`** — Avatar SFT 三段式 mock 模板
  （`finetune submit / status / fetch`，写占位 PNG magic + 256B random）。
- **`examples/vendor-cli/elevenlabs-voice-fin.sh`** — Voice SFT 三段式 mock 模板
  （同上协议，写 RIFF/WAVE header + 512B 假 PCM）。
- **`examples/vendor-cli/aws-s3-cp.sh`** — `[export.s3].upload_cmd` mock 模板
  （写本地 mirror + sha256 log，CI 无 key 也能跑）。
- **`examples/avc.toml.template`** — 完整 6 段配置（llm / embed / avatar / voice /
  video / shell / safety / export.s3），每段给真实 vendor 替换示例。
- **`examples/README.md`** — 30 秒 e2e 试用（init → render run → finetune start →
  job export s3://）+ 真 vendor 替换说明（kling / ElevenLabs API 路径）。
- 集成 1 个新增（`examples_vendor_cli_templates_run_e2e` 验证 render run 走
  kling-video.sh + job export 走 aws-s3-cp.sh 端到端）。**累计 79 单测 + 64 集成
  = 143 个测试全过**。

### Changed
- README / docs / `docs/status.md` 引用 `examples/` 目录。

### Known Limitations
- examples 模板只 mock "happy path"（submit → status=done → fetch 写占位文件）；
  真 vendor 接 kling / ElevenLabs / aws-cli 时需要把每个 case 分支换成 `curl` + `jq`。
- 模板对 set -u 严格（未定义变量会报错）；POSIX sh 兼容（dash / bash 都跑）。

---

## [0.3.1] - 2026-08-03 — Phase 2.5.1 多维 drift（face / voice / style）

### Added
- **migrations/0002_drift_dimensions.sql**：`persona_versions` 加 `face_embed` /
  `face_embed_dim` / `face_embed_sha256` / `style_embed` / `style_embed_dim` /
  `style_embed_sha256` 6 列。
- **`svc::drift::Dimension` 枚举 + dimension-generic API**：
  - `fetch_embed(db, name, version, dim)` / `write_embed(db, name, version, dim, &values)`
  - `eval_with_provider(cfg, embed_name, base, seed_text, threshold)`
  - `eval_from_db(db, name, base_v, new_v, dim) -> Option<f32>`
  - 各 dim 的 `seed_text(persona, version)` 是稳定锚点（`persona:<name>:<dim>:<v>`）
  - 旧 `fetch_voice_embed` / `eval_voice_from_db` / `eval_voice_with_provider` 留作薄包装，
    与 Phase 2 兼容。
- **`svc::finetune::run` 三维 drift 评估**：
  - voice scope → 算 voice cosine
  - avatar scope → 算 face cosine
  - 永远算 style cosine（persona style 是全局属性）
  - `DriftReport` 三个字段都填；`passed = present_cosines.iter().all(|c| *c >= threshold)`
  - `RunReport` 增 `face_cosine` / `style_cosine` 字段
- 单测 2 个新增（drift dimension seed text + columns）、集成 1 个新增
  （`drift_writes_all_three_dim_embeds_on_run` 验证三 dim embed 落库 + face 为 None
  scope=voice 时正确）。**累计 79 单测 + 63 集成 = 142 个测试全过**。

### Changed
- `DriftReport.passed` 语义：之前仅看 voice（None = true）；现在看 present dims 全过。
- `RunReport` schema 增 `face_cosine` / `style_cosine`（breaking 改动；调用方需更新）。
- `finetune run` CLI 输出增 2 字段（face_cosine / style_cosine）。

### Known Limitations
- 三维都走 embed.<name> + 不同 seed text；不是真"image embed on avatar PNG"或
  "audio embed on voice WAV"。要真 CLIP / Resemblyzer 等本地模型接入是另一条独立
  项目。本框架的"多维"是"embed 空间的不同种子切片"。

---

## [0.3.0] - 2026-08-03 — Phase 2.5 SFT/S3 闭环 + CLI 完整性

### Added
- **avatar / voice SFT 端点接通 vendor CLI**（Phase 1 那条 ⬜ 终于 ✅）：
  - `OpenAiCompatAvatarProvider::finetune` / `OpenAiCompatVoiceProvider::finetune` 在
    `[provider.<dim>.<name>].binary` 配置下真 spawn vendor CLI（仿 CliVideoProvider 三段式
    `finetune submit / status / fetch`，5 min poll + `TempFileGuard` 兜底清 tmp）
  - 协议：`binary finetune submit --<ref-image|ref-audio> <paths...>` → stdout `task_id=...`
  - `--ref-image`（avatar）/ `--ref-audio`（voice）`finetune status --task-id <id>` → `status=done`
  - `finetune fetch --task-id <id> --out <path>` → 写真 PNG / WAV 文件
  - 未配 `binary` 时按既有行为报 "requires a vendor CLI binary"
- `svc::finetune::run(fj_id, embed_provider)` 端到端：拉 `persona_samples` kind=image/audio
  → materializes 到 tmp → 调 Provider finetune → 写 target row 音频/头像列 → 调 embed
  Provider 算 voice drift cosine → `publish()` commit (`cosine ≥ threshold`) 或 rollback
  (`< threshold`)。
- `avc finetune run <fj_id> [--embed <name>]` CLI verb：包装 `svc::finetune::run`，
  succeeded → exit 0、failed_drift → exit 4、缺 `--embed`（voice scope 时）→ exit 2。
- **`avc job export --target s3://bucket/prefix/` 接 S3 / 对象存储**：
  - `ExportTarget::S3 { bucket, prefix, upload_cmd }` 走 `[export.s3].upload_cmd` 模板
  - 默认 `aws s3 cp --region us-east-1 {local} s3://{bucket}/{prefix}{name}`
  - 可换 `mc` / `rclone` / 自家脚本（只改 `upload_cmd`）
  - 每条 artifact materializes → tmp → `sh -c` → tmp 即清
  - `--out <dir>`（local FS）和 `--target s3://` 互斥
- **CLI 完整性补齐**（docs 里有但代码没实现的 verbs）：
  - `avc finetune show <id>` — fj 详情（base/target/scope/status/started_at/finished_at/drift_report）
  - `avc finetune report <id>` — drift_report_json 单独输出（管道友好）
  - `avc finetune cancel <id>` — 标 `cancelled`（仅 queued/running；succeeded/failed_drift/cancelled 拒）
  - `avc iterate show <id>` / `avc iterate cancel <id>` — 同上模式
  - `avc job wait <id> --until <status> [--timeout <secs>] [--poll <ms>]` — 阻塞轮询，
    CI 友好（timeout → exit 4，到达 terminal `failed`/`cancelled` → exit 4）
  - `avc job cancel <id>` — 标 `cancelled`（仅 queued；running 拒）
- 单测 7 个新增（export 单元：local / s3 happy / s3 fail / s3 missing / not-found /
  shell_quote / sanitize）+ 集成 14 个新增（finetune run e2e + 缺 --embed；finetune
  show/report/cancel 4 个；iterate show/cancel 2 个；job cancel/wait 4 个 + 守卫 2 个）。
  **累计 77 单测 + 62 集成 = 139 个测试全过**。

### Changed
- `svc::render::export_artifacts(db, job_id, target: ExportTarget<'_>)` 签名变：之前是
  `&Path`，现在是 `ExportTarget` 枚举（`Local(&Path)` | `S3 { ... }`）；调用方（CLI）相应更新。
- `Config` 加 `export: Option<ExportCfg>` 段（`ExportCfg::s3.upload_cmd`）。
- `Cargo.toml` version 0.2.0 → 0.3.0。

### Known Limitations
- `run_finetune_vendor_pipeline` 是同步调用 tokio runtime（new_current_thread），跑 5 min poll
  时会阻塞当前线程；后续可改后台任务 + cancel token
- face / style anchor embedding 仍未写库，drift 评估仅 voice 一维（与 Status Unplanned 一致）

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
- 单测 22 个 + 集成 12 个新增（v0.2.0 delta；累计 60 单测 + 39 集成 = 99）

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
