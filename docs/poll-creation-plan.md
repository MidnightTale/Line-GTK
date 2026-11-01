# Poll creation: implementation plan

Status: proposal, 2026-10-25. Assumption: the requested feature is general native LINE poll creation. Whether the first delivery should focus on food polls remains open. This is a planning change; no live polls have been created.

## Current implementation

- The client has no wired poll creation feature. `src/ui/shell.rs` already has a conversation menu suitable for a Create poll action.
- `protocol/src/food_polls.ts` supports per-account group configuration and Google Maps restaurant previews. Its read/save responses explicitly report `sendingSupported: false`; there is no creation or scheduling operation.
- `src/ui/food_polls.rs` contains a preparation dialog, but `src/ui/mod.rs` does not declare or wire that module, `src/ui/state.rs` has no food poll state/pending variant, and `protocol/src/main.ts` does not import the service or dispatch its RPC methods. Treat these as reusable, disconnected modules.
- `docs/food-polls.md` describes a group menu that is not wired in the current checkout. Update that document when the integration ships.
- The pinned dependency is `evex-dev/linejs` v3.5.0. Its vendored services and generated types have no poll creation wrapper. The timeline post helper and chat-app discovery methods do not establish a poll contract.
- `protocol/tests/food_polls_test.ts` covers configuration, provider discovery, private storage, and account isolation with mocked HTTP. It does not establish LINE poll compatibility.

## Scope

First delivery: manually create a native text poll in a joined group, show a confirmed result, and expose the verified poll reference so participants can open it. Group-only is an initial product scope, not a claim about LINE's complete conversation support.

Use a question and at least two nonempty text options. Propose single-choice, named voting, no deadline, and no participant-added options as initial defaults; verify that the service supports these settings before adopting them. Add multiple-choice, anonymous voting, participant-added options, and a closing deadline only where their wire semantics are verified. Enforce actual limits from the verified contract rather than copying the food preview's 2–10 restaurant setting.

Defer image/date polls, editing/deleting/closing polls, voting and results inside GTK, automatic announcements, and recurring scheduling. A usable, verified way to open the created poll is required for the first delivery. If that needs an authenticated embedded view or native voting support, revisit scope before building a creation-only UI.

Native poll creation remains the intended transport. Numbered chat messages with reply counting would be a separate feature requiring its own voting rules; do not silently substitute that behavior.

LINE documents text and date polls, options combining text and images, anonymous voting, participant-added options, and automatic chat notification upon creation. Its desktop workflow uses the conversation menu. These product behaviors do not establish support in the unofficial client. Sources: [LINE Help: About polls](https://help.line.me/line/smartphone?contentId=20003458&lang=en), [LINE poll guide](https://guide.line.me/ja/communication/chat-poll.html).

## Creation flow

1. Open the conversation menu and choose Create poll. Offer it only for supported conversations and a ready session. Fix the menu tooltip/accessibility name so it describes the expanded conversation menu.
2. Show a native GTK/Libadwaita dialog with the destination group, question, two initial option fields, Add option, and accessible Remove option actions. Preserve option order. Use visible labels and inline errors; focus the first invalid field after a failed submission.
3. Put any verified optional voting settings below the options. Explain anonymous voting in terms of voter identity visibility; do not imply that the question or participation is private from group members. Show the time zone for a deadline and send an unambiguous timestamp.
4. Keep the destination fixed while the dialog is open. Navigating to another chat must not retarget the draft. Keep drafts in memory for the first delivery, clear them on logout/account change, and confirm discard when closing a modified draft.
5. Choose Create poll to publish. Show progress and prevent duplicate submission. Preserve the draft on a confirmed failure. An uncertain outcome gets a checking/unknown state, without an automatic retry.
6. On confirmed success, show the created question and an Open poll action using a verified reference. Do not send an extra notification message if LINE already produces one. Merge any returned and received notification by server message ID through the existing message path.

Support keyboard-only option editing, predictable focus after adding/removing an option, accessible field errors, scrolling at narrow window sizes, and light/dark themes. Use existing GTK styling and translate new UI/error strings in all four language files.

## Delivery sequence

### 1. Establish the protocol contract

- Identify the actual native poll service and supported authentication for this client's secondary-device session. Determine whether poll creation and posting its chat notification are one operation or separate steps; do not infer endpoints or reuse timeline post creation without evidence.
- Verify create, read-by-ID, and reconciliation/list capabilities, plus how a user opens and votes in the created poll. Investigate existing public client code and sanitized captures/fixtures where available. Official UI help alone cannot verify wire requests.
- Record request/response fields, supported conversation types, permissions, option/question limits and character counting, optional settings, deadline rules, server identifiers, errors, notification metadata, and any server deduplication mechanism.
- Test whether any returned link works for this desktop user. Keep credentials in the sidecar; never put tokens in a URL, RPC result, fixture, or log.
- Isolate verified operations in an application-owned `protocol/src/poll_api.ts`, or update the pinned dependency through the dependency scripts. Do not edit vendored sources directly.

Exit criterion: a documented capability matrix and sanitized fixtures establish a supported create request, authoritative success response, notification behavior, usable opening path, and strategy for uncertain outcomes. If native creation is unavailable, record the limitation before proceeding to publishing UI.

### 2. Add the service and RPC contract

- Add `protocol/src/polls.ts` for validation, conversation/session checks, operation tracking, creation, and reconciliation. Keep food discovery out of this service.
- Proposed internal RPCs: `poll_capabilities`, `create_poll`, and `poll_creation_status`. These names describe application operations, not LINE endpoints. Expose only capabilities actually verified in milestone 1.
- A create request carries `chatMid`, an `operationId`, a question, ordered text options, and supported settings. Keep server IDs as strings. Return an outcome such as `created`, `failed`, or `unknown`, with a poll ID/reference only when verified. Use stable error codes with localized frontend messages.
- Bind operations to the account, session generation, destination, and validated payload. Reusing an operation ID with different input must fail. Coalesce duplicate requests for the same operation; a local operation ID does not establish server idempotency.
- Recheck session identity and group membership before remote work and after awaited steps. A login change, including logout/login to the same account, must invalidate stale work. Client replacement during recovery needs an explicit policy rather than relying only on account MID equality.
- Distinguish failure before submission from timeout/disconnect after possible commitment. Reconcile by authoritative server identifiers or verified request tokens; question/options/time similarity alone is insufficient proof. Without a conclusive read path, report an unknown outcome and direct the user to check the group before creating another poll.
- If creation and notification are separate, retain confirmed poll identity and report a notification failure without recreating the poll. Prevent queued work from continuing into another session.
- If recovery across sidecar restarts is needed, atomically persist a bounded per-account operation journal before submission, using private storage and no credentials. After a crash, reconcile unresolved records rather than replaying writes. Do not advertise restart-safe recovery without that journal.

Exit criterion: fixture-backed tests exercise successful creation, duplicate requests, session changes, and failure after possible server commitment without generating duplicate polls.

### 3. Wire the native dialog and message integration

- Add `src/ui/polls.rs`, declare/wire it in `src/ui/mod.rs`, and add the conversation menu entry in `src/ui/shell.rs`.
- Add state/pending handling in `src/ui/state.rs`, response routing in `src/ui/events.rs`, and shared models in `src/protocol.rs`. Match request/result names and optional fields across Rust and TypeScript.
- Route responses to the originating dialog/operation, using a weak reference and session identity. Ignore stale responses after close, logout, or replacement by a new dialog.
- Closing a dialog cannot undo a submitted server mutation. Keep its operation tracked so reopening can show its confirmed or uncertain outcome.
- Inspect the verified incoming poll notification format. Preserve existing message ordering and deduplication; add a minimal poll reference model/rendering path only where current message handling cannot expose a usable action.
- Keep navigation account/conversation-scoped and validate returned links before opening. An ordinary external browser must not be assumed to share LINE authentication.

Exit criterion: a poll created through GTK is visible and votable on another LINE device; the initiating client exposes a usable opening path and handles the server notification once.

### 4. Reuse creation for food polls

- Restore the preparation UI/service wiring deliberately and update its documentation to match shipped behavior. Replace hard-coded UI text with translation keys as part of that work.
- Feed the preview's question and editable restaurant options into the shared creation dialog. Freeze the reviewed options; creating a poll must not silently refetch and replace restaurants.
- Keep addresses, distances, Maps links, and provider attribution available in the preview. Verify native option formats/limits before deciding whether extra information belongs in option text or a separate, explicitly reviewed message.
- Configuration Save and Preview remain non-publishing operations. Treat `flagged`, saved time, and weekdays as preparation settings until a scheduler actually exists.
- Plan recurring creation separately after manual creation and recovery work. That plan must define time zones, missed runs, overlap, per-account/group/date deduplication, restart recovery, and behavior while the application is closed.

## Validation and acceptance

- Adapter/service tests: verified fixtures, malformed responses, validation boundaries, unsupported settings, permission loss, group departure, account/session changes, repeated operation IDs, concurrent submissions, timeout after commitment, notification failure, and unresolved operations after restart where supported.
- Rust tests: new protocol serialization and pure dialog/operation state transitions, including stale responses and preservation of the original destination.
- Manual GTK verification: keyboard navigation, option removal focus, validation, scrolling, translated labels, light/dark themes, draft recovery on failure, and unknown-result handling. Include screenshots with UI implementation changes.
- Run Rust formatting, Clippy, Rust tests, the Deno check task, and the sidecar build/smoke test when implementing. Run `python3 scripts/vendor-protocol.py --verify` for dependency changes.
- With a designated test group when implementation reaches live validation, confirm creation, settings, notification behavior, opening/voting on another device, and disconnect recovery. Record mocked and live verification separately.

The first implementation task is protocol feasibility. The main uncertainty is the native service contract and authentication, rather than constructing the form.
