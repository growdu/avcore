#!/bin/sh
# aws-s3-cp.sh — [export.s3].upload_cmd mock 模板
#
# AVCore svc::render::export_artifacts 调 S3 target 时会 spawn
# `sh -c <upload_cmd>`，每条 artifact 替换占位符：
#   {local}  {bucket}  {prefix}  {name}
# 跑完后删 tmp file。
#
# 默认 upload_cmd 模板（来自 [export.s3]）：
#   aws s3 cp --region us-east-1 {local} s3://{bucket}/{prefix}{name}
#
# 本 mock 替代真 aws-cli（要求 ~/.aws/credentials 配 access_key / secret_key）：
# - 写本地 mirror /tmp/s3-mirror/<bucket>/<prefix><name> 模拟 S3
# - 记 log 到 /tmp/s3-upload.log
# - 不真发网络（CI/无 key 也能跑）
#
# 用法（替换为真 aws-cli 或 mc / rclone）：
#   [export.s3]
#   upload_cmd = "/path/to/your/aws s3 cp --region us-east-1 {local} s3://{bucket}/{prefix}{name}"
#
# 或换工具：
#   [export.s3]
#   upload_cmd = "mc cp {local} minio/{bucket}/{prefix}{name}"
#   upload_cmd = "rclone copyto {local} s3remote:{bucket}/{prefix}{name}"

set -eu

LOCAL="${1:-}"
BUCKET="${2:-}"
PREFIX="${3:-}"
NAME="${4:-}"

if [ -z "$LOCAL" ] || [ -z "$BUCKET" ] || [ -z "$NAME" ]; then
  echo "aws-s3-cp: usage: $0 <local> <bucket> <prefix> <name>" >&2
  echo "  (called by AVCore [export.s3].upload_cmd with substituted placeholders)" >&2
  exit 2
fi

if [ ! -f "$LOCAL" ]; then
  echo "aws-s3-cp: local file not found: $LOCAL" >&2
  exit 3
fi

# 替换为真 aws-cli：
#   AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY 来自 env 或 ~/.aws/credentials
#   aws s3 cp --region "${AWS_REGION:-us-east-1}" "$LOCAL" "s3://$BUCKET/${PREFIX}${NAME}"
#
# 这里写本地 mirror 让 mock 可观察：
MIRROR_ROOT="${S3_MIRROR_ROOT:-/tmp/s3-mirror}"
LOG_FILE="${S3_UPLOAD_LOG:-/tmp/s3-upload.log}"

DEST_DIR="$MIRROR_ROOT/$BUCKET/$PREFIX"
mkdir -p "$DEST_DIR"
cp "$LOCAL" "$DEST_DIR$NAME"

# 记 log（用户可看 S3 镜像在 /tmp/s3-mirror/）
printf '%s\t%s\t%s\t%s\t%s\n' \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  "$BUCKET" "${PREFIX}${NAME}" \
  "$(wc -c < "$LOCAL")" \
  "$(sha256sum "$LOCAL" | awk '{print $1}')" \
  >> "$LOG_FILE"

# 模拟 aws s3 cp 的 stdout 行为
echo "upload: ./$LOCAL to s3://$BUCKET/${PREFIX}${NAME}"
