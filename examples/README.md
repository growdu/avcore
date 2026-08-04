# AVCore Examples

可复制可改的 vendor CLI 模板 + 完整 `avc.toml` 配置示例。所有脚本与 AVCore 0.3.x 协议兼容。

## vendor-cli/ — 4 个 mock shell 模板

| 脚本 | 配在哪 | 协议 | 接什么 |
|------|--------|------|--------|
| [`kling-video.sh`](vendor-cli/kling-video.sh) | `[provider.video.<name>].binary` | `submit / status / fetch` 三段式 | kling / Sora / Runway / Veo 等视频生成 |
| [`kling-avatar-fin.sh`](vendor-cli/kling-avatar-fin.sh) | `[provider.avatar.<name>].binary` | `finetune submit / status / fetch` 三段式 | kling 头像 SFT / 任何 face-fusion 端点 |
| [`elevenlabs-voice-fin.sh`](vendor-cli/elevenlabs-voice-fin.sh) | `[provider.voice.<name>].binary` | `finetune submit / status / fetch` 三段式 | ElevenLabs Voice Clone / doubao-voice-clone 等 |
| [`aws-s3-cp.sh`](vendor-cli/aws-s3-cp.sh) | `[export.s3].upload_cmd` 模板 | `<local> <bucket> <prefix> <name>` 占位符替换 | 任何 S3 兼容对象存储（aws-cli / mc / rclone 都行） |

### 共同约定

- **stdout 协议**：用 KV-flavor `key=value`（最易调试），AVCore 也吃 `data:{"key":"val"}` JSON 风格。
- **退出码**：0 = 成功；非 0 = 失败，AVCore 转译为 `ProviderUpstream` (exit 11) 或 `ProviderTimeout` (exit 12)。
- **真 vendor 替换**：每个 case 分支里加 `curl ...` 调真 API + `jq` 解析 JSON task_id / status。

### 30 秒 e2e 试用

```bash
# 0. 准备数据
mkdir -p /tmp/avc-demo/{data,config}
XDG_DATA_HOME=/tmp/avc-demo/data XDG_CONFIG_HOME=/tmp/avc-demo/config \
  avc init
XDG_DATA_HOME=/tmp/avc-demo/data XDG_CONFIG_HOME=/tmp/avc-demo/config \
  avc persona create --name yu

# 1. 用 mock 模板（无 token / 无外网也能跑）
cp examples/avc.toml.template /tmp/avc-demo/config/avc/avc.toml
# 把 video / avatar / voice / export 段的 binary 路径都改成
# /path/to/avc/examples/vendor-cli/<script>.sh

# 2. render run（5 节点 DAG 走 mock video CLI）
XDG_DATA_HOME=/tmp/avc-demo/data XDG_CONFIG_HOME=/tmp/avc-demo/config \
  avc render run --persona yu --version 1 --topic "demo" --video-provider kling

# 3. finetune start（建 fj job）
XDG_DATA_HOME=/tmp/avc-demo/data XDG_CONFIG_HOME=/tmp/avc-demo/config \
  avc finetune start yu --base-version 1 --scope voice

# 4. 写一条 audio sample 到 DB（用 avc sample add 缺，DB 写入）
# ...（见 tests/integration.rs:finetune_run_via_vendor_cli_writes_target_and_publishes）

# 5. job export 到 "S3"（mock mirror 到 /tmp/s3-mirror/）
mkdir -p /tmp/s3-mirror
XDG_DATA_HOME=/tmp/avc-demo/data XDG_CONFIG_HOME=/tmp/avc-demo/config \
  avc job export <job_id> --target s3://my-bucket/videos/2026/
ls /tmp/s3-mirror/my-bucket/videos/2026/   # 应见 5 个 .bin artifact
```

### 接真 vendor

每个 mock 脚本的 case 分支都有清晰的 `# 真 vendor 替换` 注释行。最常见替换：

```sh
# kling-video.sh submit: 调 kling API
TASK_ID=$(curl -fsS -X POST https://api.klingai.com/v1/videos/text2video \
  -H "Authorization: Bearer $KLING_API_KEY" \
  -F "prompt=<$PROMPT_FILE" \
  | jq -r '.data.task_id')
echo "task_id=$TASK_ID"

# elevenlabs-voice-fin.sh submit: 调 ElevenLabs clone
VOICE_ID=$(curl -fsS -X POST https://api.elevenlabs.io/v1/voices/add \
  -H "xi-api-key: $ELEVENLABS_API_KEY" \
  -F "files[]=@$REF1" \
  | jq -r '.voice_id')
echo "task_id=$VOICE_ID"   # AVCore 不区分 vendor，统称 task_id
```

## avc.toml.template — 完整配置示例

[`avc.toml.template`](avc.toml.template) 覆盖所有段：

- `[provider.llm.<name>]` — OpenAI 兼容 chat（ask / shell NL / finetune drift seed text）
- `[provider.embed.<name>]` — OpenAI 兼容 embeddings（finetune drift eval）
- `[provider.avatar.<name>]` — image gen + 可选 vendor SFT
- `[provider.voice.<name>]` — tts + 可选 vendor SFT/clone
- `[provider.video.<name>]` — 三段式 video CLI
- `[shell]` / `[safety]` — Shell NL + 安全开关
- `[export.s3]` — upload_cmd 模板

拷贝到 `~/.config/avc/avc.toml` 改 `api_key` 和 `binary` 即可。

## 协议参考

详见：
- `docs/cli.md` §4 — CLI 命令表
- `docs/api/README.md` — Provider trait / config schema
- `src/provider/real.rs::run_finetune_vendor_pipeline` — vendor SFT 三段式实现
- `src/svc/render.rs::export_artifacts` — S3 export 实现
