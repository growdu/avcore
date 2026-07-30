# 模块设计：人物形象建模（Character Modeling）

> 把"角色设定"变成可在视频生成中复用的 **形象资产** + **声音资产**。

---

## 1. 模块目标

输入：自然语言设定（外貌、风格、年龄、气质）+ 可选参考图 + 可选声音样本
输出：`Avatar`（形象资产） + `Voice`（声音资产），可绑定到 `Character`

边界：**不做**脚本生成、**不做**长记忆、**不做**领域知识。

---

## 2. 数据契约

```python
@dataclass
class AvatarSpec:
    name: str
    description: str                # 自然语言描述
    style_tags: list[str]           # 风格关键词：写实 / 二次元 / 国风 ...
    ref_images: list[URI] = []      # 参考图（已获授权）
    age_range: tuple[int, int] | None = None
    gender: str | None = None
    ethnicity_hint: str | None = None
    extra: dict = field(default_factory=dict)

@dataclass
class Avatar:
    id: str
    provider: str                   # sdxl / kling-avatar / ...
    primary_image: URI              # 主形象图
    ref_images: list[URI]
    lora: URI | None = None         # LoRA 权重（如有）
    style_id: str | None = None     # 厂商模板 ID
    face_id: str | None = None      # faceid / instantid 标识
    meta: dict

@dataclass
class VoiceSample:
    uri: URI
    duration_ms: int
    text: str                       # 该段对应文本（用于训练对齐）
    language: str = "zh"

@dataclass
class VoiceSpec:
    name: str
    language: str = "zh"
    gender_hint: str | None = None
    emotion_baseline: str = "neutral"
    samples: list[VoiceSample]      # 0 条 = 走商用音色
    extra: dict = {}

@dataclass
class Voice:
    id: str
    provider: str                   # cosyvoice / gpt-sovits / volc-tts
    voice_id: str                   # 厂商侧 ID
    sample_uri: URI
    language: str
    supported_emotions: list[str]
    meta: dict
```

---

## 3. 形象子能力

### 3.1 形象生成
- **文生图**：调用 SDXL / Flux / HunyuanDiT，按 `description + style_tags` 渲染 1~N 张候选
- **图生图**：基于参考图做风格 / 表情统一
- **多视角**：同 seed + 视角 prompt，产出 4~8 视角
- **人脸一致性**：InstantID / IP-Adapter / face_id 锁定

### 3.2 形象微调（可选）
- 若提供 ≥ 5 张高质量参考图，触发 LoRA 微调
- 微调产物（safetensors + meta）入对象存储
- 微调耗时较长，作为异步任务

### 3.3 形象资产能力
- **主形象图**：用于缩略图 / 头图 / 形象卡
- **关键帧集**：用于后续 i2v 渲染参考
- **face_id 标识**：用于运行时绑定

---

## 4. 声音子能力

### 4.1 声音克隆
- 样本要求：≥ 30s 干净人声、单人、无背景音乐
- 调用 CosyVoice / GPT-SoVITS / F5-TTS，返回 `voice_id`
- 异步任务，支持进度回调

### 4.2 商用音色
- 不提供样本时，可选用预置音色
- 走火山 / 阿里 / 微软 TTS 商用 ID

### 4.3 声音控制
- **SSML** 标记：情绪、停顿、重音、语速
- **参数控制**：stability / similarity / style exaggeration
- **多情绪**：同一 voice_id 切换情绪

---

## 5. 接口

```http
POST   /v1/avatars                        创建形象（异步任务）
GET    /v1/avatars/{id}                   查询形象
POST   /v1/avatars/{id}/render            渲染一张图
DELETE /v1/avatars/{id}                   删除

POST   /v1/voices                         创建声音（异步任务）
GET    /v1/voices/{id}                    查询声音
POST   /v1/voices/{id}/synthesize         TTS 试听
DELETE /v1/voices/{id}                    删除
```

---

## 6. 异步任务与状态

形象 / 声音生成耗时较长（10s ~ 数分钟），统一为异步任务：

```json
{
  "task_id": "uuid",
  "type": "avatar.create",
  "status": "queued | running | succeeded | failed",
  "progress": 0,
  "result": { "avatar_id": "..." },
  "error": null
}
```

前端通过 WebSocket / 轮询 / Webhook 获取完成事件。

---

## 7. Provider 适配

| 任务 | Provider | 备注 |
|------|----------|------|
| 形象 | `sdxl_ip_adapter` | 自托管，性价比高 |
| 形象 | `kling_avatar` | 商用稳定 |
| 形象 | `heygen_avatar` | 商用 |
| 声音 | `cosyvoice` | 自托管，中文好 |
| 声音 | `gpt_sovits` | 少样本克隆 |
| 声音 | `volc_tts` | 商用音色 |
| 声音 | `azure_tts` | 商用音色 |

切换 Provider 时，仅需替换实现，业务接口不变。

---

## 8. 错误与边界

| 场景 | 处理 |
|------|------|
| 参考图模糊 / 多张脸 | 拒绝，返回 `invalid_ref_image` |
| 声音样本含 BGM / 多说话人 | 拒绝，返回 `invalid_audio_sample` |
| 未提供声音样本 | 走商用音色，自动匹配 |
| LoRA 训练失败 | 重试 1 次 → 退回非微调路径 |
| 厂商限速 | 切到备选 Provider，记录埋点 |

---

## 9. 合规

- 形象：上传参考图必须勾选"已获授权"
- 声音：声音克隆必须上传"被克隆人授权书"（托管存证）
- 输出水印：默认生成时烧录不可见水印（用于追溯）

---

## 10. 关键指标

- 形象生成 P50 ≤ 15s，P95 ≤ 60s
- 声音克隆 P50 ≤ 90s，P95 ≤ 5min
- 形象一致性评分（CLIP / face embedding）≥ 0.85
- 声音相似度（speaker embedding cosine）≥ 0.80
