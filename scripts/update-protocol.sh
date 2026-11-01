#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# Review version refs and archive checksums in vendor-protocol.py first.
python3 "$ROOT/scripts/vendor-protocol.py" "$@"
"${DENO:-deno}" cache --config "$ROOT/protocol/deno.json" "$ROOT/protocol/src/main.ts"
echo "Pinned sources restored. Run protocol checks and rebuild the sidecar."
