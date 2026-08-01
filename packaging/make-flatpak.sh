#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VER="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
ARCH="$(uname -m)"
STAGE="/tmp/line-gtk-${VER}-x86_64"
OUT="/tmp/line-gtk-${VER}-${ARCH}.flatpak"

command -v flatpak >/dev/null || {
  echo "flatpak is required" >&2
  exit 1
}
[[ "$ARCH" == "x86_64" ]] || {
  echo "the current prebuilt release supports x86_64 only (found $ARCH)" >&2
  exit 1
}
command -v flatpak-builder >/dev/null || {
  echo "flatpak-builder is required (Arch: yay -S flatpak-builder)" >&2
  exit 1
}

if [[ ! -x "$STAGE/line-gtk" || ! -x "$STAGE/protocol/line-gtk-protocol" ]]; then
  env DENO="${DENO:-deno}" "$ROOT/packaging/make-prebuild.sh"
fi

WORK="$(mktemp -d /tmp/line-gtk-flatpak.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/payload"
cp -a "$STAGE"/. "$WORK/payload/"
cp "$ROOT/packaging/flatpak/dev.linegtk.LineGtk.yml" "$WORK/"
cp "$ROOT/packaging/flatpak/dev.linegtk.LineGtk.metainfo.xml" "$WORK/"

MANIFEST="$WORK/dev.linegtk.LineGtk.yml"
BUILD_DIR="$WORK/build"
REPO_DIR="$WORK/repo"
STATE_DIR="$WORK/state"

flatpak-builder \
  --force-clean \
  --user \
  --state-dir="$STATE_DIR" \
  --install-deps-from=flathub \
  --default-branch=stable \
  --repo="$REPO_DIR" \
  "$BUILD_DIR" \
  "$MANIFEST"

SMOKE_OUT="$WORK/sidecar.out"
set +e
timeout 5 flatpak-builder --run "$BUILD_DIR" "$MANIFEST" \
  env LINE_GTK_DATA=/tmp/line-gtk-flatpak-smoke \
  /app/share/line-gtk/protocol/line-gtk-protocol >"$SMOKE_OUT"
smoke_status=$?
set -e
[[ "$smoke_status" == 0 || "$smoke_status" == 124 ]]
grep -q '"event":"ready"' "$SMOKE_OUT"

rm -f "$OUT"
flatpak build-bundle \
  "$REPO_DIR" \
  "$OUT" \
  dev.linegtk.LineGtk \
  stable \
  --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo

sha256sum "$OUT"
"$ROOT/packaging/check-flatpak.sh" "$OUT"
echo "Flatpak bundle ready: $OUT"
