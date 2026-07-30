# CLI / REPL 用法

> AVCore 把"创建 persona / 训练 persona / 出片"都暴露成 CLI 命令。无需启动 server、无需浏览器。本文件给出**核心命令、典型用法、交互式 REPL**，以及**错误约定**。

> 命令一览会随 Phase 0 推进收敛；本文件描述稳定语义。

---

## 1. 安装与首次启动

```bash
# 安装（计划中）
cargo install avc

# 或从源码
git clone https://github.com/growdu/avcore
cd avcore && cargo build --release
./target/release/avc --version

# 初始化本地状态目录（幂等）
avc init
# → created ~/.local/share/avc/
#   ├── avc.db (sqlite)
#   ├── personas/
#   ├── media/
#   └── cache/

# 配置 Provider（**必须** token，所有调用走远端 API）
avc config set provider.avatar.kling.api_key   "klg_..."
avc config set provider.voice.elevenlabs.api_key "el_..."
avc config set provider.llm.openai.api_key     "sk-..."
avc config set provider.video.kling.api_key    "klg_..."

# 查看当前配置（secret 默认 mask）
avc config show
# 等价：avc config show --reveal-secrets  # 真要看需要显式 flag
```

> 所有命令都支持 `--home <path>` 指向自定义根目录，便于多环境。

---

## 2. PersonaModel 生命周期

### 2.1 创建 v1

```bash
avc persona new "Yu" \
  --description "数据库内核领域讲师，数据库内核专家" \
  --avatar-style 写实,教学 \
  --avatar-refs ./samples/ref_*.png \
  --voice-samples ./samples/voice_*.wav \
  --persona-traits 耐心,严谨,幽默 \
  --persona-catchphrase "我们直接看源码" \
  --from ./persona.toml          # 或显式参数
```

`persona.toml` 写法：
```toml
[persona]
name = "Yu"
archetype = "db_kernel_expert"
description = "..."

[avatar]
provider = "sdxl_ip_adapter"
style_tags = ["写实", "教学"]
ref_images = ["./samples/ref_1.png", "./samples/ref_2.png"]
age_range = [25, 35]

[voice]
provider = "cosyvoice"
language = "zh"
samples = [
  { uri = "./samples/voice_1.wav", duration_ms = 42000, text = "..." }
]

[persona_descriptor]
traits = ["耐心", "严谨", "幽默"]
tone = "严谨"
catchphrases = ["我们直接看源码"]
taboos = ["绝对化表述"]
formality = 0.6

[knowledge]   # 可选
binding = "loose"
corpus = "./samples/physics.md"
domain = "数据库内核"
```

执行后：
```bash
# 创建是异步任务（涉及 provider 调用）
Created task: task_01H...  persona-model.create
Status: running
Avatars:    40%
Voice:      0%
Persona:    pending
Knowledge:  pending

# 跟进度
avc task show task_01H... --watch

# 完成
avc task show task_01H...
→ status: succeeded
→ persona_model_id: pm_01H...
→ version_id: 1
```

### 2.2 查看

```bash
avc persona show yu                          # 概要
avc persona show yu --version 2              # 特定版本
avc persona list                               # 所有 persona
avc persona versions yu                      # 历史版本
avc persona inspect yu --version 2           # 详细结构 (从 avc.db 读)
avc persona dump yu --version 2 --out ./dump/# 一次性导出可读目录（只读视图，不写回 DB）
```

### 2.3 切默认版本

```bash
avc persona current yu --set 2               # 让新任务默认用 v2
avc persona current yu --set 3               # 切到 v3
```

切版本不影响已经渲染的视频。

### 2.4 归档

```bash
avc persona archive yu                       # 软删除（.archive 后缀）
avc persona restore yu                       # 恢复
avc persona prune --older-than 30d             # 物理清理过期归档
```

---

## 3. 持续训练（完善角色）

### 3.1 追加样本

```bash
# 图像样本
avc persona sample add yu \
  --kind image \
  --uri ./samples/yu_side_view.png \
  --tags side,neutral \
  --consent ./samples/yu_consent.pdf

# 声音样本
avc persona sample add yu \
  --kind audio \
  --uri ./samples/yu_new_voice.wav \
  --duration-ms 60000 \
  --text "..." \
  --consent ./samples/yu_voice_auth.pdf

# 行为样本（对话/语录）
avc persona sample add yu \
  --kind behavior_text \
  --text "今天我们换个角度想想这个问题..." \
  --tags teach,patience
```

### 3.2 启动训练

```bash
avc persona evolve yu \
  --scope avatar,voice,persona \
  --base-version 2 \
  --anchors ./samples/canary/   # 金丝雀样本（必须不像漂移）
  --consistency-threshold 0.85 \
  --fallback-to-base
```

训练是异步任务：

```bash
avc task show task_02J... --watch

Steps:
  [✓] sample_filter       120/120 samples kept
  [✓] avatar_train        lora@50, lr=1e-4, 3 epochs
  [✓] voice_train         +120s new samples integrated
  [✓] anchor_extract      face/voice/style done
  [✓] drift_eval          vs v2: 0.92 (≥ 0.85)
  [✓] publish             v3 → persona_model_id=pm_01H...
```

训练失败（漂移超阈值）时会自动回退：

```bash
avc task show task_02J...
→ status: failed_drift
→ drift_report: {"avatar":"0.78","voice":"0.91","style":"0.88"}
→ fallback_to_base applied; v3 NOT created
```

### 3.3 训练报告

```bash
avc training report task_02J... --json
```

```json
{
  "persona_model_id": "pm_01H...",
  "base_version": 2,
  "candidate_version": 3,
  "metrics": {
    "identity_consistency": 0.92,
    "style_consistency": 0.88,
    "quality_score": 0.84
  },
  "per_dim_drift": {
    "avatar": {"score": 0.92, "warning": null},
    "voice":  {"score": 0.91, "warning": null},
    "style":  {"score": 0.88, "warning": "tone_more_formal_than_parent"}
  },
  "samples_used": 120,
  "duration_min": 38
}
```

### 3.4 样本治理

```bash
avc persona sample list yu --kind audio
avc persona sample rm sample_01H...
avc persona sample consign sample_01H...    # 标金丝雀（必须不漂移）
avc persona sample stats yu               # 各类样本数量 / 质量分布
```

---

## 4. 出片

### 4.1 即席出片

```bash
avc render video \
  --persona yu \
  --version 2 \
  --topic "InnoDB Buffer Pool 替换算法" \
  --key-points "定义,示例,应用" \
  --duration 60 \
  --resolution 1080p \
  --webhook https://example.com/cb     # 可选
```

输出：

```bash
job_id: job_01H...
status: queued
estimated_seconds: 240
persona_version: 2   ← 锁定

# 跟进度
avc job show job_01H... --watch
  → script_gen done
  → tts 4/6
  → img_gen 2/6
  → i2v 0/6
  → compose pending

# 完成
avc job open job_01H...
  → final.mp4
  → cover.jpg
  → subtitle.srt
  → meta.json  (含 persona_version_id=2)
```

### 4.2 脚本与编辑

```bash
# 分镜（拿脚本对象、再渲染）
avc render script --persona yu --topic "..." --out script.json
avc render script edit script.json --patch 'scenes[0].duration_ms=9000'
avc render video --from-script script.json
```

### 4.3 反馈回灌（驱动持续训练）

```bash
avc job feedback job_01H... \
  --signal looks_unlike \
  --note "侧脸不像本人" \
  --weight 1.0

# 转成 PersonaSample 写入样本池；下次 evolve 自动消费
```

### 4.4 取消 / 重试 / 重渲染

```bash
avc job cancel job_01H...
avc job retry job_01H...        # 重试失败节点
avc job rerender-scene job_01H... --idx 2
```

---

## 5. 知识（可选）

```bash
# 初始化语料
avc corpus new --name "数据库内核" --source-type upload --uri ./physics.md
avc corpus chunks corpus_01H... --from ./physics_chunks.jsonl

# 检索试运行
avc corpus search corpus_01H... --query "InnoDB Buffer Pool 替换算法"

# 绑定到 persona
avc persona knowledge bind yu --corpus corpus_01H... --domain "数据库内核"
avc persona knowledge unbind yu
```

---

## 6. Provider 管理

```bash
avc provider list                              # 已注册 provider
avc provider show sdxl_ip_adapter              # 详情 + 配置字段
avc provider config sdxl_ip_adapter --set batch_size=4
avc provider test sdxl_ip_adapter              # 连通性测试
```

主流 Provider 在 Phase 1 起步时覆盖（**全部 token 鉴权 API**）：

| 维度 | Provider |
|------|----------|
| 形象 | `kling_avatar`, `heygen_avatar`, `doubao_image`, `seedream`, `replicate_flux_lora` |
| 声音 | `volc_tts`, `azure_tts`, `elevenlabs`, `doubao_tts`, `openai_tts` |
| LLM | `openai_compat`（兼容 OpenAI / Anthropic / DeepSeek / 智谱 / 豆包等） |
| 视频 | `kling`, `doubao_seedance`, `pika`, `runway`, `replicate_cogvideox` |
| 知识 | `openai_embed`, `volcengine_embed`, `alibaba_embed`, `cohere_embed`, `cohere_rerank` |
| 微调 | `openai_compat_sft`, `replicate_trainer`, `kling_avatar_finetune`, `elevenlabs_voice_clone` |

> **强约束**：AVCore 不支持自托管模型。`sdxl_ip_adapter` / `cosyvoice` / `gpt_sovits` 这类需要本地推理的不在 Provider 表中。
> 每个 Provider 是一份 `provider.json` 配置 + 一段 Rust trait 实现。新增 Provider 不需要修改核心代码。

---

## 7. REPL（交互式）

```bash
avc repl
```

```
avc> help
  persona          manage persona models
    new, show, list, archive, restore
    sample add, sample rm, sample list
    evolve, current
  render           generate scripts and videos
  corpus           manage knowledge corpora
  provider         list, show, config, test
  task, job        inspect or watch tasks/jobs
  config, init, verify, prune

avc> persona list
  pm_01H... (Yu)        current=v3   versions=3   status=active
  pm_02H... (Dr. Wang)    current=v1   versions=1   status=active

avc> persona show yu
  id: pm_01H...
  current: v3
  v1: archived (initial)
  v2: archived (refined voice)
  v3: active
  storage: ~/.local/share/avc/personas/pm_01H.../v3/

avc> persona evolve yu --scope voice --add ./new_voice.wav
  task_02J... started; watching...
  ...
  ✓ drift_eval passed (0.91)
  ✓ published v4

avc> render video --persona yu --topic "..."
  job_03K... started; watching...
  ...
  ✓ succeeded → media/jobs/job_03K.../final.mp4

avc> exit
```

REPL 上下文：

- 上一条命令结果会被缓存为 `$LAST` 变量：
  ```
  avc> persona show yu
  avc> persona sample list $LAST.id --kind audio
  ```
- 多行输入：命令以空行结束
- Tab 补全、命令历史（`~/.local/share/avc/repl_history`）

---

## 8. 通用约定

### 8.1 输出
- 默认人类可读（带颜色，TTY 下）
- `--json` 输出 JSON，便于脚本
- `--quiet` 只输出必要字段
- 进度类命令默认走 `--watch`，按下 q 退出

### 8.2 错误
- 退出码：0 ok；1 通用失败；2 参数错；3 资源不存在；4 已废弃/冲突；10+ provider 错误
  - `5` 鉴权失败（如 Provider token 无效 / 过期）
  - `6` token 未配置
- 错误消息统一格式：
  ```
  error[E0403]: persona_not_found
    target: yu
    hint: did you mean "Yu" (capital L)?
  
  error[E0501]: provider_unauthenticated
    provider: provider.avatar.kling
    hint: avc config set provider.avatar.kling.api_key ...
  ```
- **Provider 的 api_key 未配置**：所有调用前 `avc` 强制 preflight；明确提示 `avc config set ...`

### 8.3 退出与清理
- 任何命令 Ctrl+C 都先停下任务，再询问
- 长任务退出前尽可能把进度落盘（断点续跑基础）

---

## 9. 几个常见工作流

### 9.1 跑通首个 persona → 视频（Phase 0 目标）

```bash
avc init
avc config set provider.avatar.sdxl.base_url "..."
avc config set provider.voice.cosyvoice.api_url "..."
avc config set provider.llm.openai_compat.base_url "..."
avc config set provider.llm.openai_compat.api_key "..."

avc persona new "Yu" --from ./samples.toml
avc render video --persona yu --topic "Hello"
open media/jobs/<job_id>/final.mp4
```

### 9.2 持续运营一个角色号

```bash
# 每天上传新样本
avc persona sample add yu --kind audio --uri $(date +%F).wav ...

# 每周跑一次训练
avc persona evolve yu --scope voice --anchors ./canary/ --consistency-threshold 0.85

# 每天发视频
avc render video --persona yu --topic "$(cat topic.txt)" \
  --webhook https://my-service/avc-callback
```

### 9.3 跨机迁移

```bash
# 源机
avc export --persona yu --out yu.tar.zst
scp yu.tar.zst other:

# 目标机
avc import yu.tar.zst
avc persona show yu
```

### 9.4 接入对象存储（**默认不开**；本节供确实需要时参考）

```bash
# AVCore 默认使用本地 FS。只有当本地空间不足、需跨机共享、
# 或团队 ≥ 3 人时再考虑对象存储。
avc storage plugin install s3 --bucket my-bucket
avc persona migrate yu --to s3://my-bucket/personas/pm_xxx/
# 本地缓存保留（热），冷数据进对象存储
```

**默认推荐路径**仍是本地 FS + `avc export` / `import` 跨机迁移。详见 [`storage.md §0`](./storage.md)。
