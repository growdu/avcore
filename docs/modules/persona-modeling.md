# 模块设计：人物角色模型生成（Persona Modeling）

> 创建一个**人物角色模型（PersonaModel）的初始版本 v1**——可能是一个技术专家、一个形象鲜明的虚构人物、一个真人数字孪生、一个虚拟员工。核心是"一个可被识别、可被驱动的角色"。
>
> 本文档回答：**怎么创建、必须输入什么、产出落在哪里**。与落盘格式的强约定见 [`../storage.md`](../storage.md)。

---

## 1. 模块目标

**输入**：人物设定（自然语言）+ 可选参考素材
- 视觉参考：1~N 张参考图（已获授权）
- 声音样本：≥ 30s 干净人声（已获授权）
- 行为样本：该人物的典型语气 / 文风片段
- 领域资料（可选）：技术专家才需要；非必要不灌
- 参考人物（可选，仅用于风格借鉴）

**输出**：本地落盘的 `PersonaModel v1`，含
- `avatar/` —— 主形象图、多视角、LoRA（若有）、face_id
- `voice/` —— 干净样本 + speaker embedding
- `persona.json` —— 人设结构化
- `knowledge/`（可选）—— 语料索引
- `identity_anchor.json` —— 跨版本一致性锚点
- `manifest.json` —— 版本元数据（账本）

**边界**：本模块**只创建 v1**。再训练由 [`persona-evolution.md`](./persona-evolution.md) 负责。

---

## 2. 行结构（与 storage.md §2 完全一致）

> AVCore 使用单一 SQLite。`persona_versions` 是**宽表**——一行 = 一个 `PersonaModelVersion`，所有资产 BLOB 在该行内。下面是该行涉及的列（详见 [`../storage.md`](../storage.md)）：

```
persona_versions (一行):
  ├── 主键: (persona_model_id, version)
  ├── 父版本: parent_version
  ├── status: building / ready / deprecated
  │
  ├── avatar_* 列
  │     avatar_primary BLOB          # 主形象 PNG
  │     avatar_primary_mime TEXT
  │     avatar_primary_sha256 TEXT
  │     avatar_views_blobs BLOB      # 多视角（zip/拼接）
  │     avatar_refs_blobs BLOB       # 用户上传参考图
  │     avatar_lora_ref_json TEXT    # 远端 model_id 引用（无权重）
  │     avatar_face_id, avatar_provider, ...
  │
  ├── voice_* 列
  │     voice_sample BLOB            # 干净人声 WAV
  │     voice_transcript TEXT
  │     voice_embed BLOB             # speaker embedding
  │     voice_id_remote TEXT         # 远端 voice_id
  │
  ├── persona_descriptor_json TEXT   # 人设 JSON
  ├── knowledge_binding_json TEXT    # 可选绑定
  │
  ├── anchor_face_emb / voice_emb / style_emb BLOB
  │     + 对应 sha256 + dim
  │
  ├── manifest_json TEXT             # 完整 manifest（导出用）
  ├── metrics_json TEXT
  └── created_at, training_job_id
```

> v1 完成时**立即**抽取并写入 `anchor_*_emb` BLOB + sha256，构成后续演进的基线。

---

## 3. 为什么以 PersonaModel 为中心

旧设计把"角色"与"专家"切成平行两条线。这有几个坏处：

- 强迫用户在创建第一个角色时就选择"它是普通人还是专家"
- 让"持续训练"看起来像是专家专属，而事实上所有角色都需要
- 知识不能后期接入，因为被建模成另一个独立对象

**AVCore 的选择**：把"形象、声音、人设、知识"作为**同层可装可拆**的能力维度，PersonaModel 是它们的容器。

- 同一个模型可以**先塑形，再灌知识**（v1 无知识、v2 加物理语料）
- 同一个模型可以**完全不灌知识**（虚拟主播、品牌代言人、Vlogger）
- 知识是可选维度，不是必须属性

---

## 4. 角色类型与最小输入

| 类型 | 视觉 | 声音 | 人设 | 知识 |
|------|------|------|------|------|
| 技术专家 | 必填 | 必填 | 必填 | 可选 |
| 虚拟主播 / 品牌代言人 | 必填 | 必填 | 必填 | 否 |
| 真人数字孪生 | 必填（本人授权） | 必填（本人授权） | 可推断 | 否 |
| 虚拟员工 | 可选（可纯口播） | 必填 | 必填 | 可选 |
| Vlogger / Storyteller | 必填 | 必填 | 必填 | 否 |

> "可选" = 不传也能跑（用商用音色 / 通用头像模板 / 无知识）。

---

## 5. 数据契约

```rust
// 顶层 persona model
struct PersonaModel {
    id: String,                       // pm_xxx
    name: String,
    archetype: String,                // mentor/vlogger/anchor/mascot/...
    description: String,
    current_version: u32,             // 默认指 v1
    version_ids: Vec<u32>,
    status: Status,                   // active / archived
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

// 版本（不可变快照）
struct PersonaVersion {
    id: (String, u32),                // (persona_model_id, version)
    parent_version: Option<u32>,
    avatar: Avatar,
    voice: Voice,
    persona: PersonaDescriptor,
    knowledge: Option<KnowledgeBinding>,
    identity_anchor: IdentityAnchor,
    metrics: VersionMetrics,
    status: VersionStatus,            // building/ready/deprecated
    training_job_id: Option<String>,
    dir_path: PathBuf,                // ~/.local/share/avc/personas/pm_xxx/v1/
    created_at: DateTime<Utc>,
}

struct Avatar {
    provider: String,                 // sdxl_ip_adapter / kling_avatar / ...
    primary_image: PathBuf,           // avatar/primary.png
    views: Vec<PathBuf>,              // avatar/views/*.png
    ref_images: Vec<PathBuf>,         // avatar/ref/*.png
    lora: Option<PathBuf>,            // avatar/lora/weights.safetensors
    face_id: Option<String>,
}

struct Voice {
    provider: String,
    voice_id: String,
    sample: PathBuf,                  // voice/sample.wav
    language: String,
    supported_emotions: Vec<String>,
    embed: PathBuf,                   // voice/embed.bin
}

struct PersonaDescriptor {
    traits: Vec<String>,
    tone: String,
    catchphrases: Vec<String>,
    taboos: Vec<String>,
    scenario_prompts: BTreeMap<String, String>,
    formality: f32,
    temperature: f32,
    response_length: String,
    language: String,
}

struct IdentityAnchor {
    face: EmbeddingRef,               // face/face_emb.bin
    voice: EmbeddingRef,
    style: EmbeddingRef,
    computed_at: DateTime<Utc>,
}
```

---

## 6. 创建流程

```
$ avc persona new "Yu" --from ./persona.toml
        │
        ▼
[1] 校验输入（consent、文件存在、维度齐全）
        │
        ▼
[2] 建立版本目录 personas/pm_xxx/v1/（空 + manifest 状态=building）
        │
        ▼
[3] 形象生成 ──▶ avatar/primary.png + views/ + face.json
        │          （可选）LoRA 训练，写入 lora/
        │
        ▼
[4] 声音生成 ──▶ voice/sample.wav + voice/embed.bin
        │
        ▼
[5] 人设抽取 ──▶ persona.json
        │
        ▼
[6] 知识索引（可选）──▶ knowledge/
        │
        ▼
[7] Identity Anchor 抽取 ──▶ identity_anchor.json
        │
        ▼
[8] 落 manifest.json，置 status=ready，写 SQLite
        │
        ▼
[9] 任一步骤失败 → 标记 status=failed，清中间产物（除非显式 --keep-partials）
```

**异步任务**：Provider 调用（尤其是 LoRA 训练 / 大模型生图）耗时较长，统一为 `task_xxx` 异步对象。`avc task show task_xxx --watch` 跟进。

---

## 7. 视觉子能力

### 7.1 文生图
- 调用 SDXL / Flux / HunyuanDiT，按 `description + style_tags` 渲染候选
- 多角度同 seed 出 4~8 视图
- 默认人脸一致性：InstantID / IP-Adapter / FaceID

### 7.2 LoRA 微调（可选）
- ≥ 5 张高质量参考图触发
- 调用 Provider 的 SFT/fine-tune 端点（提交样本 + base_avatar_ref），**远端训练**
- 返回**不下载权重**：仅在 `avatar/lora/ref.json` 写 `{ model_id, provider, trained_at, base_model, ... }`
- Provider 任务 ID 由长轮询推进；AVCore 端是异步包装

### 7.3 Provider 路由（全部 token 鉴权）

| Provider | 特点 |
|----------|------|
| `kling_avatar` | 商用稳定，token 调用 |
| `heygen_avatar` | 商用稳定，token 调用 |
| `doubao_image` | 字节豆包 image API，token 调用 |
| `seedream`（即梦） | 阿里即梦 image API，token 调用 |
| `replicate_flux_lora` | 商用平台访问开源 Flux LoRA，token 调用 |

切换 = 改 `provider.json` 字段，不改业务代码。

> 本框架**不支持自托管模型**（如 `sdxl_ip_adapter` / `cosyvoice` 等被设计为本地运行的不在 Provider 表中）。所有实现都是 HTTP API + Bearer token。

---

## 8. 声音子能力

### 8.1 声音克隆
- 样本要求：≥ 30s 干净人声、单说话人、无 BGM
- Provider：CosyVoice / GPT-SoVITS / F5-TTS
- 输出：`voice/sample.wav`（永久留底）+ `voice/embed.bin`（speaker embedding）

### 8.2 商用音色
- 不提供样本时，走火山 / 阿里 / 微软 TTS 商用 ID
- 仅写 `voice/sample.wav`（厂商试听音）+ `provider.json`

### 8.3 控制
- SSML：`<emotion>`、`<break>`、`<emphasis>`、语速
- 多情绪：同一 voice_id 切换情绪标签

---

## 9. 人设建模

输入自然语言 + 行为样本，输出 `persona.json`。  
内置模板：mentor（讲师）/ vlogger / anchor / mascot / instructor（教练）/ entertainer（主持）/ support（客服）/ storyteller。  
LLM 抽取 + 人工确认（CLI 上展示 diff 让用户接受）。

LLM 调用走 `llm.openai_compat` Provider（兼容 OpenAI / 豆包 / DeepSeek / 智谱等），不绑定模型厂商。

---

## 10. 知识接入（可选）

只有当角色真的"懂某个领域"才接入。详见 [`knowledge-aspect.md`](./knowledge-aspect.md)。

不接入 ≠ 不能讲内容——一个没有知识但有人设的 persona 也可以做"段子手"、"日常点评"、"情感陪伴"。

---

## 11. CLI 接口

```bash
avc persona new "Yu" \
  --description "数据库内核领域讲师，数据库内核专家" \
  --avatar-style 写实,教学 \
  --avatar-refs ./samples/ref_*.png \
  --voice-samples ./samples/voice_*.wav \
  --persona-traits 耐心,严谨,幽默 \
  --persona-catchphrase "我们直接看源码" \
  --from ./persona.toml         # 或显式参数

# 任务查询
avc task show task_01H... --watch
```

`samples.toml` 写法见 [`../cli.md §2.1`](../cli.md)。

---

## 12. 错误与边界

| 场景 | 处理 |
|------|------|
| 参考图模糊 / 多张脸 | 拒绝，返回 `invalid_ref_image` |
| 声音样本含 BGM / 多说话人 | 拒绝，返回 `invalid_audio_sample` |
| 未提供声音样本 | 走商用音色，自动匹配 |
| LoRA 训练失败 | 重试 1 次 → 退回非微调路径 |
| Provider 限速 | 切到备选 Provider |
| 任何子步骤失败 | 标记创建任务失败；非 `--keep-partials` 时清理中间产物；可重试整链路 |

---

## 13. 合规

- **形象授权**：上传参考图必须附 `consent_proof`（PDF 路径 + hash）
- **声音授权**：声音克隆必须上传"被克隆人授权书"
- **真实人物复刻**：默认禁用；`avc config set safety.real_person.enabled true` 才能开
- **不可见水印**：可选开启，写入 provider 产物阶段

---

## 14. 关键指标

- 端到端 P50 ≤ 90s（不含 LoRA，不含知识）
- 含 LoRA 训练 P95 ≤ 15 min
- 含知识索引 P95 ≤ 5 min
- 形象一致性 face embedding cosine ≥ 0.85（自检，与锚点同源）
- 声音相似度 speaker embedding cosine ≥ 0.80

---

## 15. 上下游

- **上游**：CLI 调用、集成方在 REPL 中手动创建
- **下游**：
  - [persona-evolution.md](./persona-evolution.md)：在 v1 基础上持续训练
  - [video-generation.md](./video-generation.md)：锁定某个版本出片
