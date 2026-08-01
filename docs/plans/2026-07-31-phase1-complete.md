# Phase 1 完成 · 全任务清单

> **关键发现：** Phase 0 把 5 个 Provider trait 定义好之后**全部没接线**——`grep -rn "Mock" src/` 显示 mock.rs 没有任何 factory 引用，CLI 也完全不调 Provider。"→ 真" 不仅是替换 mock，而**首次接线** Provider trait。
>
> 本计划把 Phase 1 剩余 7 项合一。LLM/Embed 已在前面两刀完成，这里只剩：
> 1. DriftEvaluator 真算（vs 手 mock）
> 2. LLM NL → 原子计划真解析（ask 真"动手"）
> 3. Avatar 真 Provider
> 4. Voice 真 Provider
> 5. Video 真 Provider
> 6. DAG 引擎真调度（pipeline-svc）
> 7. Shell 内 NL 解析
> 8. Corpus 切 chunk + embed

**Goal:** 把 Phase 1 中所有声明为 ⬜ 的事项做真实现 → 完整跑通"persona onboard → finetune → render run"端到端真链。

**Architecture:**

所有"真 X Provider" 沿用既有模板：
- `OpenAiCompatAvatarProvider` / `VoiceProvider` / `VideoProvider`，对应各家 API 形状各自定义
- 工厂 `make_avatar()`/`make_voice()`/`make_video()` 仿 `make_llm()`/`make_embed()`
- 请求 / 响应 schema 在 `real.rs` 子模块里独立定义

NL 流水线：
- ask：NL → LLM (chat/completions) → JSON plan → 验证 → 原子命令执行（写操作仅 TTY + 已确认）
- Shell：NL → 同上 + 确认 UI

DAG：
- pipeline-svc 的 `execute()` 升级：处理依赖 + 重试 + job_steps 落库
- 至少支持 render.run 这一条链：script_gen → tts + img_gen → i2v → compose → artifacts BLOB

Corpus：
- corpus create (file → chunks + embed)
- corpus search (cosine top-K)
- corpus attach/detach/reindex

Drift 真算：
- finetune publish 增 `--eval` flag 走 embed 真算 vs base voice embed 算 cosine 相似度
- 未配 embed 时仍接受手 mock（已有 `--passed/--failed` 不破坏）

**Tech Stack:** 全用既有依赖（rusqlite / reqwest / tokio / serde），零新依赖。

---

## 任务拆分

### Wave A · 无真外部 API 依赖（最高 ROI）

**A1. DriftEvaluator 真算（走 embed）**
- Files: `src/svc/finetune.rs`, `src/cli/finetune.rs`
- Step 1: `publish()` 增可选 `--embed <name>` flag（CLI），未传则保持手 mock 不破坏。
- Step 2: 把 drift_eval 抽到 `src/svc/drift.rs`，含 `eval_voice(base_embed, new_embed, anchor, threshold) -> DriftReport`。
- Step 3: 增 `drift eval <fj_id> --embed <name>` 独立子命令，无需 publish 也跑。
- Test: `finetune_drift_eval_uses_embed_when_configured`（mock embed server），`finetune_drift_eval_falls_back_to_manual_when_no_embed`。
- 非范围：不替默认 `publish --passed/--failed` 行为，只是新增能力。

**A2. LLM NL → 原子计划真解析**
- Files: `src/ask/mod.rs`, `src/shell/mod.rs`
- Step 1: ask 现已发请求到 LLM，把回显 reply 改成 "解析为 plan JSON"：prompt 提供原子清单 + 当前 schema + ctx。
- Step 2: 解析 JSON：read_only 时自动跑；write 时 TTY 走 y/n；非 TTY 拒绝。
- Step 3: Shell 模式同样接入 NL 路径（plan → 确认 → 执行），并保留原子透传不破坏。
- Step 4: at minimum 支持 `persona list` / `persona show` / `persona set-traits` / `persona set-catchphrase` / `persona set-render` 这 5 个 NL 映射。
- Test: `ask_nl_plan_to_set_traits_executes`（mock LLM 返回 plan → 真读 DB 验 traits 已设）。
- 非范围：finetune / render 等长任务 NL 调度仍属 "Phase 1.3"，独立 plan。

**A3. Corpus 切 chunk + embed**
- Files: `src/svc/corpus.rs`(new), `src/cli/corpus.rs`
- Step 1: 新建 `src/svc/corpus.rs`：`create(file)` 按段落/换行切 chunk 调 embed 写 `corpus_chunks` 表。
- Step 2: `search(corpus_id, query, topk)` 调 embed API 算 query 向量 + 全表 cosine 排序取 top-K。
- Step 3: `corpus attach/detach` 已存在 wire，确认 `corpus create / search` 真接。
- Test: `corpus_create_chunks_and_embed_round_trip`（mock embed server）。

---

### Wave B · 真 Provider trait 接线（avatar / voice / video + DAG 真调度）

**B1. Avatar 真 Provider（OpenAI 兼容 images）**
- Files: `src/provider/real.rs`
- Step 1: `OpenAiCompatAvatarProvider` 调 `/images/generations`（OpenAI DALL-E shape）→ base64 PNG；或 `/v1/images/edits` 多视图打包。
- Step 2: factory `make_avatar(&Config, name)`。
- Step 3: `avc provider test avatar.<name>` 探针。
- 非范围：与 persona-svc 的 attach-avatar 真接，留 wave B5（pipeline）一起做。

**B2. Voice 真 Provider（OpenAI 兼容 TTS + ElevenLabs）**
- Files: `src/provider/real.rs`
- Step 1: `OpenAiCompatVoiceProvider` 暴露 clone / synth / finetune 三接口。OpenAI `/audio/speech` 给 synth；clone 走 ElevenLabs API（因为 OpenAI 无 clone）——为最小工作量走 trait 抽象 + 2 个具体 impl：`OpenAiTtsSynthProvider`（synth only）+ `ElevenLabsVoiceProvider`（clone + finetune）。
- Step 2: factory `make_voice(&Config, name)`。
- Step 3: `avc provider test voice.<name>` 探针。

**B3. Video 真 Provider**
- Files: `src/provider/real.rs`
- Step 1: `KlingVideoProvider` 调 kling API（提交 + 轮询 + 拿 mp4）——简化：mock adapter 同 ref 形态，把"提交-轮询-拿结果"封装为 `CliVideoProvider`，可被模拟（httpbin / mock server）验。
- Step 2: factory `make_video(&Config, name)`。
- Step 3: `avc provider test video.<name>` 探针。

**B4. Pipeline DAG 真调度**
- Files: `src/svc/pipeline.rs`, `src/svc/render.rs`, `src/db/mod.rs`
- Step 1: pipeline `execute()` 升级为可走"script_gen → tts + img_gen → i2v → compose"五节点；节点结果落到 `job_steps` 表。
- Step 2: 重试 + 失败回滚（job 状态 dictated by node status）。
- Step 3: 不接外部 Provider：先走 Mock Provider 真跑这一条链，验 DAG 调度正确（这是 Phase 1 的"骨架先跑通"路径，已接的 LLM/Embed 真 provider 替换对应节点即可）。
- Test: `render_run_full_dag_produces_artifacts_blo b`（mock 五节点，仅跑 DAG）。

**B5. Render → 真 Provider 接线**
- Files: `src/cli/render.rs`, `src/svc/render.rs`
- Step 1: `render run` 在 create_job 后：触发 pipeline.execute(...) 真跑完，artifacts BLOB 落库 + status 更新。
- Step 2: 失败 → job status=failed + error_json。
- Test: `render_run_executes_pipeline_and_succeeds`。

---

### Wave C · 文档 + CI 收尾

**C1. docs/status.md** 全量更新（所有 ⬜ → ✅）
**C2. docs/api/README.md** 加 avatar / voice / video 真实现小节
**C3. docs/cli.md / docs/shell.md** 加 NL→plan 示例
**C4. CHANGELOG.md** 加 Phase 1 完整条目

---

## 验收

* `cargo test --locked --all-targets -- --test-threads=1` 全绿
  * 目标：unit ≥ 15 / integration ≥ 30
* `avc ask --yes "列出所有角色"` 真读 DB 输出 list
* `avc ask --yes "把 Yu 的 traits 改成严谨务实"` 真 set-traits 落库（NL→plan→exec）
* `avc render run --persona yu --topic X` 真走 DAG 五节点，artifacts BLOB 落库（用 Mock Provider 即满足验收）
* `avc provider test {avatar,voice,video,llm,embed}.<name>` 全部覆盖

---

## 不做的明确清单（推到 Phase 2+）

* 真调外部 SFT 端点（avatar.create / voice.clone / avatar.finetune）需要真 token，
  Phase 1 已通过 wire 结构 + Mock 跑通路径，Phase 2 加 token 接通。
* `feedback` 路径 / `render pack` 批跑。
* 移动端 / 多用户 / 跨机。

---

## 执行节奏

* Wave A 先串行（不互相依赖，但都在同一个 module）——由主代理手工做 90 分钟内可完。
* Wave B 同时跑 subagent 并行 3 个（avatar / voice / video provider 各 1）；DAG 与 render 接线由主代理手工。
* Wave C 文档合并 commit。
