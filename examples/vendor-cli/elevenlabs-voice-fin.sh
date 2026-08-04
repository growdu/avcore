#!/bin/sh
# elevenlabs-voice-fin.sh — Voice vendor SFT 协议 mock 模板
#
# AVCore OpenAiCompatVoiceProvider::finetune 在 `cfg.binary` 设了的情况下
# 调 `binary finetune submit / status / fetch` 三段式（v0.3+）：
#   1. finetune submit --ref-audio <wav1> [<wav2> ...] → stdout task_id=...
#   2. finetune status --task-id <id> → stdout status=done|pending|failed
#   3. finetune fetch --task-id <id> --out <wav_path> → 写真 WAV
#
# 用法（替换成真 ElevenLabs Voice Clone / doubao-voice-clone 端点）：
#   [provider.voice.elevenlabs]
#   binary = "/path/to/your/elevenlabs-cli"
#   api_key = "..."  # 不需要，binary 内部用
#
# 本模板给占位 WAV（RIFF header + 512 bytes random PCM）让 pipeline 跑通 e2e。
# 真 vendor 接 ElevenLabs /voices/add + /voices/{id}/stream 即可。

set -eu

case "${1:-}" in
  finetune)
    case "${2:-}" in
      submit)
        # 期望：--ref-audio <path1> [<path2> ...]
        if [ "${3:-}" != "--ref-audio" ]; then
          echo "elevenlabs-voice-fin: expected --ref-audio, got '${3:-}'" >&2
          exit 2
        fi
        if [ "$#" -lt 4 ]; then
          echo "elevenlabs-voice-fin: --ref-audio requires at least 1 path" >&2
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
          echo "elevenlabs-voice-fin: ref files missing:$missing" >&2
          exit 3
        fi
        # 真 vendor 替换：curl POST multipart to ElevenLabs /v1/voices/add → 解析 voice_id
        TASK_ID="elevenlabs-$(date +%s)-$$"
        echo "task_id=$TASK_ID"
        ;;

      status)
        # 真 vendor 替换：curl GET /v1/voices/{id} → 解析 status
        # mock 直接 done
        echo "status=done"
        ;;

      fetch)
        # 期望：--task-id <id> --out <wav_path>
        OUT_PATH=""
        while [ "$#" -gt 0 ]; do
          case "$1" in
            --out) OUT_PATH="$2"; shift 2;;
            *) shift;;
          esac
        done
        if [ -z "$OUT_PATH" ]; then
          echo "elevenlabs-voice-fin fetch: --out <path> required" >&2
          exit 2
        fi
        mkdir -p "$(dirname "$OUT_PATH")"
        # 写占位 WAV：RIFF header + 512 bytes random（fake PCM payload）
        {
          printf 'RIFF'
          # 4 bytes file size - 8 (little-endian; 512 + 36 = 548 = 0x224)
          printf '\x24\x02\x00\x00'
          printf 'WAVEfmt '
          # fmt chunk: 16 bytes, PCM = 1, channels = 1, rate = 16000
          printf '\x10\x00\x00\x00\x01\x00\x01\x00\x80\x3e\x00\x00\x80\x3e\x00\x00\x02\x00\x10\x00'
          printf 'data'
          # 4 bytes data size = 512
          printf '\x00\x02\x00\x00'
          head -c 512 /dev/urandom
        } > "$OUT_PATH"
        ;;

      *)
        echo "elevenlabs-voice-fin: unknown finetune subcommand '$2'" >&2
        exit 2
        ;;
    esac
    ;;

  *)
    echo "elevenlabs-voice-fin: unknown subcommand '$1' (expected finetune)" >&2
    exit 2
    ;;
esac
