# 模块设计：视频生成（Video Generation）

> 用一个**人物角色模型（指定版本）** + 脚本 + 渲染选项，产出最终视频。它是演进的"消费者"——直接消费 `PersonaModelVersion` 的不可变目录，避免受后续训练影响。

---

## 1. 模块目标

**输入**：
- `persona_model_id` + `version_id`（不指定 = 当前默认版本）
- `Script`（含分镜）+ 渲染选项

**输出**（落 `~/.local/share/avc/media/jobs/{job_id}/`）：
- `final.mp4`
- `cover.jpg`
- `subtitle.srt`
- `meta.json` —— **包含 `persona_version_id` 与所有 provider 快照参数**

**边界**：复用 persona 资产；视频生成过程不污染 persona 模型。

---

## 2. 数据契约

```rust
struct Scene {
    idx: u32,
    narration: String,
    visual_prompt: String,
    avatar_action: Option<String>,
    duration_ms: u32,
    emotion: String,
    motion_strength: f32,
    camera: String,
    ref_image_hint: Option<String>,
}

struct Script {
    id: String,
    persona_model_id: String,
    persona_version_id: u32,         // 锁定，生成时不再变动
    topic: String,
    template_id: Option<String>,
    scenes: Vec<Scene>,
    bgm_id: Option<String>,
    style_overrides: Value,
    duration_ms: u32,
    created_at: DateTime<Utc>,
}

struct VideoJob {
    id: String,
    script_id: String,
    persona_model_id: String,
    persona_version_id: u32,         // 锁定，永不漂移
    status: JobStatus,
    progress: f32,
    options: JobRenderOptions,
    artifacts: Artifacts,
    error: Option<JobError>,
    created_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
}

struct JobRenderOptions {
    resolution: String,              // 720p / 1080p / 4k
    aspect_ratio: String,
    fps: u32,
    enable_subtitle: bool,
    subtitle_style: Option<Value>,
    enable_bgm: bool,
    enable_watermark: bool,
    priority: u8,                    // 1~10
    webhook_url: Option<String>,
    extra: Value,
}
```

> `persona_version_id` 必须显式记录。即使后续 persona 训练到 v5，已渲染视频永远锁死在当时的版本。

---

## 3. 端到端流水线

```
Script（已绑定 persona_version_id）
  │
  ▼
[1] 脚本预处理     ── 校验、合规审核、人工编辑（可选）
  │
  ▼
[2] 旁白 TTS       ── 调用 vN.voice
  │                  并发（按 Scene 分片）
  ▼
[3] BGM 匹配       ── 按场景情绪选
  │
  ▼
[4] 关键帧生成     ── 复用 vN.avatar
  │
  ▼
[5] 图生视频 (i2v) ── Kling / CogVideoX / AnimateDiff
  │
  ▼
[6] 口型同步       ── wav2lip / SadTalker（商用数字人可跳）
  │
  ▼
[7] 后期合成       ── 拼接、字幕、BGM、水印
  │
  ▼
[8] 封装输出       ── 写 media/jobs/{job_id}/ + meta.json（含 version）
```

详细节点 / 调度见 [`pipeline.md`](./pipeline.md)。

---

## 4. 脚本生成

### 4.1 输入
- `persona_model_id` + `version_id`
- `topic` + `key_points` + `target_duration` + 可选模板

### 4.2 Prompt 组装
```text
[系统] 你是一名分镜师，根据"主题"与"知识点"生成分镜。
[角色人设] {persona_descriptor.traits, .tone, .catchphrases, .taboos, .scenario_prompts}
[领域知识] {retrieved_chunks}    # 仅当 knowledge 已绑定
[主题] {topic}
[关键点] {key_points}
[时长] {target_duration} s
[景别偏好] {camera_pref}
[输出 JSON Schema] {schema}
```

### 4.3 后处理
- 校验：每 Scene 时长之和 = 总时长 ± 10%
- 拆分：单 Scene > 15s 强制拆分
- 兜底：LLM 输出非法 → 用模板

### 4.4 人机协同
- 脚本生成后返回给开发者，可 `avc render script edit --patch ...` 编辑后再渲染
- 编辑以 JSON Patch 提交，保留 diff
- 编辑后调用渲染时仍使用同一 `persona_version_id`

---

## 5. 音频生成（TTS）

- `voice.synth(voice_id, text, ssml)` —— `voice_id` 取自该版本的 voice 资产
- SSML：情绪、停顿、重音、语速
- 每 Scene 并发；> 300 字自动切分
- 输出 `word_timestamps` 用于字幕对齐

---

## 6. 画面生成

### 6.1 关键帧
- 复用 `personas/pm_xxx/vN/avatar/` 下所有资产
- face_id / LoRA / InstantID 由 Provider 自动应用
- **永远读快照**，绝不读"当前默认版本"

### 6.2 图生视频
- 输入：关键帧 + 音频（驱动口型）
- **远端模型 API**：Kling / Doubao Seedance / 即梦 / Pika / Runway / Replicate 上的 CogVideoX
- 默认 5s 起步，可拼接
- AVCore 不加载任何视频模型，仅持有 Provider 返回的 clip URL / 临时下载产物

### 6.3 商用数字人替代
- 当 `PersonaVersion.mode = digital_human` 时，直接调用 HeyGen / D-ID / 商汤如影 等商用 API
- 跳过关键帧 + i2v
- 所有调用通过 token 鉴权

---

## 7. 后期合成

```rust
fn compose(scenes: Vec<SceneClip>, bgm: Audio, opts: &JobRenderOptions) -> Video {
    let mut tl = concat(scenes, transitions=auto_transition(&scenes));
    if opts.enable_subtitle { tl = burn_subtitle(tl, &scenes); }
    if opts.enable_bgm      { tl = mix_bgm(tl, &bgm, vol=0.15); }
    if opts.enable_watermark{ tl = overlay_wm(tl, &opts.tenant_wm); }
    encode(tl, opts.resolution, opts.fps)
}
```

转场策略：按 Scene 情绪自动 fade / cut / slide。

---

## 8. CLI 用法

```bash
avc render video \
  --persona lily \
  --version 2 \
  --topic "牛顿第一定律" \
  --key-points "定义,示例,应用" \
  --duration 60 \
  --resolution 1080p \
  --webhook https://example.com/cb

avc job show job_xxx --watch

# 产物
avc job open job_xxx     # 用文件管理器打开产物目录
cat media/jobs/job_xxx/meta.json | jq .
```

`meta.json` 例：

```json
{
  "job_id": "job_xxx",
  "persona_model_id": "pm_01H...",
  "persona_version_id": 2,
  "topic": "牛顿第一定律",
  "duration_ms": 60000,
  "providers": {
    "tts": { "name": "cosyvoice", "version": "v0.6" },
    "video": { "name": "kling", "version": "v1.2" }
  },
  "render_options": { ... },
  "created_at": "...",
  "finished_at": "..."
}
```

---

## 9. 接口（程序化）

虽然 AVCore 是 CLI 优先，但仍提供 Rust crate 给上层集成：

```rust
use avc::{Avc, Model, RenderOptions};

let avc = Avc::open_default()?;                    // ~/.local/share/avc
let model = avc.persona("lily")?.version(2)?;
let job = avc.render().video(
    &model,
    &RenderOptions::default()
        .topic("牛顿第一定律")
        .key_points(["定义", "示例", "应用"])
        .duration(Duration::from_secs(60))
).await?;

let result = job.wait().await?;
println!("mp4: {}", result.video_path);
```

---

## 10. 任务状态机

```
queued ──▶ running ──┬──▶ succeeded
                     ├──▶ failed ──▶ retry ──▶ queued
                     └──▶ cancelled
```

- `succeeded` / `failed` / `cancelled` 都写 `meta.json`（含 reason）

---

## 11. 与持续演进的关系

| 场景 | 行为 |
|------|------|
| 用户对成片反馈"不像本人" | `avc job feedback` → 进 `persona_samples` → 下次 evolve 自动消费 |
| persona 已升级到 v5 | 已渲染视频继续绑其 v1/v2/v3，不重新生成 |
| 想用最新效果 | 新建脚本时不传 `--version`（走默认） |
| 想做 A/B | `--versions 3,4 --ab-ratio 50/50`（Phase 2 支持） |

---

## 12. 关键指标

- 60s 视频端到端 P95 ≤ 8 min
- 渲染成功率 ≥ 95%
- 字幕对齐误差 ≤ 200 ms
- 口型同步相似度 ≥ 0.80

---

## 13. 上下游

- **上游**：
  - [persona-modeling.md](./persona-modeling.md) / [persona-evolution.md](./persona-evolution.md) 提供 persona + version
  - [knowledge-aspect.md](./knowledge-aspect.md) 提供检索召回
  - [pipeline.md](./pipeline.md) 提供编排
- **下游**：CLI / crate 消费产物；用户反馈回灌到 evolution
