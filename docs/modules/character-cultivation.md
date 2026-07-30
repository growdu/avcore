# 模块设计：角色养成（Character Cultivation）

> 让"演员"不仅长得像，还要"会说话、会反应、有性格"。

---

## 1. 模块目标

输入：人设描述（性格、语气、口头禅、禁忌、场景偏好）
输出：`Persona`，可绑定到 `Character`

边界：**不灌领域知识**（由 [expert-cultivation.md](./expert-cultivation.md) 负责）；
**不做视频生成**。

---

## 2. 数据契约

```python
@dataclass
class Persona:
    id: str
    character_id: str
    # 基础属性
    name: str                       # 角色自称
    archetype: str                  # archetype：mentor / entertainer ...
    traits: list[str]               # 性格词：耐心 / 幽默 / 严谨
    tone: str                       # 整体语气：温和 / 犀利 / 正式
    language: str = "zh"
    # 行为
    catchphrases: list[str]         # 口头禅
    taboos: list[str]               # 禁忌话题 / 措辞
    scenario_prompts: dict[str, str]  # 场景化 prompt：教学 / 营销 / 客服
    # 控制
    response_length: str = "medium" # short/medium/long
    formality: float = 0.5          # 0 极口语 ~ 1 极正式
    temperature: float = 0.7
    # 记忆（可选）
    memory_enabled: bool = False
    memory_store: str | None = None # 长期记忆存储标识
    meta: dict
```

---

## 3. 能力设计

### 3.1 人设建模
- 自然语言 → 结构化 Persona（LLM 抽取 + 人工确认）
- 提供模板：讲师 / 销售 / 客服 / 主持人 / 虚拟员工 ...

### 3.2 场景化 Prompt
针对不同任务类型，预设 Prompt 片段：

| 场景 | 关键指令 |
|------|----------|
| 教学 | "请用通俗语言、举例说明，避免使用未定义术语" |
| 营销 | "请突出产品价值、有节奏感、结尾给出明确 CTA" |
| 客服 | "请先共情，再给方案，避免使用绝对化承诺" |
| 直播 | "请用口语化、有互动感、避免长段落" |

### 3.3 风格化控制
- **语气**（formality）：0~1 滑杆
- **长度**（response_length）：short/medium/long
- **温度**（temperature）：影响创意性
- **风格词**：可注入风格参考（如"参考罗永浩的演讲风格"）

### 3.4 长期记忆（可选）
- `memory_enabled = true` 时启用
- 记忆载体：
  - **短期**：当前 Session 的多轮对话
  - **长期**：跨 Session 的关键事件（用户偏好、过往脚本）
- 存储：向量数据库 + 关系表
- 召回：每次 LLM 调用时召回 top-K 相关记忆注入

### 3.5 行为约束
- **禁忌词过滤**：生成内容后过滤 taboo 词
- **合规拦截**：敏感话题直接拒绝
- **风格校准**：自动检测输出与 persona 偏离度，超阈值重生成

---

## 4. 接口

```http
POST   /v1/personas                       创建人设
GET    /v1/personas/{id}                  查询
PUT    /v1/personas/{id}                  更新
DELETE /v1/personas/{id}                  删除

POST   /v1/personas/{id}/simulate         对话模拟（试运行）
POST   /v1/personas/{id}/chat             多轮对话（带记忆）

GET    /v1/personas/{id}/memories         长期记忆列表
DELETE /v1/personas/{id}/memories/{mid}   删除记忆
```

---

## 5. 与其他模块的协作

```
Persona ──▶ Script 生成时的 system prompt
         ──▶ TTS 时的语气 / 情绪 hint
         ──▶ 对话场景（与用户交互）
```

### 5.1 Persona + LLM 调用

```python
def build_system_prompt(persona: Persona, scenario: str) -> str:
    return f"""
你是一名{scenario}场景下的{persona.archetype}，名字叫 {persona.name}。
性格：{', '.join(persona.traits)}
语气：{persona.tone}
口头禅：{persona.catchphrases}
禁忌：{', '.join(persona.taboos)}
{scenario_prompts[scenario]}
"""
```

### 5.2 Persona + TTS

将 persona 的 tone / formality 翻译为 TTS 参数：
- `formality > 0.7` → 偏正式语速、稳定音色
- `formality < 0.3` → 偏口语、夸张情绪
- `tone = 犀利` → 加快语速、提高能量

---

## 6. 模板与示例

### 6.1 内置模板
- **企业讲师**：温和、严谨、善于举例
- **带货主播**：热情、有节奏、强 CTA
- **客服专员**：耐心、共情、解决问题
- **虚拟员工**：专业、克制、信息密度高
- **故事讲述者**：情绪化、有画面感

### 6.2 自定义
- 自由文本 + 结构化字段混编
- LLM 辅助填充缺失字段

---

## 7. 关键指标

- 风格一致性评分（LLM-as-Judge）≥ 0.80
- 禁忌词命中率 = 0
- 长期记忆召回准确率 ≥ 0.75
- Persona 试运行对话 P50 ≤ 2s
