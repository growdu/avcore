# API 概览

> 完整 API 规范以 OpenAPI 文档为准（`openapi.yaml` 待生成）。本文给出**门控、典型用法、版本与回滚的关键约定**。

---

## 1. 协议与基础

- 协议：HTTP/1.1 + TLS 1.2+
- 数据格式：JSON（UTF-8）
- 鉴权：`Authorization: Bearer {api_key}` 或 HMAC 签名
- 基础路径：`https://{host}/v1`
- 限流：`X-RateLimit-Limit / Remaining / Reset` 响应头
- Trace：`X-Request-Id` 请求头（透传 trace_id）

---

## 2. 端点一览

### Persona Model（顶层）
```
POST   /persona-models                               创建 PersonaModel（异步）
GET    /persona-models/{id}                          查询
GET    /persona-models/{id}/versions                 历史版本
GET    /persona-models/{id}/versions/{vid}          指定版本快照
PUT    /persona-models/{id}/current-version          设置默认版本（用于新任务）
POST   /persona-models/{id}/ab                       开启 A/B 流量分配
DELETE /persona-models/{id}                          归档
```

### 视觉形象（每个版本是不可变快照）
```
POST   /persona-models/{id}/avatars                  创建 / 替换形象（异步）
POST   /persona-models/{id}/versions/{vid}/avatar    在指定版本上记录视觉快照
GET    /avatars/{aid}                                查询
DELETE /avatars/{aid}                                删除（仅可删未绑定版本）
```

### 声音
```
POST   /persona-models/{id}/voices                   创建 / 替换声音（异步）
POST   /voices/{vid}/synthesize                      TTS 试听
```

### 人设
```
POST   /persona-models/{id}/persona                  设置/更新人设
POST   /persona-models/{id}/versions/{vid}/persona   在版本上固化人设
GET    /personas/{pid}                               查询
POST   /personas/{pid}/simulate                      试运行对话
```

### 知识能力（可选）
```
POST   /corpora                                      创建语料
POST   /corpora/{id}/chunks                          追加 chunks
POST   /corpora/{id}/reindex                         重建索引
POST   /corpora/{id}/search                          检索（调试 / 联调用）
POST   /persona-models/{id}/knowledge                绑定 / 替换知识
DELETE /persona-models/{id}/knowledge                解绑
POST   /persona-models/{id}/knowledge/ask            试运行问答
```

### 持续训练与样本
```
POST   /persona-models/{id}/samples                  提交样本
GET    /persona-models/{id}/samples                  列样本
DELETE /persona-samples/{sid}                        移除
POST   /persona-samples/{sid}/consign                标金丝雀样本

POST   /persona-models/{id}/training-jobs            创建训练任务（异步）
GET    /training-jobs/{jid}                          查询
POST   /training-jobs/{jid}/cancel                   取消
POST   /training-jobs/{jid}/resume                   续跑
GET    /training-jobs/{jid}/report                   训练报告（含一致性 / 漂移）

POST   /persona-models/{id}/versions/{vid}/deprecated 停用版本
```

### 视频生成与脚本
```
POST   /scripts                                       生成分镜（绑定 persona_version_id）
PUT    /scripts/{id}                                  编辑
GET    /scripts/{id}                                  查询
POST   /scripts/{id}/preview-narration                仅生成旁白音频预览

POST   /jobs                                          创建渲染任务
GET    /jobs/{id}                                     查询
GET    /jobs/{id}/steps                               步骤进度
GET    /jobs/{id}/artifacts                           产物列表
POST   /jobs/{id}/cancel                              取消
POST   /jobs/{id}/retry                               重试
POST   /jobs/{id}/rerender-scene                      重渲染某 Scene
POST   /jobs/{jid}/feedback                           用户反馈（→ 回灌样本）
```

### 通知
```
POST   /webhooks                                      注册回调
DELETE /webhooks/{id}                                 注销
WS     /ws/jobs?job_id=...                            实时进度
```

### 管理
```
GET    /admin/providers                                列出 Provider
PUT    /admin/providers/{name}                        调整配置
GET    /admin/quotas                                  查询租户配额
PUT    /admin/quotas/{tenant_id}                      调整
```

---

## 3. 典型调用顺序

### 3.1 首次接入（一次性创建 PersonaModel）

```bash
# 1. 创建顶层 PersonaModel（异步任务，返回 task_id）
curl -X POST $BASE/persona-models -d '{
  "name": "Lily",
  "description": "30 岁东亚女性，温和笑容，教学型主播"
}'

# 2. 给模型添加形象
curl -X POST $BASE/persona-models/$PMID/avatars -d '{
  "description": "30 岁东亚女性，短发，温和笑容",
  "style_tags": ["写实", "教学"],
  "ref_images": ["..."]
}'

# 3. 添加声音
curl -X POST $BASE/persona-models/$PMID/voices -d '{
  "samples": [{"uri": "oss://.../sample.wav", "duration_ms": 45000, "text": "..."}],
  "language": "zh"
}'

# 4. 设置人设
curl -X POST $BASE/persona-models/$PMID/persona -d '{
  "traits": ["耐心","严谨","幽默"],
  "tone": "温和",
  "catchphrases": ["来，我们一步步看"],
  "taboos": ["绝对化表述"],
  "scenario_prompts": {"teach": "请用通俗语言讲解..."}
}'

# 5. （可选）若该角色是"领域专家"，绑定语料
curl -X POST $BASE/corpora -d '{"name": "高中物理"}'
curl -X POST $BASE/corpora/$CRID/chunks -d @chunks.json
curl -X POST $BASE/persona-models/$PMID/knowledge -d '{
  "corpus_ids": ["'$CRID'"],
  "domain": "高中物理",
  "grounding_mode": "loose"
}'
```

完成后会得到 v1，可用 `GET /persona-models/{id}/versions` 拿到 version_id。

### 3.2 持续训练（追加样本 + 出新版本）

```bash
# 1. 追加声音样本
curl -X POST $BASE/persona-models/$PMID/samples -d '{
  "kind": "audio",
  "uri": "oss://.../new-sample.wav",
  "duration_ms": 60000,
  "text": "...",
  "language": "zh",
  "consent_proof": "auth_xxx"
}'

# 2. 启动训练
curl -X POST $BASE/persona-models/$PMID/training-jobs -d '{
  "base_version_id": "pmod_'$PMID'_v1",
  "scope": ["voice"],
  "config": {
    "full_retrain": false,
    "epochs": 3,
    "consistency_threshold": 0.85,
    "fallback_to_base": true
  }
}'

# 3. 查训练状态
curl $BASE/training-jobs/$JID

# 4. 训练通过后切默认版本
curl -X PUT $BASE/persona-models/$PMID/current-version -d '{ "version_id": "v2" }'
```

### 3.3 生产视频（每次任务）

```bash
# 1. 生成分镜（绑定版本）
curl -X POST $BASE/scripts -d '{
  "persona_model_id": "'$PMID'",
  "persona_version_id": "v2",
  "topic": "牛顿第一定律",
  "key_points": ["定义", "示例", "应用"],
  "target_duration": 60
}'

# 2. （可选）编辑脚本
curl -X PUT $BASE/scripts/$SID -d @script_edited.json

# 3. 创建任务
curl -X POST $BASE/jobs -d '{
  "script_id": "'$SID'",
  "options": {
    "resolution": "1080p",
    "enable_subtitle": true,
    "webhook_url": "https://example.com/cb"
  }
}'

# 4. 轮询 / 接收 Webhook
curl $BASE/jobs/$JID
```

---

## 4. 关于"版本"的强约束

- **生成必须锁定版本**：脚本 / 任务 上都带有 `persona_version_id`。后续 persona 升级到 v5 不会影响已渲染视频。
- **切版本不影响历史**：调 `PUT /persona-models/{id}/current-version` 只影响"之后新任务默认用哪个版本"。
- **回滚等价于"指针回拨"**：不删任何 version，只改 current。

---

## 5. 错误约定

```json
{
  "error": {
    "code": "invalid_input",
    "message": "ref_image 模糊",
    "field": "ref_images[0]",
    "trace_id": "..."
  }
}
```

| HTTP | code | 含义 |
|------|------|------|
| 400 | `invalid_input` | 参数错误 |
| 401 | `unauthorized` | 鉴权失败 |
| 403 | `forbidden` | 权限不足 / 授权缺失 |
| 404 | `not_found` | 资源不存在 |
| 409 | `conflict` | 状态冲突（如重复创建） |
| 422 | `provider_error` | 上游模型错误 |
| 422 | `drift_detected` | 训练后一致性不达标，已回退 |
| 429 | `rate_limited` | 限流 |
| 500 | `internal` | 内部错误 |
| 503 | `unavailable` | 资源耗尽 / 维护中 |

---

## 6. Webhook 协议

```http
POST {webhook_url}
Content-Type: application/json
X-Avcore-Signature: sha256={hmac}
X-Avcore-Event: job.succeeded

{
  "event": "job.succeeded",
  "job_id": "...",
  "tenant_id": "...",
  "persona_model_id": "...",
  "persona_version_id": "pmod_xxx_v2",
  "artifacts": {
    "video_url": "...",
    "cover_url": "...",
    "subtitle_url": "...",
    "duration_ms": 60000
  },
  "timestamp": 1730000000
}
```

签名：`HMAC-SHA256(secret, body)`，用于验签。

训练任务亦发事件：`training.succeeded` / `training.failed_drift`。

---

## 7. WebSocket 协议

```text
WS /v1/ws/jobs?job_id=xxx
↓
{"type":"progress","progress":0.45,"current_step":"i2v"}
{"type":"step.succeeded","step":"tts"}
{"type":"job.succeeded","artifacts":{...}}
```

训练任务同样支持：`WS /v1/ws/training-jobs?job_id=xxx`，事件包括 `sample_filter.done` / `drift_eval.done` / `version.published`。

心跳：30s 一次 ping/pong。
