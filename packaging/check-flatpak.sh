#!/usr/bin/env bash
set -euo pipefail

bundle="${1:?usage: check-flatpak.sh BUNDLE.flatpak}"
check_dir="$(mktemp -d /tmp/line-gtk-bundle-check.XXXXXX)"
trap 'rm -rf "$check_dir"' EXIT

ostree init --repo="$check_dir/repo" --mode=archive
flatpak build-import-bundle "$check_dir/repo" "$bundle" >/dev/null
ref="app/dev.linegtk.LineGtk/$(uname -m)/stable"
ostree --repo="$check_dir/repo" checkout --user-mode "$ref" "$check_dir/checkout"
metadata="$(<"$check_dir/checkout/metadata")"

grep -q '^name=dev.linegtk.LineGtk$' <<<"$metadata"
grep -q '^runtime=org.gnome.Platform/' <<<"$metadata"
grep -q '^command=line-gtk$' <<<"$metadata"
test -x "$check_dir/checkout/files/bin/line-gtk"
test -x "$check_dir/checkout/files/share/line-gtk/protocol/line-gtk-protocol"

echo "flatpak smoke test passed: $bundle"
