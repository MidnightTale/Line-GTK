#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "$0")/.." && pwd)"
deno_bin="${DENO:-deno}"
output="${1:-$project_root/protocol/line-gtk-protocol}"

"$deno_bin" compile \
  --no-prompt \
  --allow-read \
  --allow-write \
  --allow-net \
  --allow-run=ffmpeg,ffprobe \
  --allow-sys \
  --allow-env=HOME,PATH,DENO_DIR,Q_DEBUG,NODE_DEBUG,NO_COLOR,LINE_GTK_DATA,LINE_GTK_LANG,LINE_GTK_CACHE_RETENTION,LINE_GTK_AUDIO_INPUT,LINE_GTK_AUDIO_OUTPUT,LINE_DEVICE,LINE_VERSION,LINE_CALL_DEVNAME,LINE_CALL_DEVICE_INFO,LINE_CALL_OPUS_SIGNAL,LINE_CALL_DEBUG \
  --output "$output" \
  "$project_root/protocol/src/main.ts"

chmod 0755 "$output"
