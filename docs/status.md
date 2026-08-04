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
| `openai_compat` Avatar 真实现 | ✅ | `OpenAiCompatAvatarProvider` 接 `/v1/images/generations`（OpenAI dall-e-3 / DashScope wanx / 智谱 CogView / Ollama SD 等）；finetune 仍为 stub（vendor SFT 端点 Phase 2）；`provider test avatar.<name>` 探针 |
| `openai_compat` Voice 真实现 | ✅ | `OpenAiCompatVoiceProvider` 接 `/audio/speech`（OpenAI tts-1）；`clone` Phase 1 占位 WAV（OpenAI 无 clone 接口，复用 vendor endpoint）；finetune stub；`provider test voice.<name>` 探针 |
| Cli Video Provider 真实现 | ✅ | `CliVideoProvider` 抽象 vendor-CLI "submit/poll/mp4"（kling-cli / doubao-cli 等）；Phase 1 占位 mp4 BLOB，接口固定 Phase 2 接 vendor；`provider test video.<name>` 探针 |
| Provider 路由表（provider.json + 注册） | ✅ | 已落地：`~/.config/avc/avc.toml` 的 `[provider.<dim>.<name>]` 段 + `extra_headers`，工厂 `make_llm/make_embed/make_avatar/make_voice/make_video(&Config, name)` 解析 |
| avatar / voice SFT 节点真调 Provider | ✅ | `OpenAiCompatAvatarProvider::finetune` / `OpenAiCompatVoiceProvider::finetune` 在 `[provider.<dim>.<name>].binary` 配置下真 spawn vendor CLI（`finetune submit / status / fetch` 三段式 + 5 min poll + TempFileGuard）；`svc::finetune::run(fj_id, embed_provider)` 端到端跑 SFT → drift → publish；CLI `avc finetune run <fj_id> [--embed <name>]`；未配 binary 时仍按既有行为报 "requires a vendor CLI binary" |
| 多维 drift（face / voice / style） | ✅ | migration 0002 加 `face_embed` / `style_embed` 6 列；`svc::drift::Dimension` 枚举 + `fetch_embed / write_embed / eval_from_db / eval_with_provider` 维度泛化；`svc::finetune::run` 三 dim 同时算：voice scope 算 voice、avatar scope 算 face、永远算 style；`DriftReport` 3 字段都填；`passed = present_dims.all(>= threshold)`；`RunReport` 增 `face_cosine` / `style_cosine` |
| examples 模板（vendor CLI + avc.toml） | ✅ | `examples/vendor-cli/{kling-video,kling-avatar-fin,elevenlabs-voice-fin,aws-s3-cp}.sh` 4 个 mock 模板 + `examples/avc.toml.template` + `examples/README.md`；CI 无 key 也能跑 e2e |
| drift_eval 用 Provider 返回的 embedding 真算 | ✅ | `svc/drift::eval_voice_with_provider` 真发请求到 embed Provider 算 cosine；`finetune drift eval <fj_id> --embed <name> --threshold ...` 子命令；未配 embed 时降级走 DB 已有 vector |
| DAG 引擎真调度 | ✅ | `svc/pipeline::run()` Kahn 拓扑排序 → 顺序执行节点 → 落 `job_steps`（status / attempt / outputs_json / duration_ms）；节点 BLOB 落 `artifacts` 表（base64 解码 + sha256）；失败 → job status='failed' + error_json |
| LLM chat 真解析 NL → plan | ✅ | `avc ask "..."` 发请求 → 解析为 `Plan JSON` → 验证白名单 atom → read_only 自动跑 / write 在 TTY 走 y/n、非 TTY 缺 `--yes` 拒绝；支持 `persona list/show/versions/set-traits/set-catchphrase/set-render/commit/promote`；集成测试 `ask_nl_plan_executes_read_only_plan` |
| Shell 内 NL 解析 | ✅ | `avc shell` 输入 NL 时按启发式分类（首 token 在 KNOWN_NOUNS 才走 atomic，否则走 ask::dispatch_nl）。`avc> 列出所有角色` → 真发请求 → 解析 plan → 调 cli::run 执行每步；3 shell unit tests 覆盖分类规则 |
| corpus 切 chunk + embed | ✅ | `svc/corpus::create_from_file()` 双换行/单换行回退切 chunk → 调 embed Provider → 落 `corpus_chunks`；`search` 调 embed API 算 query 向量 + 全表 cosine top-K；CLI: `corpus create/chunks/search/list/attach/detach`；集成测试 `corpus_create_and_search_round_trip` |

---

## Phase 2 — 多 persona 矩阵 + 真实出片

| 模块 | 状态 |
|------|------|
| video.render DAG 节点（script_gen / tts / img_gen / i2v / compose） | ✅ | `Pipeline::run` 5 节点 + 真 vendor CLI 接入（Phase 2.1） + `job export` 落 FS + `job feedback` 写样本（Phase 2.2/2.3） |
| artifacts BLOB 落库 + export | ✅ | `svc::render::export_artifacts` 双 target：`ExportTarget::Local` 落 `<out_dir>/<kind>__<name>__<id>.bin`（CLI `--out <dir>`，Phase 2 行为零回归）；`ExportTarget::S3 { bucket, prefix, upload_cmd }` 调 `sh -c` 跑 `[export.s3].upload_cmd` 模板上传每个 artifact，tmp 完即清（CLI `--target s3://bucket/prefix/`）。upload_cmd 默认 `aws s3 cp --region us-east-1 {local} s3://{bucket}/{prefix}{name}`，可换 `mc` / `rclone` / 自家脚本。 |
| feedback 路径（`avc job feedback`） | ✅ | `svc::render::feedback()` 写 `persona_samples(kind='feedback', source='user_feedback')`；CLI `avc job feedback <job_id> --looks-unlike [--reason ...]` |
| render pack（topics-file 批跑） | ✅ | `svc::render::pack(persona, version?, topics_file)` 逐行读 topic → 串调 create_job + pipeline.run；失败不中断 + 返 `(job_ids, errors)`；CLI `avc render pack <persona> --topics-file <path>`；任一失败 exit 4 供 CI 探测 |

---

## Phase 3+（按需扩展，不在主线）

- 多用户 / 跨机迁移（与本框架无关，独立项目）
- Web UI（不属于内核）
- Provider 健康检查 / 限速策略增强
- 升级阈值到 100+ persona 的 side-file 拆分

---

## 测试矩阵

```
tests/integration.rs           64 tests
├── version_and_help
├── init_idempotent_guard
├── persona_lifecycle_json
├── refine_changes_persist
├── finetune_creates_v2_then_publish
├── finetune_publish_failed_drifts_rollback
├── finetune_rejects_missing_base_version       [Phase 0] 缺失 base → NotFound
├── finetune_rejects_non_ready_base_version     [Phase 0] building → Conflict
├── finetune_rejects_duplicate_target_version   [Phase 0] 重复 target → Conflict，target/job 不重复写入
├── finetune_concurrent_starts_are_conflicts    [Phase 0] 3×8 并发：每轮 1 成功 + 7 exit 4，无 BUSY/exit 20
├── render_rejects_missing_version              [Phase 0] 缺失 version → NotFound
├── render_rejects_non_ready_version            [Phase 0] building → Conflict
├── config_set_get_round_trip                   [Phase 0] 点号路径 round-trip
├── config_rejects_empty_provider_name          [Phase 0] set/get 对空 name 对称拒绝
├── ask_without_llm_errors
├── ask_with_real_llm_round_trip                [Phase 1] ask 真发请求到 OpenAI 兼容 LLM（最小 HTTP 端点）
├── ask_nl_plan_executes_read_only_plan         [Phase 1] NL→plan JSON→真执行
├── provider_test_unknown_llm_name_says_not_configured   [Phase 1] provider test 未配置
├── provider_test_unsupported_dim               [Phase 1] 已被 B1 替换为真探针
├── provider_test_embed_unknown                 [Phase 1] 不存在的 embed provider
├── provider_test_avatar_unknown                [Phase 1] 不存在的 avatar provider
├── provider_test_voice_unknown                 [Phase 1] 不存在的 voice provider
├── provider_test_video_unknown                 [Phase 1] 不存在的 video provider
├── corpus_create_and_search_round_trip         [Phase 1] corpus 切 chunk + embed + search
├── finetune_drift_eval_requires_voice_embed_on_base        [Phase 1] drift eval 缺 voice_embed → Conflict
├── finetune_drift_eval_with_provider_uses_embed_api        [Phase 1] drift eval 真发请求到 embed Provider
├── render_run_executes_full_pipeline_and_produces_artifacts [Phase 1] render run 真走 5 节点 DAG + artifacts 落库
├── render_avatar_provider_posts_exact_script_and_persists_png          [Phase 2] avatar 真发请求
├── render_avatar_http_429_fails_img_gen_without_downstream_work         [Phase 2] avatar 429 不污染下游
├── render_voice_provider_posts_exact_script_and_persists_audio          [Phase 2] voice 真发请求
├── render_voice_http_429_fails_tts_without_downstream_work              [Phase 2] voice 429 不污染下游
├── job_export_writes_artifacts_to_fs                                    [Phase 2] job export
├── job_feedback_writes_sample_with_kind_feedback                        [Phase 2] job feedback
├── job_feedback_without_flag_returns_arg_error                          [Phase 2] job feedback 参数校验
├── render_pack_runs_multiple_jobs_from_topics_file                      [Phase 2] render pack 正路
├── render_pack_skips_empty_topics_file                                  [Phase 2] render pack 空文件
└── render_pack_requires_topics_file                                     [Phase 2] render pack 缺文件
├── finetune_run_via_vendor_cli_writes_target_and_publishes           [Phase 2.5] finetune run 端到端（mock vendor CLI + mock embed HTTP）
├── finetune_run_without_embed_arg_for_voice_scope_errors             [Phase 2.5] voice scope 缺 --embed → Arg 守卫
├── job_export_to_s3_target_invokes_upload_cmd                        [Phase 2.5] export --target s3:// 走 mock upload_cmd
├── job_export_rejects_both_out_and_target                            [Phase 2.5] --out + --target 互斥守卫
├── job_export_requires_out_or_target                                 [Phase 2.5] 缺 --out/--target → Arg 守卫
├── finetune_show_returns_job_details / _unknown_returns_notfound     [Phase 2.5] finetune show
├── finetune_report_without_drift_conflicts / _after_publish_returns  [Phase 2.5] finetune report
├── finetune_cancel_running_then_re_cancelled / _after_publish        [Phase 2.5] finetune cancel
├── iterate_show_and_cancel_happy_paths / _show_unknown_returns       [Phase 2.5] iterate show/cancel
├── job_cancel_queued_and_wait_until_succeeded                        [Phase 2.5] job cancel + wait 四场景
├── job_wait_unknown_id_returns_notfound                              [Phase 2.5] wait 守卫
└── job_wait_missing_until_flag_arg_errors                            [Phase 2.5] wait 缺 --until 值 → Arg
└── drift_writes_all_three_dim_embeds_on_run                         [Phase 2.5.1] finetune run 三 dim embed 落库
└── examples_vendor_cli_templates_run_e2e                             [Phase 2.5.2] examples 模板 e2e（kling-video.sh + aws-s3-cp.sh）
├── iterate_list_returns_apply_records / _unknown_persona_errors     [Phase 2.5.3] iterate list 守卫
├── iterate_apply_set_knowledge_merges_binding / _set_manifest_merges_render_options [Phase 2.5.3] 三段 merge 语义
├── iterate_apply_three_sections_together / _missing_version_errors / _unknown_persona_errors [Phase 2.5.3] apply 三段 / Arg / NotFound
├── persona_show_returns_full_row_json / _unknown_errors              [Phase 2.5.3] persona show 守卫
└── persona_versions_lists_after_finetune                             [Phase 2.5.3] versions 在 finetune start 后返 [1,2]
└── iterate_show_parses_changes_json_as_object                       [Phase 2.5.3] show 把 changes_json 解析为 object
└── persona_list_filters_by_status                                   [Phase 2.5.3] list --status 过滤四场景

src/**/[cfg(test)]              79 unit tests
├── ask::dispatch_nl                                                  4
├── provider::real (OpenAiCompat LLM/Embed/Avatar/Voice + 解析)        15
├── shell::dispatch (启发式分类规则)                                  3
├── svc::corpus (chunk / embed / search)                              3
├── svc::drift (cosine / eval 工具)                                   4
└── svc::pipeline (DAG 拓扑 + 节点执行 + outputs + artifacts)         31

合计：155 个测试全过（79 单测 + 76 集成）。
```

CI：

- `.github/workflows/rust.yml`：stable toolchain + `cargo build --locked --all-targets` + `cargo test --locked --all-targets` + `cargo clippy`（warning 不阻断）
- `.github/workflows/docs.yml`：mkdocs build --strict → GitHub Pages
