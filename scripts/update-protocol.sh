#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="${HOME}/.deno/bin:${PATH}"
cd "$ROOT/protocol"
deno add jsr:@evex/linejs@latest jsr:@evex/linejs-types@latest
echo "Protocol packages updated. Rebuild UI with: cargo build --release"
