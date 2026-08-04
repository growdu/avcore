#!/bin/sh
# kling-avatar-fin.sh — Avatar vendor SFT 协议 mock 模板
#
# AVCore OpenAiCompatAvatarProvider::finetune 在 `cfg.binary` 设了的情况下
# 调 `binary finetune submit / status / fetch` 三段式（v0.3+）：
#   1. finetune submit --ref-image <path1> [<path2> ...] → stdout task_id=...
#   2. finetune status --task-id <id> → stdout status=done|pending|failed
#   3. finetune fetch --task-id <id> --out <png_path> → 写真 PNG
#
# 用法（替换成真 kling 头像 SFT 端点）：
#   [provider.avatar.kling]
#   binary = "/path/to/your/kling-cli"
#
# 本模板给占位 PNG（PNG magic + 256 bytes random）让 pipeline 跑通 e2e。
# 真 vendor 接 kling face-fusion API 即可。

set -eu

case "${1:-}" in
  finetune)
    case "${2:-}" in
      submit)
        # 期望：--ref-image <path1> [<path2> ...]
        # 校验：第 3 token 必须是 --ref-image；至少 1 个 ref 路径
        if [ "${3:-}" != "--ref-image" ]; then
          echo "kling-avatar-fin: expected --ref-image, got '${3:-}'" >&2
          exit 2
        fi
        if [ "$#" -lt 4 ]; then
          echo "kling-avatar-fin: --ref-image requires at least 1 path" >&2
          exit 2
        fi
        # 校验所有 ref 路径存在
        i=4
        missing=""
        while [ "$i" -le "$#" ]; do
          eval "p=\$$i"
          [ -f "$p" ] || missing="$missing $p"
          i=$((i + 1))
        done
        if [ -n "$missing" ]; then
          echo "kling-avatar-fin: ref files missing:$missing" >&2
          exit 3
        fi
        # 真 vendor 替换：curl POST multipart to kling face-fusion API → 解析 task_id
        TASK_ID="kling-avatar-$(date +%s)-$$"
        echo "task_id=$TASK_ID"
        ;;

      status)
        # 期望：--task-id <id>
        # mock：直接 done；真 vendor 替换为 curl GET status
        echo "status=done"
        ;;

      fetch)
        # 期望：--task-id <id> --out <png_path>
        OUT_PATH=""
        while [ "$#" -gt 0 ]; do
          case "$1" in
            --out) OUT_PATH="$2"; shift 2;;
            *) shift;;
          esac
        done
        if [ -z "$OUT_PATH" ]; then
          echo "kling-avatar-fin fetch: --out <path> required" >&2
          exit 2
        fi
        mkdir -p "$(dirname "$OUT_PATH")"
        # 写占位 PNG：magic + 256 bytes random
        printf '\x89PNG\r\n\x1a\n' > "$OUT_PATH"
        head -c 256 /dev/urandom >> "$OUT_PATH"
        ;;

      *)
        echo "kling-avatar-fin: unknown finetune subcommand '$2'" >&2
        exit 2
        ;;
    esac
    ;;

  *)
    echo "kling-avatar-fin: unknown subcommand '$1' (expected finetune)" >&2
    exit 2
    ;;
esac
