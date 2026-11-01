#!/usr/bin/env bash
set -euo pipefail

archive="${1:?usage: check-package.sh ARCHIVE.tar.gz-or-pkg.tar.zst}"
listing="$(tar -tf "$archive")"

for required in \
  /line-gtk \
  /protocol/line-gtk-protocol \
  /assets/lang/vie.json \
  /docs/fork-shirafukayayoi.md \
  /docs/protocol-reliability.md \
  /assets/lang/eng.json \
  /assets/lang/thai.json \
  /assets/lang/ja.json \
  /dev.linegtk.LineGtk.desktop
do
  if ! grep -q "${required}$" <<<"$listing"; then
    echo "missing package entry: $required" >&2
    exit 1
  fi
done

project_root="$(cd "$(dirname "$0")/.." && pwd)"
temporary="$(mktemp -d -t line-gtk-package.XXXXXXXX)"
trap 'rm -rf "$temporary"' EXIT
tar --no-same-owner -xf "$archive" -C "$temporary"
mapfile -t sidecars < <(find "$temporary" -type f -name line-gtk-protocol)
if [[ "${#sidecars[@]}" != 1 ]]; then
  echo "expected exactly one compiled sidecar in package" >&2
  exit 1
fi
python3 "$project_root/scripts/check-sidecar.py" "${sidecars[0]}"

echo "package smoke test passed: $archive"
