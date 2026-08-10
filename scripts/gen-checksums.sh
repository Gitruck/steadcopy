#!/usr/bin/env bash
# 为发布产物生成 SHA-256 清单。三处公示（Releases / 官网 / 应用内说明）以它为准。
set -euo pipefail
B="app/src-tauri/target/release/bundle/nsis"
OUT="release/SHA256SUMS.txt"
mkdir -p release
: > "$OUT"
for f in "$B"/*.exe; do
  [ -e "$f" ] || continue
  ( cd "$(dirname "$f")" && sha256sum "$(basename "$f")" ) >> "../../../../../$OUT" 2>/dev/null \
    || ( cd "$(dirname "$f")" && sha256sum "$(basename "$f")" ) >> "$OLDPWD/$OUT"
done
echo "已写出 $OUT："
cat "$OUT"
