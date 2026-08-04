#!/bin/sh
# kling-video.sh — CliVideoProvider 协议 mock 模板
#
# AVCore CliVideoProvider 调 vendor CLI 走三段式（见 src/provider/real.rs）：
#   1. submit — 提交任务，stdout 必须含 `task_id=...`（KV-flavor）
#      也容许 `data:{"task_id":"..."}` JSON。
#   2. status — 轮询，stdout 必须含 `status=done|pending|failed`
#   3. fetch  — 拉成品，--out <path> 写真 mp4 文件
#
# 用法（替换为你自家 Sora / Runway / Veo / kling-cli 真二进制后改 binary 路径）：
#   [provider.video.kling]
#   binary = "/path/to/your/kling-cli"
#
# 本模板展示 "占位 mp4" 行为：写一个 1KB 假 mp4（magic + random bytes），可让
# AVCore pipeline 跑通 e2e 测试。要接真 vendor 只需把每个 case 分支换成
# `curl https://api.klingai.com/v1/...` + 解析 JSON task_id 即可。
#
# 协议兼容性：与 AVCore 0.3.x 配套；KV-flavor + JSON-flavor 都吃。

set -eu

case "${1:-}" in
  submit)
    # 期望参数：--prompt @<script_path> --ref-image <avatar.png> --ref-audio <voice.wav>
    # 校验：所有 ref 必须存在；缺一个 → exit 2 显式失败（避免静默通过）
    missing=""
    shift
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --prompt)
          P="${2#@}"  # 去掉前导 @（vendor 协议约定）
          [ -f "$P" ] || missing="$missing --prompt($P)"
          shift 2;;
        --ref-image)
          [ -f "$2" ] || missing="$missing --ref-image($2)"
          shift 2;;
        --ref-audio)
          [ -f "$2" ] || missing="$missing --ref-audio($2)"
          shift 2;;
        *) shift;;
      esac
    done
    if [ -n "$missing" ]; then
      echo "kling-video submit: missing files:$missing" >&2
      exit 3
    fi
    # 真 vendor 替换：curl POST to kling API → 解析 task_id
    TASK_ID="kling-$(date +%s)-$$"
    echo "task_id=$TASK_ID"
    ;;

  status)
    # 期望参数：--task-id <id>
    # 真 vendor 替换：curl GET task status → 解析 status
    # 这里 mock 直接返 done（让 pipeline 跑完）
    echo "status=done"
    ;;

  fetch)
    # 期望参数：--task-id <id> --out <path>
    OUT_PATH=""
    shift
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --out) OUT_PATH="$2"; shift 2;;
        *) shift;;
      esac
    done
    if [ -z "$OUT_PATH" ]; then
      echo "kling-video fetch: --out <path> required" >&2
      exit 2
    fi
    mkdir -p "$(dirname "$OUT_PATH")"
    # 写占位 mp4：magic + 1024 bytes random → 1KB blob
    printf 'MOCK_KLING_MP4_ftyp' > "$OUT_PATH"
    head -c 1024 /dev/urandom >> "$OUT_PATH"
    ;;

  *)
    echo "kling-video: unknown subcommand '${1:-}' (expected submit|status|fetch)" >&2
    exit 2
    ;;
esac
