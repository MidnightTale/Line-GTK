# Changes proposed for upstream

This integration collects the fork's changes since upstream `master` commit
`4b85a0d` (release 0.1.2). It includes contributions imported from
[shirafukayayoi/Line-GTK](https://github.com/shirafukayayoi/Line-GTK), with their
commit authors preserved, and subsequent work in
[WindyWin/Line-GTK](https://github.com/WindyWin/Line-GTK).

## Added features

- Email/password login with phone verification, optional Secret Service credential
  storage, and startup-only automatic sign-in.
- Japanese and Vietnamese translations and font defaults. Vietnamese is the
  current default language; existing language selections are retained.
- Quoted replies, navigation to the original message, group member mentions,
  mention notifications, and contact profiles.
- Standard message reactions, received/custom LINE emoji reaction handling,
  reaction participant lists, Unicode emoji input, and received inline LINE emoji.
- Confirmed message unsending, whole-message copying, and a dialog for selecting
  and copying part of a message.
- Chat category filters, locally hidden and pinned chats, contact blocking, and
  global/per-contact read receipt pauses with an explicit mark-read action.
- Group sender names, persistent group event notices, and structured call-history
  cards.
- Conversation albums and original photo downloads; shared media browsing with
  navigation back to source messages.
- Video playback and saving, sticker hover previews, attachment drafts with
  removable previews, and link preview cards that automatically load thumbnails,
  with YouTube and TikTok oEmbed support.
- Desktop autostart and an option to start in the background.

## Reliability and behavior changes

- Cached chat restoration at startup and while switching conversations, delta
  refreshes, and updates to individual message rows.
- Older-history loading by scrolling with a bounded visible message window.
- Recovery after sleep or network loss, reconnect retries, and deduplication of
  refreshed messages while preserving the current conversation state.
- Read/unread synchronization with other devices, account-scoped chat preferences,
  and checks that discard stale asynchronous results after account/chat changes.
- Message decryption and group metadata fixes, plus encrypted video integrity and
  original-media validation.
- Sticker notification images and sender-aware group notifications, including
  updates when images or names arrive after the initial notification.

## Protocol and packaging impact

- Rust/TypeScript RPC models and events expand together for the added features.
- Protocol dependencies are downloaded from checksum-pinned GitHub archives into
  an ignored local directory. Version pins, the verification manifest, import
  map, and update scripts remain committed. Offline builds reuse prepared sources
  or cached archives after compiler/npm caching.
- Sidecar startup/ping smoke checks and CI cover Rust, protocol, and packaging.
- Arch/local package helpers and Flatpak configuration include dependency and
  translation support. Automatic email sign-in requires a Secret Service provider.

## Limits and review follow-up

- Native poll creation and scheduling are not implemented. Food-poll preparation
  source and tests exist, but the current application does not wire its UI/RPCs.
- Voice calls remain experimental; an account switcher is not implemented.
- Received LINE emoji are supported; sending owned LINE emoji as message content
  and animated LINE emoji playback are not implemented. Custom emoji reactions
  can be rejected by LINE.
- Pins and hidden-chat preferences are local to each account on this computer.
  Albums are read-only; album video support is unavailable.
- The Vietnamese default is a fork policy choice for upstream review.
- Logged-in GTK interaction checks, interoperability with an official LINE client,
  and updated screenshots remain outstanding. Existing README screenshots are
  illustrative and do not verify the new interactions.

## Validation

Checked on 2026-10-26:

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo build --release`: passed.
- `cargo test --all-targets --all-features`: 125 passed, one display-dependent test
  ignored.
- `deno task --config protocol/deno.json check`: format, lint, type checking, and
  231 tests passed.
- `python3 scripts/vendor-protocol.py --verify`: 244 dependency files verified.
- Six Python dependency-preparation regression tests passed.
- Fresh source-tree preparation passed both with an empty archive cache using
  GitHub downloads and offline using the existing archive cache. The fresh tree
  also passed all 231 protocol tests and offline sidecar compilation/smoke checks.
- `DENO_OFFLINE=1 ./scripts/build-sidecar.sh`: compiled successfully; empty-session
  startup and ping smoke checks passed.

These automated checks do not establish live LINE interoperability or replace
manual verification of the affected GTK interactions. Distribution package builds
were not run as part of this submission.
