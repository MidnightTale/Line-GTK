# Distribution notes

Release and AUR builds compile `protocol/src/main.ts` into
`protocol/line-gtk-protocol`. The resulting package is self-contained and does
not download JavaScript dependencies on first launch. Deno is a build-time tool
only for packaged releases; source-tree development can still use Deno directly.

`make-prebuild.sh` creates the release archive and runs `check-package.sh` to
verify the executable, compiled protocol runtime, desktop entry, and language
catalogs are present.

## Flatpak status

Flatpak is a good next distribution target, but publishing a manifest before the
application's subprocess/audio portal boundaries are complete would require
overly broad filesystem and device permissions. The current Deno sandbox is
already narrower than `-A`, while Flatpak work still needs:

- PipeWire microphone and speaker access through portals;
- document-portal paths for arbitrary attachments;
- notification and background/tray portal validation;
- vendored Cargo sources and the compiled protocol artifact in the source archive.

Until those are completed and tested, the release tarball and AUR packages are
the supported offline distributions. This avoids advertising a Flatpak sandbox
that provides misleading security guarantees.
