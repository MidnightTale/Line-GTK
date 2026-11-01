# Distribution notes

Release and AUR builds compile `protocol/src/main.ts` into
`protocol/line-gtk-protocol`. The resulting package is self-contained and does
not download JavaScript dependencies on first launch. Deno is a build-time tool
only for packaged releases; source-tree development can still use Deno directly.

`make-prebuild.sh` creates the release archive and runs `check-package.sh` to
verify the executable, compiled protocol runtime, desktop entry, and language
catalogs are present.

Both AUR PKGBUILDs disable `strip`: Deno appends its JavaScript payload after
the ELF sections, and stripping that executable corrupts it. Cargo still strips
the Rust application through its release profile. `check-package.sh` extracts
the final tarball or `.pkg.tar.zst` and requires the actual sidecar to emit
`ready` and answer `ping` with an empty data/cache directory; checking filenames
alone cannot detect a stripped Deno executable.

Build/check the git AUR package, then validate the resulting artifact with:

```bash
./packaging/check-package.sh /path/to/line-gtk-git-VERSION-x86_64.pkg.tar.zst
```

## Flatpak bundle

`make-flatpak.sh` wraps the prebuilt release in a Flatpak using the GNOME 50
runtime. It grants network, display, audio, GPU, notification, tray, and the
Discord IPC socket access needed by the current client. It does not grant home
directory or host filesystem access; GTK's document portal provides selected
attachments and download destinations.

On Arch Linux, install the builder and create the bundle with:

```bash
yay -S --needed flatpak flatpak-builder
./packaging/make-flatpak.sh
```

The output is `/tmp/line-gtk-VERSION-x86_64.flatpak`. Install it locally with:

```bash
flatpak install --user /tmp/line-gtk-VERSION-x86_64.flatpak
flatpak run dev.linegtk.LineGtk
```

This manifest consumes the locally built, self-contained payload and is intended
for GitHub release bundles. A future Flathub submission should replace the local
payload source with reproducible, network-independent source modules.

## Complete release assets

`make-release-assets.sh` builds and verifies both supported GitHub assets:

```bash
./packaging/make-release-assets.sh
```

It prints SHA-256 checksums and the matching `gh release create` command, but
does not commit, tag, push, or publish anything.

Protocol builds automatically prepare checksum-pinned GitHub library downloads.
Only the dependency manifest is committed; source files and licenses are included
in the built package. Fresh builds need network access. With prepared sources or
archives in `target/protocol-archives` and cached npm/compiler dependencies, use
`DENO_OFFLINE=1` to build without dependency downloads. See
[protocol dependency notes](../docs/protocol-reliability.md).
