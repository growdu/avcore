# API 概览

> 完整 API 规范以 OpenAPI 文档为准（`openapi.yaml` 待生成）。本文件仅给出门控与典型用法。

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

### 角色与资产
```
POST   /characters
GET    /characters/{id}
PUT    /characters/{id}
DELETE /characters/{id}
POST   /characters/{id}/avatar
POST   /characters/{id}/voice
GET    /characters/{id}/preview
```

### 知识与专家
```
POST   /corpora
GET    /corpora/{id}
POST   /corpora/{id}/chunks
POST   /corpora/{id}/reindex
POST   /corpora/{id}/search
POST   /experts
POST   /experts/{id}/ask
```

### 脚本
```
POST   /scripts                    # 生成分镜
GET    /scripts/{id}
PUT    /scripts/{id}               # 编辑
POST   /scripts/{id}/preview-narration  # 仅生成旁白音频预览
```

### 任务
```
POST   /jobs
GET    /jobs/{id}
GET    /jobs/{id}/steps
GET    /jobs/{id}/artifacts
POST   /jobs/{id}/cancel
POST   /jobs/{id}/retry
POST   /jobs/{id}/rerender-scene
```

### 通知
```
POST   /webhooks
DELETE /webhooks/{id}
WS     /ws/jobs?job_id=...
```

### 管理
```
GET    /admin/providers
PUT    /admin/providers/{name}
GET    /admin/quotas
PUT    /admin/quotas/{tenant_id}
```

---

## 3. 典型调用顺序

### 3.1 首次接入（一次性创建角色）

```bash
# 1. 创建角色
curl -X POST $BASE/characters -d '{
  "name": "Lily",
  "persona": "温和、严谨的物理讲师"
}'

# 2. 创建形象（异步任务，返回 task_id）
curl -X POST $BASE/characters/$CID/avatar -d '{
  "description": "30 岁东亚女性，短发，温和笑容",
  "style_tags": ["写实", "教学"],
  "ref_images": ["..."]
}'

# 3. 创建声音
curl -X POST $BASE/characters/$CID/voice -d '{
  "samples": [{"uri": "oss://.../sample.wav", "duration_ms": 45000, "text": "..."}],
  "language": "zh"
}'

# 4. 创建专家
curl -X POST $BASE/corpora -d '{"name": "高中物理"}'
curl -X POST $BASE/corpora/$CRID/chunks -d @chunks.json
curl -X POST $BASE/experts -d '{
  "character_id": "...",
  "domain": "高中物理",
  "corpus_ids": ["..."]
}'
```

### 3.2 生产视频（每次任务）

```bash
# 1. 生成分镜
curl -X POST $BASE/scripts -d '{
  "character_id": "...",
  "topic": "牛顿第一定律",
  "key_points": ["定义", "示例", "应用"],
  "target_duration": 60
}'

# 2. （可选）编辑脚本
curl -X PUT $BASE/scripts/$SID -d @script_edited.json

# 3. 创建任务
curl -X POST $BASE/jobs -d '{
  "script_id": "...",
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

## 4. 错误约定

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
| 429 | `rate_limited` | 限流 |
| 500 | `internal` | 内部错误 |
| 503 | `unavailable` | 资源耗尽 / 维护中 |

---

## 5. Webhook 协议

```http
POST {webhook_url}
Content-Type: application/json
X-Avcore-Signature: sha256={hmac}
X-Avcore-Event: job.succeeded

{
  "event": "job.succeeded",
  "job_id": "...",
  "tenant_id": "...",
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

---

## 6. WebSocket 协议

```text
WS /v1/ws/jobs?job_id=xxx
↓
{"type":"progress","progress":0.45,"current_step":"i2v"}
{"type":"step.succeeded","step":"tts"}
{"type":"job.succeeded","artifacts":{...}}
```

心跳：30s 一次 ping/pong。
