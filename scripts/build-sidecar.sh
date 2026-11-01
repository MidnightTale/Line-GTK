#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "$0")/.." && pwd)"
deno_bin="${DENO:-deno}"
output="${1:-$project_root/protocol/line-gtk-protocol}"
python3 "$project_root/scripts/vendor-protocol.py" --ensure
offline=()
if [[ "${DENO_OFFLINE:-0}" == 1 ]]; then
  offline=(--cached-only)
fi

"$deno_bin" compile \
  --config "$project_root/protocol/deno.json" \
  --lock "$project_root/protocol/deno.lock" \
  --frozen-lockfile \
  "${offline[@]}" \
  --no-prompt \
  --allow-read \
  --allow-write \
  --allow-net \
  --allow-run=ffmpeg,ffprobe \
  --allow-sys \
  --allow-env=HOME,PATH,DENO_DIR,Q_DEBUG,NODE_DEBUG,NO_COLOR,LINE_GTK_DATA,LINE_GTK_SKIP_RESTORE,LINE_GTK_LANG,LINE_GTK_CACHE_RETENTION,LINE_GTK_CACHE_KEY,LINE_GTK_AUDIO_INPUT,LINE_GTK_AUDIO_OUTPUT,LINE_DEVICE,LINE_VERSION,LINE_CALL_DEVNAME,LINE_CALL_DEVICE_INFO,LINE_CALL_OPUS_SIGNAL,LINE_CALL_DEBUG \
  --output "$output" \
  "$project_root/protocol/src/main.ts"

chmod 0755 "$output"
python3 "$project_root/scripts/check-sidecar.py" "$output"
