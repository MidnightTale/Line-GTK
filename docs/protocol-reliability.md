# Protocol dependencies and connection recovery

## Build dependencies

The protocol uses pinned downloads of linejs 3.5.0, linejs-types 3.5.0,
loose-types 1.0.1, and the standard packages matching the original lockfile:
path 1.1.6, internal 1.0.14, assert 1.0.19. Sources come from upstream GitHub
archives with SHA-256 verification and MIT licenses retained. Downloaded source
files live in the ignored `protocol/vendor/deps` directory; only its verification
manifest is committed. Application-owned call patches under
`protocol/vendor/linejs-call` remain committed.

`protocol/dependencies.json` maps package exports to the prepared local files,
including ShopService. No application module is loaded from JSR.
`python3 scripts/vendor-protocol.py --ensure` reuses verified sources or prepares
missing ones. It checks downloaded archives, the complete source manifest, and
the import map before installing sources, and never updates pins. Modified local
sources are rejected. `--verify` only checks existing files without downloading.
The sidecar build and Deno `prepare`, `check`, `dev`, and `sidecar` tasks prepare
sources automatically. Direct `deno check`/`deno test` commands require running
preparation first.

The first build needs GitHub archive access, npm access, and Deno's compiler
runtime. Archives are cached in `target/protocol-archives`; use `--archives DIR`
to supply another cache. `DENO_OFFLINE=1 ./scripts/build-sidecar.sh` prevents
dependency downloads, reusing prepared sources or restoring from cached archives.
Preparation also accepts `--offline`. Missing offline archives produce an error
with setup instructions. npm dependencies remain pinned with integrity hashes in
`protocol/deno.lock`, and compilation uses its frozen lockfile. A fresh toolchain
or empty compiler/npm cache still requires network access.

To change library versions, review and update the refs and archive SHA-256
values in `scripts/vendor-protocol.py`, the npm pins, and any affected local
call patches. Then run `./scripts/update-protocol.sh`, inspect the resulting
manifest/import-map/lockfile diff, run protocol tests, and rebuild the sidecar.
The update command deliberately regenerates pins; normal preparation does not.
It does not select a floating `latest` version. Packaging prepares sources before
compilation and includes the downloaded sources and licenses in the payload, so
installed applications do not need build-time downloads.

## Packaged sidecar

Keep `!strip` in both AUR PKGBUILDs. Rust's release profile strips only the
Rust executable, while makepkg must leave Deno's appended payload intact.
The build smoke-tests the executable; `packaging/check-package.sh` also tests
the extracted final archive. Run that checker after makepkg, since package
post-processing happens after `package()` returns.

## LINE connection failures

Every LINE HTTP exchange has a deadline. QR authentication permits a longer
exchange for phone confirmation. Session restoration permits three
retries after 1, 2, and 4 seconds, for network failures and HTTP 408/429/5xx.
Writes such as sending messages are never automatically replayed after a
network failure, since a timeout does not prove the server failed to accept
the original message.

Group membership changes can explicitly reject encryption with
`E2EE_RECREATE_GROUP_KEY` (99). Another member's client may already have
rotated the key, so the sidecar first checks that every group member has an
encryption key and then reads the server's latest group key. If that generation
differs from the one just rejected it is reused. Only otherwise does the sidecar
register a new shared key. If a member has no encryption key it refuses with
`E2EE_MEMBER_KEYS_MISSING` instead of registering a key that member cannot read.
Each repair logs only identifiers (`[group-key]`: key ids and member counts),
never key material or message text. A rejection during text sending permits one
new encrypted envelope; media sending rebuilds its envelope without uploading
the file again. Concurrent repairs share registration within a client session.
Historical group keys remain cached so old messages can still be read. This recovery is installed
after QR, email/password, and saved-token login, including connection recovery.

QR and password login import the phone's encrypted keychain before enabling
messaging. QR metadata can be an object or a JSON string. Missing or invalid
keychains fail login with a translated message; they never generate replacement
identity keys. Imports are staged until the authenticated account is known and
the selected key matches the server public key. Private key generations are
stored by account and key ID, retaining older generations for history. Legacy
unscoped keys migrate only after verification against that account's public
keys. A stale account alias cannot override the newly imported generation.

Incoming encrypted messages with missing keys or transient key-fetch failures
retry decryption twice, after 250 and 750 milliseconds. The live receive loop
holds the message until recovery finishes, preserving order and avoiding a
second notification. Session changes cancel retries. Exhausted retries preserve
the encrypted message as a failure placeholder for later history refresh.
Unknown sender-key generations are fetched by their exact ID rather than
substituting the latest public key. Invalid messages and unsupported versions
do not trigger these retries.

Saved tokens are cleared only for explicit LINE authentication errors. A
network outage, HTTP 403, unsupported API, or server failure preserves the
session. The UI distinguishes connection/server/protocol failures and offers
Retry through the existing recovery action, which preserves credentials.
An actual expired session still requires QR login.

Boot restoration runs without blocking protocol stdin. A new login/logout
cancels its network requests so a late restoration cannot replace the new
session. Detached linejs polling failures are classified too: transient errors
trigger bounded process recovery, and authentication failures request login.
Repeated errors cannot schedule duplicate recoveries. The recovery budget only
resets after the restored process has remained alive for 30 seconds.

These measures handle client recovery and packaging. They cannot prevent LINE
outages, remove account restrictions/rate limits, or guarantee compatibility
after LINE changes its protocol. Unknown errors remain visible in the log and
may require a reviewed linejs update.
