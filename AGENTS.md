# Repository Guidelines

## Project Structure & Module Organization

Line-GTK is an unofficial Linux LINE client with a Rust GTK4/Libadwaita frontend and a Deno TypeScript protocol sidecar. `src/ui/` contains interface modules; other `src/` modules handle configuration, credentials, notifications, and sidecar communication. `protocol/src/` implements authentication, messaging, and storage; `protocol/tests/` holds sidecar tests and fixtures. Keep requests and events consistent across `src/protocol.rs` and the sidecar. Translations and desktop assets live in `assets/`, build helpers in `scripts/`, distribution tooling in `packaging/`, and architecture notes in `docs/`.

## Build, Test, and Development Commands

Install stable Rust, GTK4 4.14+, Libadwaita 1.5+, Deno 2, Python 3, and ffmpeg/ffprobe. Run from the repository root:

```bash
cargo build --release                       # Build the native client
./scripts/build-sidecar.sh                   # Prepare, verify, compile, and smoke-test the sidecar
LINE_GTK_ROOT="$PWD" ./target/release/line-gtk # Launch locally
cargo fmt --all -- --check                   # Check Rust formatting
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features       # Run Rust tests
deno task --config protocol/deno.json check   # Format, lint, type-check, and test TypeScript
```

Use `DENO_OFFLINE=1 ./scripts/build-sidecar.sh` after dependencies are cached. See `packaging/README.md` for distribution builds.

## Coding Style & Naming Conventions

Use Rust 2024 conventions: four-space indentation, `snake_case` functions/modules, and `PascalCase` types. Apply `cargo fmt --all`. TypeScript uses Deno formatting, two-space indentation, `camelCase` functions, and descriptive `snake_case.ts` module names; format with `deno fmt --config protocol/deno.json`. Keep features in focused modules and update applicable translation files in `assets/lang/` for UI text changes.

## Testing Guidelines

Rust unit tests live alongside implementations in `#[cfg(test)]` modules. Sidecar tests use `Deno.test` and `@std/assert` in `protocol/tests/<feature>_test.ts`. Add regression tests for changed behavior, particularly account isolation, recovery, and message ordering. No numeric coverage threshold is configured. Run the checks above before submitting; manually verify affected GTK interactions and describe the results.

## Commit & Pull Request Guidelines

Recent history uses short, descriptive action-oriented subjects, such as “Load older messages by scrolling, with a bounded message window”; English and Japanese both appear. Follow that style and keep commits focused. PRs should explain behavior changes, link relevant issues, list validation performed, and include screenshots for visible UI changes. Flag protocol or packaging impacts.

## Security & Dependencies

Never commit session tokens, encryption keys, passwords, or account caches. Keep saved passwords in the OS Secret Service. Protocol dependencies are pinned; use the dependency scripts and `python3 scripts/vendor-protocol.py --verify` rather than editing vendored sources directly.
