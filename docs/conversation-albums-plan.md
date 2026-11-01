# Conversation albums: implementation plan

Status: proposal, 2026-10-24. Scope assumes full photo album management; the user's preferred scope is still open. This document does not authorize operations on live LINE albums.

## Existing implementation

- `src/ui/shell.rs` provides an Albums button in the conversation header.
- `src/ui/albums.rs` opens a modal album window scoped to the conversation, with an optional All chats view, pagination, refresh, photo viewing, saving, and retry states.
- `protocol/src/albums.ts` provides `list_albums`, `list_album_photos`, and `download_album_photo`, dispatched through `protocol/src/main.ts`.
- The sidecar filters a global album list by conversation. Each request scans at most four pages, so an empty result with a continuation cursor is not a final empty state.
- Photos are fetched in pages of 12. Original downloads are limited to three concurrent operations; the grid currently downloads originals for previews.
- Session checks, account/conversation/album-specific cache keys, and stale UI response guards already exist. Preserve these protections.
- `protocol/tests/albums_test.ts` covers pagination, session changes, cache isolation, image validation, and download concurrency. These are mocked tests, not proof of live server compatibility.
- The pinned `MoaService` in `protocol/vendor/deps/linejs/base/service/moa/mod.ts` only exposes listing and downloading. It has no create, rename, upload, or delete methods.

## Proposed scope

First delivery: improve browsing and support creating albums, adding local photos, renaming albums, removing selected photos, and deleting albums, where the verified protocol permits them.

Follow-up: add conversation photos to an album, batch saving, and opening album references from messages. Defer video uploads, premium original-quality uploads, sharing to other conversations, and background transfers while the application is closed.

LINE documents albums for 1-to-1 and group chats; legacy multi-person chats must first become groups. It currently documents 100 albums per chat, 1,000 items per album, 300 items per upload, names of up to 50 characters, and unique album names within a chat. Video and original-quality uploads require eligible premium membership. Deletions cannot be restored. Treat server responses as authoritative and recheck these rules when implementing. Source: [LINE album help](https://help.line.me/line/smartphone?contentId=20000248&lang=en).

## User experience

Keep the existing conversation-header entry and native GTK/Libadwaita appearance. Retain the album window for the first delivery so conversation navigation does not need a redesign.

1. **Album list:** show conversation name, album title, photo count, refresh, and New album. Add cover thumbnails only if a preview source can be verified. All chats remains a browsing view; creation requires an explicit conversation.
2. **Album detail:** show title, count, Add photos, Select, and a menu containing Rename and Delete album. Back restores the list and scroll position.
3. **Create:** enter a name, select local images, review thumbnails and destination, then choose Create. Validate names and files inline. Require at least one photo in the initial UI unless verified server behavior supports a useful empty-album workflow.
4. **Add photos:** use the native file chooser, show selected files and validation failures, then show per-file progress and a final result. Keep successful files visible when other files fail.
5. **Selection:** expose Save selected and Remove from album with the selection count. Confirm removal and album deletion with the album/conversation name and explain that the shared contents will be removed for participants.
6. **Feedback:** distinguish loading, truly empty, more pages available, permission denied, quota reached, expired session, album removed elsewhere, partial upload, and an operation whose outcome is uncertain.

Use keyboard-accessible actions and selection, visible focus, accessible names, and predictable Escape/Back behavior. Preserve the current album view when a recoverable request fails. Clear stale selections when the album or account changes. Keep Vietnamese as the default and update English, Japanese, Vietnamese, and Thai strings for new actions and states.

## Delivery sequence

### 1. Verify protocol feasibility

- Establish supported requests and response shapes for create, rename, upload/register photos, remove photos, and delete album. Do not infer write endpoints from read endpoints.
- Verify permissions, supported conversation types, standard-quality image handling, upload staging/finalization, server notifications, error codes, and whether request identifiers provide idempotency.
- Separate metadata creation from photo attachment if the server uses separate operations; define recovery when creation succeeds but attachment fails.
- Check whether conversation-specific album listing, thumbnails, and album-change events are available. The existing global list may be incomplete for conversation browsing; verify before treating it as exhaustive.
- Capture sanitized fixtures without credentials or private account data. Any live mutation validation needs a designated test conversation and explicit authorization.
- Prefer an application-owned adapter under `protocol/src/` using verified public client facilities, or update the pinned dependency through the dependency scripts. Do not edit vendored files directly. Run `python3 scripts/vendor-protocol.py --verify` for dependency changes.

Exit criterion: a capability matrix and fixture-backed contract for each supported action. Unsupported actions stay out of the UI. This milestone determines the scope of subsequent management work.

### 2. Improve browsing independently

- Preserve conversation identity throughout requests; prevent results for a previous conversation from appearing in a newly opened context.
- Preserve loaded list/detail state and scroll position on Back.
- Fetch verified thumbnails for the grid when available; otherwise load originals on demand or for visible tiles, with the existing concurrency bound.
- Keep pagination bounded and distinguish an unmatched scanned page from the end of the conversation's albums.

Exit criterion: large albums remain responsive, and opening an album does not immediately download every loaded original.

### 3. Create, rename, and add photos

- Add the verified protocol adapter, service operations, RPC dispatch, and GTK request handling together.
- Proposed application RPC names: `create_album`, `rename_album`, and `add_album_photos`. These are internal names, not claims about LINE endpoints.
- Require `chatMid` for every write and `albumId` for existing albums. Keep IDs as strings. Use an operation ID for progress correlation; it does not imply server-side deduplication.
- Validate files and enforce bounded file sizes, decoded dimensions, file counts, and upload concurrency. Exact accepted formats and limits must come from milestone 1.
- Freeze the destination account/conversation/album for an operation. Check the session before every remote step and before applying results.
- Disable duplicate submission while an operation is pending. Distinguish confirmed failure from a timeout after the server may have committed a write; reconcile before offering resubmission.
- Return per-file outcomes and any created album ID. Report partial completion without silently replaying successful files. Stop queued work on cancellation; report any remote changes already committed.

Exit criterion: a created album and added photos are visible after refresh and on another LINE device, with partial failures recoverable without duplicate albums or uploads.

### 4. Remove photos and delete albums

- Proposed application RPC names: `remove_album_photos` and `delete_album`.
- Enforce verified permissions in the sidecar as well as the UI. Serialize conflicting writes to the same album.
- Apply removal after confirmed success, invalidate affected metadata/photo lookup entries and cache eligibility, and refresh counts. Coordinate in-flight downloads so deleted content is not reintroduced into the view.
- Handle albums or photos already removed on another device. Reconcile uncertain outcomes before retrying.
- Refresh after successful local changes. Use verified server events if available; otherwise refresh when the album window regains focus and on explicit request, coalescing overlapping refreshes.

Exit criterion: deletion updates the list/detail correctly, and stale requests cannot restore removed tiles.

### 5. Conversation photo actions and batch saving

- Add Add to album to eligible image-message actions, scoped to that conversation. Reuse a verified original retrieval path; do not upload a preview as the original. Explain when the source photo has expired.
- Add Save selected, then Save album with bounded downloads, filename collision handling, progress, cancellation, and a partial-failure summary.
- Add album-message navigation only after verifying the incoming message metadata and event format.

## Implementation boundaries

| Area | Expected changes |
| --- | --- |
| `protocol/src/albums.ts` | Album orchestration, validation, session guards, write recovery, cache invalidation |
| New `protocol/src/album_api.ts`, if needed | Verified wire operations, isolated from UI and service orchestration |
| `protocol/src/main.ts` | RPC dispatch and progress events |
| `src/protocol.rs` | Shared request/result/event models for new operations |
| `src/ui/albums.rs` | List/detail state, dialogs, selection, progress, confirmation |
| `src/ui/state.rs`, `src/ui/events.rs` | Pending operations, response/progress routing, session teardown |
| Image-message action modules | Follow-up Add to album entry |
| `assets/lang/{eng,ja,vie,thai}.json` | New strings and error messages |
| `protocol/tests/albums_test.ts` and adapter tests | Regression coverage and verified response fixtures |

Split transfer state and pure state transitions into focused modules if needed rather than continuing to grow the album widget module.

## Validation and acceptance

- Service/adapter tests: contract parsing, permission/quota errors, invalid input, account/conversation isolation, stale responses, cancellation, partial commits, timeout after commit, duplicate submission, remote deletion, and cache invalidation.
- Rust tests: changed pure state transitions and protocol serialization, including ignoring stale progress after close, navigation, or account change.
- Run repository checks: Rust formatting, Clippy, Rust tests, Deno check task, dependency verification where applicable, and the sidecar build/smoke test.
- Manually verify GTK keyboard navigation, file chooser, selection, light/dark themes, narrow windows, translated labels, and return to the conversation. Include screenshots with visible UI changes.
- With authorized test data, verify supported actions in a 1-to-1 chat and a group, confirm results on another LINE device, and exercise lost connectivity and partial uploads. Record mocked and live verification separately.

The first implementation task is protocol feasibility. Browsing improvements can ship independently while write support is established.
