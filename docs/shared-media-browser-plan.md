# Shared media browser: implementation plan

Status: first delivery implemented, 2026-10-24. Batch selection/saving remains the second delivery.

Cache direction confirmed by the user: reuse the existing cache, persist all discovered shared items and obtainable attachments, and defer advanced cache management. Bound active work and UI memory, not the amount of shared content retained on disk.

Implementation notes: the conversation header menu opens all three tabs. The browser renders pages of up to 60 items with Previous/Older controls, keeping widgets and decoded textures bounded while retaining the complete discovered index on disk. Metadata and scan checkpoints persist per account/conversation, existing originals are reused, and missing originals are cached with up to three concurrent transfers. Legacy message metadata without account provenance is verified through current-account discovery before adoption; original files remain reusable. Advanced cache policies remain deferred.

Validation: Rust formatting, Clippy, 101 Rust tests, 209 sidecar tests, native release build, dependency verification, and compiled-sidecar startup/ping checks passed. Tests include persistent source metadata, exact large IDs, restart reuse, sparse history, transfer concurrency, account changes, unsend, and clearing during an active download. An isolated GTK fixture smoke test also passed all three tabs and source-message navigation without read receipts. Live LINE compatibility still needs verification with a real conversation.

## Feature and scope

Add a conversation-scoped **Shared media** browser based on the supplied screenshot: **Photos / Videos**, **Links**, and **Files**, grouped by month with newest items first. The screenshot's Select action becomes a second delivery for batch saving.

This browser lists attachments and URLs from chat messages. Keep the existing LINE Albums entry and the separate proposal in `conversation-albums-plan.md` independent.

First delivery:

- Entry from the selected conversation's header menu, disabled without a selected conversation.
- All three tabs, month grouping, incremental loading, refresh, and useful empty/error states.
- Open photos, play videos, preview supported documents, save individual originals, open/copy links, and Show in conversation.
- Preserve each tab's scroll position and loaded results while the browser remains open.
- Populate all tabs from existing cached messages/media immediately, then progressively discover and cache the selected conversation's remaining accessible shared content. Reuse that cache across tabs, browser openings, and application restarts.

Second delivery: Select, Save selected for attachments, progress, cancellation, and per-item retry. Selection applies only to loaded items in the active tab; switching tabs clears it. Links retain Open and Copy actions without attachment selection.

Defer global cross-chat browsing, forwarding, deletion, album management, remote link previews, full-history search, background account-wide indexing, and advanced cache management such as quotas, automatic eviction, and storage-management UI.

## What the repository already provides

| Existing code | Reuse and constraint |
| --- | --- |
| `src/ui/shell.rs`, `src/ui/albums.rs` | Conversation header and native window conventions; album APIs are a different data source. |
| `src/protocol.rs::MessageInfo` | Message ID, content type, sender, timestamp, file name, duration, and cached paths. File size is not currently modeled. |
| `protocol/src/commands.ts::doFetchOlder` | Existing history request and normalization patterns. Its hydration and chat state effects should not run during gallery discovery. |
| `protocol/src/message_history.ts` | Disk history is capped at 2,000 messages per chat. Seed shared-item records from this cache and retain them beyond chat-history trimming; continue discovery for uncached history. |
| `src/ui/history.rs` | The chat renders a bounded window of 200 messages and supports message jumps. Its current jump searches at most 100 pages. |
| `src/ui/media.rs`, `src/ui/video.rs`, `src/ui/downloads.rs` | Viewers, save preferences, filename sanitization, and collision handling. The download helper currently takes the destination chat from `current_chat`; add an explicit origin context. |
| `protocol/src/outgoing_media.ts`, `protocol/src/main.ts::refetchMessageData` | E2EE-aware original downloads and video validation. The refetch fallback only searches the most recent 100 messages, so it needs an older-message resolution path. |
| `protocol/src/unsend.ts` | Persistent account-specific unsend tombstones and `message_unsent` events; apply them to browser results and pending downloads. |
| `protocol/src/cache_policy.ts`, `protocol/src/main.ts` | Reuse existing cache storage, media files, helpers, and clear actions. Preserve account ownership when reusing entries; add no browser-specific quota or eviction system. |

No dedicated server-side shared-media search API has been established by this investigation. The baseline uses the history APIs already present in the pinned client. A specialized API can replace discovery later if verified.

## Desktop interaction

Open a transient GTK/Libadwaita window titled **Shared media**, with the conversation name visible throughout. Use the application's native theme and typography. Retain the originating conversation identity if the main window changes chats; never silently retarget pending actions.

```text
Shared media · Conversation name             Refresh   Select*
       Photos / Videos     Links     Files

October 2026
[photo] [video · 0:24] [photo] [photo]
[photo] [photo]        [photo] [video · 1:08]

September 2026
[photo] [photo]        [photo] [photo]
                  Load older

* Added with batch saving in the second delivery.
```

- **Photos / Videos:** responsive square thumbnail grid, video indicator and duration when known. Show cached previews immediately while a bounded background queue caches missing previews and originals for all discovered attachments. Opening an item prioritizes its original in that queue.
- **Links:** rows containing the domain, displayed URL, sender, and sent date. Extract HTTP(S) URLs from normal message text, preserving distinct occurrences across messages. Deduplicate an identical URL within one message. Exclude quoted previews and Flex action payloads initially. Do not fetch websites or favicons automatically.
- **Files:** rows containing file icon, name, sender, date, and size when supplied by verified metadata. A missing size remains unknown. FILE messages enter this tab; stickers, voice messages, call records, and unsent messages do not.
- **Item actions:** Open, Save for attachments, Copy link for URLs, and Show in conversation. Use the existing supported image/video/document viewers and external-app fallback.
- **Navigation:** closing a viewer restores the browser's position. Show in conversation activates the originating chat, then navigates to the source message and highlights it. Handle unreachable targets explicitly.
- **Feedback:** distinguish cached results, background discovery/caching, offline partial results, failed page, unavailable original, and confirmed end of accessible history. An empty filtered page is not an empty conversation. Show simple cached/pending/failed status; cached attachments remain usable offline.
- **Live updates:** show a New items available action rather than inserting rows above the user's current position. Refresh merges by stable item key and preserves the visible anchor. Unsend removes affected entries immediately.

Use keyboard-operable tabs, visible focus, arrow navigation through media tiles, Enter to open, and Escape to leave selection before closing. Give each tile an accessible name with type, sender, date, and selection state. Month headings and loading status need accessible labels. Test narrow layouts and long translated labels.

## Discovery and protocol design

Introduce `protocol/src/shared_media.ts` for discovery/classification and `src/ui/shared_media.rs` for the window. Build on the existing message/media cache, adding a persistent shared-item index and a resumable conversation scan checkpoint in the same cache infrastructure. Reference existing media files instead of making browser-specific copies. Keep UI pagination separate from background discovery.

Proposed internal RPC, subject to fixture verification:

```text
list_shared_media({ chatMid, category, cursor?, limit? })
  -> { chatMid, category, items, nextCursor, scanState }

category: photos_videos | links | files
scanState: more | end
```

This RPC pages local indexed results by category. Return discovery/caching status alongside the page so exhausting cached rows does not imply remote history is exhausted. Start or resume one conversation discovery job when the browser opens; that job indexes every category together, independently of the selected tab. Failures use the existing RPC error mechanism, not `scanState: end`. The UI can say Searching older messages while discovery continues without matches.

Each item carries `messageId`, `chatMid`, `createdTime`, `senderMid`, `kind`, and optional `fileName`, `fileSize`, `durationMs`, `thumbnailPath`, or `url`. Use a stable identity consisting of account, chat, message, and URL occurrence where applicable. Keep message IDs as strings. Sort by descending timestamp with an exact numeric message-ID tie-breaker; define deterministic fallback ordering for malformed IDs and timestamps.

### Persistent caching and bounded work

1. Capture account, client instance, and session/work generation before starting. Recheck after every await and before emitting results or publishing cache files.
2. Seed the persistent index from existing cached messages and media, including available entries beyond the chat UI's current window. Reconcile new messages from the latest cursor and resume older discovery from the persisted checkpoint. Fetch normalized history without invoking chat hydration; enqueue missing attachments through the shared cache service. Verify anchor inclusion and timestamp semantics with fixtures.
3. Advance using the oldest **raw message scanned**, including messages that do not match the tab. Never derive the continuation from the last matching attachment.
4. Process at most three raw pages of 100 messages per work batch and return up to 60 indexed items per UI page initially. Persist every discovered shared item across all categories before advancing the scan checkpoint. These are work-batch and rendering limits, not disk-retention limits.
5. Yield between work batches, then continue progressively through the selected conversation's accessible history. Sparse pages must not stop discovery. Load older reveals additional indexed results while the background job continues; do not rescan history separately for each tab.
6. Detect repeated anchors, duplicate-only pages, and non-progressing cursors. Return a recoverable stalled-pagination error if completion is uncertain. Only verified exhaustion produces `end`, meaning the end of history accessible to this client.
7. Bind UI cursors to account, session generation, chat, and category. Persist the conversation scan checkpoint and attachment outcomes independently of those transient cursors. Closing the browser pauses its discovery/cache jobs while preserving completed work; reopening or restarting resumes missing work. Account/session changes stop active work and reject stale results.

Persist all discovered shared-item records, source-message metadata needed for retrieval/navigation, extracted URLs, thumbnails, and obtainable image/video/file originals. Reuse already cached files and deduplicate pending downloads by account/chat/message. Persist unavailable/failed outcomes so one expired attachment does not block the rest; expose explicit retry. URLs are cached as message data, without downloading the linked websites.

Reuse existing cache entries with their verified account association and preserve the existing private-file protections. A discontinuous cache or its oldest item is not evidence that all intervening history was scanned. Keep a simple persisted checkpoint/completion state so reopening does not repeat completed history discovery. Retain shared records when the 2,000-message chat cache trims older messages.

Bound only active jobs, resident query pages, and decoded textures. Do not add a shared-item count cap, automatic browser-cache eviction, a separate media store, or an advanced retention engine in this feature. Keep the existing explicit cache-clear actions working; detailed storage policies and management controls are follow-up work.

### Originals and thumbnails

Discovery must persist enough account-scoped source metadata to resolve an older message without searching only the latest 100 messages. Factor an origin-aware resolver shared by thumbnail retrieval and downloads: reuse the persisted source record and existing cached original first, then refetch around a verified message anchor only when necessary. Keep any required encryption material inside protected sidecar storage and out of RPC results, logs, and plan fixtures.

Preserve the existing `download_media` RPC for chat callers. Add a browser download operation or extend the internal resolver with explicit origin metadata; determine the final shape after anchor behavior is verified. The sidecar must confirm the account/chat/message association and content type rather than trusting a UI hint.

Preserve account/chat/message ownership for new cache records and check it before reusing existing paths. A session change must prevent both stale UI results and stale cache publication. Apply unsend masking again before accepting a late thumbnail or original response.

Use one bounded attachment queue, initially at most three concurrent transfers, to cache every discovered attachment. Prioritize explicitly opened items, then visible/near-visible previews, then remaining background work. Decode textures only for the rendered window. Use a virtualized list of month-heading rows and adaptive tile rows, or an equivalent GTK model proven by a small prototype. Avoid an ever-growing FlowBox. Decode and scale off the GTK main thread; reuse existing async texture helpers where suitable.

Keep preview and original paths distinct. Expired originals leave a useful unavailable state; never save a thumbnail as the original video or image. Clearing message cache also resets the derived shared-item index/checkpoint. Clearing media cache invalidates indexed file paths and queued writes; require a new browser open or explicit retry to refill cleared media. Prevent in-flight jobs from immediately restoring cleared entries. Preserve existing read-receipt policy: browsing or downloading shared items must not itself mark the conversation read. Advanced retention/eviction behavior remains deferred.

### Message navigation

The existing jump helper starts from the current chat window and can miss very old gallery items after its page limit. Extend navigation to accept an explicit conversation and verified source anchor, load a bounded message window around the target, and retain a route back to latest messages. Do not lift the current cap into an unlimited backward scan. Report inaccessible/unsent targets and preserve read-pause/history behavior.

## Delivery sequence

1. **Reuse and extend the cache.** Seed shared-item records from existing cached messages/media; add persistent source records, a resumable scan checkpoint, and attachment outcomes using existing storage helpers. Verify old-message resolution and account ownership with sanitized fixtures. Exit: existing attachments are reused, and indexed older items survive restart and chat-cache trimming.
2. **Implement the sidecar service.** Add classification, HTTP(S) extraction, progressive conversation discovery in bounded batches, a bounded queue caching all obtainable attachments, session checks, unsend handling, and origin-aware resolution. Wire RPCs through `protocol/src/main.ts`, Rust models in `src/protocol.rs`, and wrappers in `src/sidecar.rs`. Exit: all categories query the shared cache, missing content is cached progressively, and restart resumes work without duplicate transfers or read receipts.
3. **Implement the browser.** Add the header-menu entry, window/model, tabs, month grouping, virtualized rendering, per-item actions, and pending-response routing. Add the anchored message-navigation change. Update `assets/lang/{eng,ja,vie,thai}.json`. Exit: all three tabs work with older messages, including sparse and partially unavailable history.
4. **Add batch saving.** Add selection, count, Save selected, one destination chooser, bounded transfers, and cancellation. Reserve filenames or serialize destination writes so concurrent same-named files cannot overwrite each other. Keep successful results when another file fails; retry failed items only. Exit: mixed success and cancellation produce accurate per-item outcomes without duplicate saves.

Expected integration points: `src/ui/{mod,shell,state,events}.rs`, `src/ui/history.rs`, `src/ui/media.rs`, `src/ui/downloads.rs`, `protocol/src/main.ts`, `protocol/src/outgoing_media.ts`, and the new service/UI modules. Keep classification, cursor transitions, and transfer state testable outside widget code. No dependency/vendor changes are expected for the baseline history approach.

## Validation and acceptance

- Sidecar regression tests: all categories; multiple/punctuated/Unicode URLs; unsent and undecryptable messages; existing cache/file reuse; persistent index/checkpoint recovery; shared records surviving chat-cache trimming; empty pages during background discovery; equal timestamps; duplicate/stalled cursors; expired UI cursors; network failure after partial results; older originals; bounded transfer concurrency; no duplicate transfers across tabs/restarts; account changes; pause/resume; explicit cache clearing.
- Rust tests: protocol parsing, month grouping in local time, stable item identity/order, stale response rejection, explicit origin routing, and selection transitions. Batch saving adds meaningful collision, partial failure, and cancellation tests.
- GTK manual checks: open/close, resize, scroll and return from viewers, translated labels, light/dark themes, keyboard-only operation, accessibility labels, older-message navigation, and account switch while requests are pending. Capture screenshots for the implementation PR.
- Verify with a representative large-history fixture that widget count, decoded-image memory, and active transfers remain bounded while all discovered records and obtainable attachments are retained on disk. Reopening must show cached results immediately and avoid downloading existing originals again. Record observations instead of inventing a latency target.
- Verify unchanged unread counts and absence of read-receipt calls during discovery, thumbnail loading, and saving. Test Show in conversation under existing read-pause rules.
- Run repository checks for implementation: `cargo fmt --all -- --check`, Clippy, Rust tests, `deno task --config protocol/deno.json check`, and the sidecar build/smoke test. Run vendor verification only if dependencies change. Record mocked verification separately from live LINE checks.

The first implementation milestone is reusing and extending the persistent cache. History discovery then fills it progressively, and all three tabs browse the same cached content. Advanced cache management is a later feature.
