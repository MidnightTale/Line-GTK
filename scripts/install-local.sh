#!/usr/bin/env bash
# Rebuild the working tree into the line-gtk-local pacman package and install it.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="${LINE_GTK_BUILD_DIR:-/tmp/opencode/line-gtk-local}"

current="$(pacman -Q line-gtk-local 2>/dev/null | awk '{print $2}' || true)"
rel="${current##*-}"
[[ "$rel" =~ ^[0-9]+$ ]] || rel=0

mkdir -p "$work"
cp "$root/packaging/local/PKGBUILD" "$work/PKGBUILD"
cd "$work"
rm -f ./*.pkg.tar.zst

LINE_GTK_SRC="$root" LINE_GTK_PKGREL=$((rel + 1)) makepkg -f --noconfirm
pkg="$(ls "$work"/line-gtk-local-*.pkg.tar.zst)"
pkexec pacman -U --noconfirm "$pkg"
pacman -Q line-gtk-local
echo "Installed. Restart LINE GTK to use it."
