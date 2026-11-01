# Creators' packs and personal PNGs: implementation plan

Status: implemented in the working tree, 2026-10-28. Browser-based LINE STORE discovery is the initial flow; in-app catalog/search remains a separate optional feature.

Implementation notes:

- Both sources use `protocol/src/sticker_library.ts`, with account-scoped manifests and content-addressed originals outside the cache. Preview/review copies use the clearable sticker cache and are drained before cache deletion. A reviewed PNG’s digest is checked again during import.
- The picker uses a collection dropdown and 60-image pages. Saved image packs appear before native collections finish refreshing. Native collections use paged owned-product summaries and load grids on selection.
- Imports use the originating RPC request ID as their job identity, emit progress while that request is pending, and return the library result when finished. This keeps cancellation and timeout handling in the existing request lifecycle. There is no separate early job-acknowledgement response.
- Added `inspect_sticker_pngs`, `adopt_sticker_library`, and `sticker_owned_pack` alongside the planned library RPCs. The GTK picker/editor/dialog callbacks retain the originating account/conversation scope; logout and recovery close these dialogs.
- Legacy adoption is explicit. Saved editing aliases remain available across restarts and are applied only when the new per-item setting has not already been saved. Removing a store image also records its exclusion so Retry missing images does not restore it.
- Originals shared by several packs remain until the last reference is removed. GTK editors and sends snapshot originals into media cache before operating, so removing a library entry does not invalidate an active editor or queued upload.

Validation results are recorded below. The remaining design text preserves the implementation rationale and future catalog direction.

## Intended result

Make two sources available as reusable packs in the existing sticker picker:

1. Packs published by creators on LINE, added using a LINE STORE pack link or ID.
2. Personal PNGs, selected from disk and copied into a named pack.

Keep the sending behavior visible to the user:

| Source | Picker behavior | Delivery |
| --- | --- | --- |
| Account-owned LINE pack, including creators' packs | Existing native sticker collection | Native LINE sticker through the existing ownership-aware path |
| Pack downloaded through a store link | Saved image collection with source link | PNG photo or animated GIF through the image sender |
| Personal PNG pack | Saved image collection | PNG photo or GIF converted from APNG |

Importing a store link does not acquire ownership. Downloaded image packs retain image delivery even when an account owns the corresponding native pack; the native collection remains a separate choice. Preserve transparency in uploaded PNG bytes, while describing the recipient's final presentation only after live verification.

Discovery choice for the first delivery: use the external browser. Point Browse creators' packs at [LINE STORE's creators' catalog](https://store.line.me/stickershop/home/user/en), which has popular and new collections and category links. If in-app browsing/search is chosen, add the feasibility milestone described below before implementing the catalog. A public catalog page does not establish a supported catalog API.

## Existing implementation to reuse

- `protocol/src/sticker_import.ts` accepts numeric IDs and HTTPS links on the supported LINE hosts.
- `protocol/src/stickers.ts` already resolves pack metadata, downloads static PNGs, restores imported packs, and returns them with `imageOnly: true`. It currently retains at most 24 imported packs and accepts a partially downloaded pack without exposing per-item outcomes.
- `src/ui/composer_media.rs` already exposes store-link import, pack tabs, click-to-send images, and an image editing action.
- `src/ui/sticker_image_editor.rs` and `src/sticker_image.rs` provide size choices, optional flat-background removal, previews, and saved settings. Editing currently accepts images up to 2048 by 2048 pixels, but checks dimensions after decoding.
- `protocol/tests/sticker_import_test.ts` covers link validation, persistence, static asset reuse, and invalid imports. `protocol/tests/sticker_image_send_test.ts` verifies PNG bytes are uploaded as IMAGE with the PNG MIME type.
- The imported manifest currently lives at `cache/stickers/image-packs.json`, so clearing sticker cache also removes saved pack records. Image options currently use filenames as identity, which is unsuitable for arbitrary personal PNG names.
- Native pack enumeration currently requests up to 80 summaries and renders only the first 12 packs. This limits visibility of owned creators' packs independently of image importing.

## User flows

### Download a creators' pack

1. Open the picker and choose Add pack → From LINE STORE. Browse creators' packs opens the catalog in the browser; the dialog also accepts a pasted link or numeric ID.
2. Resolve metadata and show the title, available static previews, item count, source link, and the label Sends as photos before starting the full download.
3. Choose Download. Show downloaded/total counts, cancellation, and per-item failures. A pack with failed items is marked Incomplete and offers Retry missing; successful items remain usable.
4. Reimporting the same store pack updates that pack, reuses valid originals, and preserves its identity and saved settings. Keep the previous working revision until replacement data is committed; do not discard local items on metadata failure.
5. The pack appears immediately in the picker. Clicking sends with saved settings; the existing Edit size and background action remains available through both a visible action and keyboard access.

### Import personal PNGs

1. Choose Add pack → PNG files. Use a multi-file PNG chooser, then choose New pack with a name or an existing personal pack.
2. Show previews and individual validation results. Preserve selection order and allow removal before import. Explain that the app keeps copies, so later moving the source files is safe.
3. Choose Import to copy valid files into the library. Invalid files remain listed with a useful reason; do not silently skip them. Allow the user to explicitly continue with valid files.
4. Preserve original PNG bytes and transparency. Default personal PNGs to Original size with background removal off; retain existing store-image defaults for compatibility. Edits create sendable derivatives and never overwrite library originals.
5. Reimporting identical content into the same pack does not create duplicate items. Different images with the same filename remain separate. Allow identical content in different packs with independent settings.

### Use and manage saved packs

- Show image collections with a Photos delivery label; retain native packs' existing sticker delivery. Switching collection tabs keeps the current selection clear.
- Provide Rename, Add PNGs for personal packs, Remove item, and Remove pack. Removing a library entry never changes the original source files or previously sent messages. Offer confirmation for pack removal and use Undo for item removal where practical.
- Retain library entries and copied originals across restarts and ordinary cache clearing. Cache clearing removes thumbnails and send derivatives; explicit library removal deletes unreferenced originals after active readers finish.
- Open saved image packs without a network fetch. Sending still requires a ready session. Keep thumbnail decoding and file operations off the GTK main thread, and bound rendered widgets for large packs.
- Keep visible labels, keyboard navigation, focus restoration after dialogs, progress, cancellation, and actionable errors. Use existing GTK/Libadwaita styling and translate strings in all four language files.

## Storage and service design

Use one sidecar-owned image library for both sources, proposed in `protocol/src/sticker_library.ts`. Keep existing native LINE enumeration and sending in `stickers.ts`.

- Store a versioned manifest and originals under an account-scoped durable library directory, outside `cache/`. Derive filesystem-safe account directories through the existing account identity conventions; do not use raw external input as a path.
- A pack records an app-generated ID, source kind (`line_store` or `local_png`), name, optional LINE package ID/source URL, ordered item IDs, and download status. An item records its own ID, optional LINE sticker ID, original content digest, dimensions, and availability/error status. Library IDs are distinct from LINE identifiers.
- Key editing settings by account/pack/item identity rather than filename. Deduplicate original bytes within an account using a content digest while keeping per-item editing choices independent. Keep original paths distinct from generated thumbnail and send paths.
- Validate actual PNG content, readable regular-file status, dimensions, and byte limits before committing. Check headers/dimensions before full decoding and enforce bounded reads and decoded memory. Initial proposed import limits are 10 MiB per file, 2048 pixels per axis, and 100 files per batch; these are application limits, not claimed LINE limits. More batches may extend a pack.
- Reject corrupt and renamed non-PNG inputs. Accept bounded APNG animations and preserve their originals, converting to GIF when sending imported images. ZIP/folder import, other image formats, sound, popup effects, and editable LINE text stickers are follow-up work.
- Use atomic manifest replacement and private-file helpers. Copy into staging first; publish validated originals and then their manifest references. Serialize per-account mutations so concurrent imports or removals cannot overwrite one another.
- Bind jobs to the account and session/work generation. Cancellation or an account change stops pending work and rejects late results. Keep successful committed work; never let an old job repopulate a removed pack or cleared cache.
- Bound transfers, initially three concurrent downloads. Download only validated pack/sticker IDs through existing trusted CDN construction. Expose explicit retry for unavailable static artwork rather than claiming every store product supports this route.
- Remove the silent 24-pack truncation for the durable library. Page collection lists/items and bound active work rather than silently discarding saved collections.

Migration: preserve the legacy manifest until a new manifest and copied originals are committed. Legacy entries have no account provenance; offer a one-time adoption into the signed-in account rather than assigning them to every subsequent account. Adopt only valid readable entries, keep unresolved images retryable, and migrate legacy store-image options without changing appearance. Repeating migration must not duplicate packs. Do not delete the legacy files as part of the first migration.

## Integration and delivery sequence

1. **Persistent library and migration.** Implement the shared manifest, stable IDs, durable original storage, validation, account isolation, atomic mutations, and legacy adoption. Exit: packs survive restart and cache clearing; duplicate filenames and concurrent mutations behave correctly.
2. **Creators' pack download.** Refactor existing link import to populate the library, with metadata preview, progress, cancellation, partial status, and retry. Keep the existing `list_stickers` response usable during migration. Exit: a representative creators' pack can be imported, reopened offline, and sent as a PNG photo.
3. **Personal PNG import.** Add the multi-file chooser, review dialog, pack selection, copy operation, and stable image settings. Exit: imported files remain usable after their source files move, with transparency and independent settings preserved.
4. **Picker and management.** Add source actions and delivery labels, pack/item management, bounded rendering, and localized errors. Bind send/edit callbacks to the originating account and conversation so navigation cannot retarget a pending send.
5. **Owned-pack completeness.** Verify pagination fields in the pinned shop service and enumerate all owned packs progressively. Replace the 12-pack display restriction with lazy collection loading. Do not infer pagination semantics from the existing offset/limit call. Exit: owned creators' packs beyond the first page become selectable through the native path.

Internal library RPCs: `preview_sticker_pack`, `import_sticker_pack`, `import_sticker_pngs`, `list_sticker_library`, `update_sticker_library`, and `cancel_sticker_import`, with a `sticker_import_progress` event. These are application operations, not claims about LINE endpoints. The existing request ID identifies each import; progress and final results carry account identity, counts, and stable per-item failures. Request/result fields match across `src/protocol.rs`, `src/sidecar.rs`, and `protocol/src/main.ts`.

The PNG chooser passes explicitly selected local paths through the application's local RPC. Resolve/copy them only during that import, with validation and limits; keep original source paths out of persisted manifests and logs. Return library item paths for previews through the existing local-asset mechanism.

Expected UI modules: existing `src/ui/composer_media.rs` and `src/ui/sticker_image_editor.rs`, a focused `src/ui/sticker_library.rs`, and pending/response routing in `src/ui/{mod,state,events}.rs`. Extend `src/sticker_image.rs` for stable option identity. No vendored dependency edits are expected for image-library support.

If in-app store discovery is selected: first establish a usable public catalog/search contract, locale/region behavior, pagination, product types, and failure handling using verified sources and sanitized fixtures. Then add searchable paged results and pack previews feeding the same importer. Keep this milestone separate from image downloading; do not make personal PNG support depend on catalog feasibility. Purchase/login/checkout stay in LINE STORE for this proposal.

## Acceptance and validation

- Creators' import: valid links/IDs, representative creator-pack fixtures, malformed metadata, unavailable products, incomplete downloads, retry missing only, cancellation, repeated imports, and stable ordering/settings after refresh.
- Personal files: transparent PNGs, same-name different-content files, duplicate content, corrupt/non-PNG/APNG files, oversize dimensions/bytes, unreadable files, source mutation during copying, removal of source files, and mixed valid/invalid batches.
- Persistence: restart/offline reuse, atomic-write failures, migration retry, library retention after cache clearing, orphan cleanup, concurrent import/removal, and no late writes after cancellation/account change/removal.
- Delivery: photo imports never call native `send_sticker`; owned creators' packs retain native sending. Verify PNG MIME type and original alpha bytes through the existing sender. Test stale chat/account callbacks and failed-send recovery.
- GTK: keyboard-only import/edit/remove, focus, progress/cancel/retry, narrow windows, large collections, light/dark themes, and all translations. Capture screenshots when the interface is implemented.
- Run repository Rust/Deno checks and sidecar build/smoke tests for implementation changes. Perform manual cross-client verification with representative downloaded and personal images, and record recipient rendering separately from fixture-backed transport tests. No live messages are authorized by this planning document.

## Completed validation (2026-10-28)

- `cargo fmt --all -- --check` and `cargo clippy --all-targets --all-features -- -D warnings` passed.
- `cargo test --all-targets --all-features`: 148 passed, 7 ignored. The ordinary sandbox blocked a local socket test and the image loader’s D-Bus access; the same suite passed with those local capabilities enabled.
- `deno task --config protocol/deno.json check`: formatting, lint, type checking, dependency preparation, and all 278 tests passed. Coverage includes preview-cache cancellation, per-file import errors, source changes after review, restart/cache reuse, removal/deduplication, legacy adoption, account changes, and stale-write rejection immediately before atomic commit.
- `cargo build --release` passed. `DENO_OFFLINE=1 ./scripts/build-sidecar.sh` compiled the sidecar and passed its ready/ping smoke test with all 244 vendored dependency files verified.
- The ignored GTK fixture test was run separately in a private D-Bus session with isolated XDG directories and a mock sidecar. It passed collection selection, native grid loading, 60/1-item pagination, store preview, PNG review, successful import completion, and dialog cleanup. Screenshots were inspected in light and dark modes. The desktop PNG chooser and a full keyboard-only interaction pass still need manual verification.
- A read-only live check against the public CDN successfully previewed and downloaded all 40 static images from [creators’ pack 21802595](https://store.line.me/stickershop/product/21802595/en) into a temporary library. It used no account credentials, removed the temporary library afterward, and sent no messages. Native owned-pack enumeration and photo delivery to another LINE client still need verification with a real signed-in account.

Fixture screenshots: [personal pack picker](screenshots/sticker-library/picker-light.png), [downloaded creators’ pack](screenshots/sticker-library/creator-pack-light.png), [store preview](screenshots/sticker-library/store-preview-light.png), [PNG import, light](screenshots/sticker-library/png-review-light.png), [PNG import, dark](screenshots/sticker-library/png-review-dark.png).

## Animated imports and file optimization (2026-10-28)

The image library now accepts validated APNGs and prefers animation assets for store packs with `hasAnimation` metadata. Static fallback stays usable and marks the pack incomplete so the missing animation can be retried. Validation checks chunk CRCs, frame controls and sequence numbers, compressed streams, dimensions, total duration, and decoded canvas budget (240 frames, 30 seconds, 32 million pixels). Personal PNG/APNG reviews still verify the selected content digest at import.

`protocol/src/sticker_optimize.ts` provides bounded ffmpeg conversion. Animated output uses a global GIF palette, binary transparency, configurable frame rate/colors, and optional proportional downscaling. Static output uses PNG compression and optional downscaling. Same-size static output that grows is discarded in favor of the smaller original bytes. Conversion output is capped at 10 MiB, work is serialized, the timeout is 45 seconds, and cache clearing cancels/drains conversions before deleting their output. `prepare_sticker_image` creates or reuses a cache derivative; `set_sticker_optimization` saves per-item settings. Neither operation overwrites the durable original or calls a native sticker-send method.

The **Optimize image** dialog keeps size comparison and actions in its footer, with resolution and GIF-specific controls, preview, Save settings, and Send/Send GIF. Saved settings apply to later sends. The existing artwork padding/background editor remains separate for static images; optimization runs after those visual changes. Animated sources use the optimizer to avoid flattening their frames. GIF image messages and APNG previews follow the existing animation setting. Controls and errors are translated in all four catalogs.

Conversion requires the existing ffmpeg runtime dependency. The Deno check task now permits ffmpeg/ffprobe for integration tests, and the protocol CI job installs ffmpeg explicitly. No vendored sources or dependency versions were changed for this feature.

Validation: 285 protocol tests and 149 Rust tests pass (7 Rust tests remain ignored in the ordinary suite). Integration tests verify GIF frames/timing/transparency, PNG sizing, saved presets and cache reuse, converter cancellation/draining, creator animation fallback/upgrades, and encrypted GIF IMAGE metadata without live sends. The isolated GTK fixture verifies both optimizer layouts, animated preview decoding, and a GIF send through its mock sidecar.

A live public-CDN check downloaded all 24 animated items in [NIKKE pack 37304940](https://store.line.me/stickershop/product/37304940/en). Its first APNG converted from 215,737 bytes to a 48,636-byte GIF using 128 px, 10 fps, and 64 colors. ffprobe confirmed a 128 × 120, 10-frame GIF with a one-second duration. The temporary library was removed; no account credentials or messages were involved. Recipient playback in official LINE remains a live verification step.

GIF sends now upload a PNG still preview with alpha instead of a JPEG preview, so transparency survives before the recipient downloads the full animation. Preview generation also avoids upscaling small originals. The encrypted-send regression decrypts the separate preview, checks its PNG signature, and decodes a transparent pixel. This fixes flattened preview content; any loading background drawn by the official client still needs live verification.

### Smaller GIF downloads

Unconfigured animations now use 128 px, 10 fps, and 64 colors instead of original dimensions, 15 fps, and 128 colors. Static PNG defaults and explicit saved presets are preserved. **Small GIF** previews the compact preset for items that already have larger saved settings; additional 96 px, 5/8 fps, and 32-color choices are available. Palette conversion processes changed rectangles, and derivative cache version 2 prevents reuse of old conversion results. Durable originals are unchanged.

A public-CDN measurement of NIKKE sticker 915179497 produced a 221,763-byte GIF with the old settings and a 48,636-byte GIF with the new default, a 78% reduction. The source APNG was 215,737 bytes. The regression suite also checks a larger generated animation shrinks by more than half, remains animated at 128 × 96 with a one-second duration, preserves its original, and respects saved settings after restart. The protocol suite now has 286 passing tests.

Optimizer screenshots: [static PNG](screenshots/sticker-library/optimize-static-light.png), [animation, light](screenshots/sticker-library/optimize-animation-light.png), [animation, dark](screenshots/sticker-library/optimize-animation-dark.png).


### Pack tabs, context menus, and automatic GIF settings

Each pack now has a tab in a horizontally scrollable strip. Only the selected pack loads its gallery; changing tabs releases hidden previews. Clicking an animated image sends a GIF directly. Grid thumbnails animate as soon as they appear, following the app animation setting, and restart when mapped again.

Right-click an image for optimization, static-image artwork editing, or removal. Right-click a pack tab for its LINE STORE link, adding PNGs, retrying incomplete downloads, renaming, or removal where applicable. Menu/Shift+F10 also opens these menus. These actions no longer occupy space below each thumbnail or the gallery.

Unconfigured GIFs use 32 colors and 10 fps. A source whose longest edge is at most 128 px uses the 128 px preset; larger sources use 256 px. Conversion preserves aspect ratio and never upscales. The picker and send path derive defaults from the original dimensions, including existing libraries; explicitly saved settings remain intact. The Small GIF shortcut uses 128 px, 10 fps, and 32 colors.

Validation: all 286 protocol tests and 152 ordinary Rust tests pass, as do formatting, Clippy, and the isolated GTK fixture. The fixture checks tab navigation and pagination, mapped context menus, thumbnail frame changes without hovering, and direct/default plus customized GIF sends through the mock sidecar. No live message was sent.

Screenshots: [pack tabs](screenshots/sticker-library/tabs-light.png), [animated grid](screenshots/sticker-library/animated-grid-light.png), [image menu](screenshots/sticker-library/image-context-menu-light.png), [pack menu](screenshots/sticker-library/pack-context-menu-light.png).
