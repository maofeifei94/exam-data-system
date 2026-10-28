#!/bin/bash
# 从项目本地 .ssh/ 恢复 ~/.ssh 密钥
# （沙箱每次启动会清空 ~/.ssh，本项目 .ssh/ 已 gitignore 不会提交）
set -e

PROJ_DIR="$(cd "$(dirname "$0")" && pwd)"
SRC="$PROJ_DIR/.ssh"

if [ ! -d "$SRC" ]; then
  echo "错误：项目 .ssh/ 目录不存在，请确认密钥已保存在 $SRC" >&2
  exit 1
fi

mkdir -p ~/.ssh && chmod 700 ~/.ssh
cp "$SRC/"* ~/.ssh/
chmod 600 ~/.ssh/id_ed25519 ~/.ssh/config
chmod 644 ~/.ssh/id_ed25519.pub
echo "✓ SSH 密钥已从 $SRC 恢复到 ~/.ssh"
ls -la ~/.ssh/
