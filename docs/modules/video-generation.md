# 模块设计：视频生成（Video Generation）

> 把"角色 + 脚本 + 音频 + 画面"装配成最终视频。

---

## 1. 模块目标

输入：`Character` + `Script`（含分镜）+ 渲染选项
输出：完整视频文件（mp4）+ 封面 + 字幕 + 任务日志

边界：**不**重新训练形象 / 声音，复用已建模资产。

---

## 2. 数据契约

```python
@dataclass
class Scene:
    idx: int
    narration: str                   # 旁白文本
    visual_prompt: str               # 画面描述
    avatar_action: str | None        # 表情 / 动作描述
    duration_ms: int
    emotion: str = "neutral"         # 影响 TTS / 表情
    motion_strength: float = 0.5     # 动作幅度
    camera: str = "medium"           # 景别
    ref_image_hint: str | None = None

@dataclass
class Script:
    id: str
    character_id: str
    topic: str
    template_id: str | None
    scenes: list[Scene]
    bgm_id: str | None
    style_overrides: dict
    duration_ms: int
    created_at: datetime

@dataclass
class JobRenderOptions:
    resolution: str = "1080p"        # 720p / 1080p / 4k
    aspect_ratio: str = "16:9"
    fps: int = 30
    enable_subtitle: bool = True
    subtitle_style: dict | None = None
    enable_bgm: bool = True
    enable_watermark: bool = True
    enable_intro: bool = False
    enable_outro: bool = False
    priority: int = 5                # 1~10
    webhook_url: str | None = None
    extra: dict
```

---

## 3. 端到端流水线

```
Script
  │
  ▼
[1] 脚本预处理     —— 校验、合规审核、人机协同（可选编辑）
  │
  ▼
[2] 旁白 TTS       —— 逐 Scene 合成音轨（含情绪 / 停顿 / 时间戳）
  │                   并发
  ▼
[3] BGM 匹配       —— 按场景情绪推荐 / 选择 BGM
  │
  ▼
[4] 关键帧生成     —— 每 Scene 出 1~N 张关键帧（IP-Adapter / lora）
  │                   并发
  ▼
[5] 图生视频       —— 每 Scene 出 5~10s 视频片段
  │                   并发
  ▼
[6] 口型同步       —— 音视频对齐（可选，商用数字人可跳过）
  │
  ▼
[7] 后期合成       —— 拼接、转场、字幕烧录、BGM 混音
  │
  ▼
[8] 封装输出       —— 转封装、生成封面、生成预览 GIF
  │
  ▼
final.mp4 + cover.jpg + subtitle.srt + meta.json
```

详细节点说明见 [pipeline.md](./pipeline.md)。

---

## 4. 脚本生成

### 4.1 输入
- `topic` + `key_points` + `target_duration` + `template_id`
- 可选：参考脚本（让 LLM 模仿风格）

### 4.2 Prompt 组装
```text
[系统] 你是分镜师，根据"主题"和"知识点"生成分镜。
[角色人设] {persona}
[专家语料] {retrieved_chunks}
[主题] {topic}
[关键点] {key_points}
[时长] {target_duration} 秒
[景别偏好] {camera_pref}
[输出 JSON Schema] {schema}
```

### 4.3 后处理
- 校验：每 Scene 时长之和 = 总时长 ± 10%
- 拆分：单 Scene > 15s 时强制拆分为多 Scene
- 兜底：LLM 输出非法 → 用模板生成

### 4.4 人机协同
- 脚本生成后返回给开发者，可编辑后再触发渲染
- 编辑以 JSON Patch 提交，保留 diff

---

## 5. 音频生成（TTS）

### 5.1 调用
- `voice.synthesize(voice_id, text, ssml)`
- SSML 标记插入：
  - 情绪：`<emotion value="happy"/>`
  - 停顿：`<break time="500ms"/>`
  - 重音：`<emphasis level="strong">关键词</emphasis>`

### 5.2 并发
- 每 Scene 一个 TTS 任务，并发执行
- 单段最大长度 300 字符，超长自动切分

### 5.3 时间戳
- 输出 `word_timestamps` / `sentence_timestamps` 用于字幕

---

## 6. 画面生成

### 6.1 关键帧
- 文生图：`prompt = scene.visual_prompt + character.style_prompt`
- 一致性：复用 `avatar.face_id` / `lora` / `instantid`

### 6.2 图生视频（i2v）
- 输入：关键帧 + 音频（驱动口型）
- 模型：Kling / 可灵 / AnimateDiff / CogVideoX
- 时长：5s 起步，可拼接至目标时长

### 6.3 商用数字人替代
- 当 Character 标记 `mode=digital_human` 时，直接调用 HeyGen / D-ID / 商汤如影
- 跳过关键帧 + i2v，只做 TTS + 厂商渲染

---

## 7. 口型同步

- 工具：wav2lip / video-retalking / SadTalker
- 适用：自托管视频路径
- 不适用：商用数字人（厂商已自带）

---

## 8. 后期合成

```python
def compose(scenes: list[SceneClip], bgm: Audio, options: JobRenderOptions) -> Video:
    # 1. 各 Scene 拼接
    timeline = concat(scenes, transitions=auto_transition(scenes))
    # 2. 字幕烧录
    if options.enable_subtitle:
        timeline = burn_subtitle(timeline, scenes, options.subtitle_style)
    # 3. BGM 混音
    if options.enable_bgm:
        timeline = mix_bgm(timeline, bgm, volume=0.15)
    # 4. 水印
    if options.enable_watermark:
        timeline = overlay_watermark(timeline, options.tenant_watermark)
    # 5. 转码
    return encode(timeline, resolution=options.resolution, fps=options.fps)
```

转场策略：按 Scene 情绪自动选 fade / cut / slide。

---

## 9. 接口

```http
POST   /v1/scripts                       生成分镜
PUT    /v1/scripts/{id}                  编辑分镜
GET    /v1/scripts/{id}                  查询

POST   /v1/jobs                          创建渲染任务
GET    /v1/jobs/{id}                     查询
GET    /v1/jobs/{id}/steps               任务步骤
GET    /v1/jobs/{id}/artifacts           产物列表
POST   /v1/jobs/{id}/cancel              取消
POST   /v1/jobs/{id}/retry               重试
POST   /v1/jobs/{id}/rerender-scene      重渲染某个 Scene
```

### 9.1 创建 Job 样例

```http
POST /v1/jobs
{
  "script_id": "scr_xxx",
  "options": {
    "resolution": "1080p",
    "aspect_ratio": "16:9",
    "enable_subtitle": true,
    "enable_bgm": true,
    "webhook_url": "https://example.com/cb"
  }
}
```

### 9.2 返回

```json
{
  "job_id": "job_xxx",
  "status": "queued",
  "estimated_seconds": 240
}
```

---

## 10. 任务状态机

```
queued ──▶ running ──┬──▶ succeeded
                     ├──▶ failed ──▶ (retry) ──▶ queued
                     └──▶ cancelled
```

- `queued`：等待资源
- `running`：至少一个 step 在执行
- `succeeded`：所有 step 成功
- `failed`：任一关键 step 失败且重试耗尽
- `cancelled`：用户主动取消

---

## 11. 进度与回调

### 11.1 进度数据
```json
{
  "job_id": "job_xxx",
  "progress": 0.45,
  "current_step": "i2v",
  "step_progress": {
    "script_gen": 1.0,
    "tts": 1.0,
    "img_gen": 1.0,
    "i2v": 0.5,
    "compose": 0.0
  },
  "eta_seconds": 120
}
```

### 11.2 通道
- **Webhook**：完成 / 失败时 POST
- **WebSocket**：实时进度流
- **SSE**：服务端推送
- **轮询**：`GET /v1/jobs/{id}`

---

## 12. 性能与优化

- **节点级并发**：TTS / img_gen / i2v 全部并行
- **缓存**：相同（character_id + script_hash）命中复用
- **渐进式输出**：先生成低清预览，再异步升级高清
- **分片渲染**：长视频分片并行，最后合并
- **GPU 池化**：Kling / 视频模型独立池

---

## 13. 关键指标

- 60s 视频端到端 P95 ≤ 8 分钟
- 渲染成功率 ≥ 95%
- 字幕对齐误差 ≤ 200ms
- 口型同步相似度 ≥ 0.80
