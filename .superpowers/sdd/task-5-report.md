# Task 5 Report

## Status

Implemented in `uncached-core`.

## Changes

- Added backend Trash refusal tests for Windows, Linux/Android Unix, and macOS paths.
- Added assertions that `CleanupCapabilities.trash` is false while Windows permanent capability remains true and Unix/macOS remain validation-only.
- Added public `clean_to_trash` integration coverage proving `CoreError::SecureTrashUnavailable`, target path preservation, and content preservation.
- Strengthened existing public cleanup coverage to assert directory and content remain unchanged after Trash refusal.
- No production Trash handoff, path fallback, staging operation, or mutation was added. Existing secure backends already returned typed `SecureTrashUnavailable`; production behavior remains fail-closed.
- Preserved injected legacy Trash string mapping in test-only policy helpers.

## Verification

- Focused secure-cleanup tests: 15 passed.
- Focused public Trash test: 1 passed.
- Full `cargo test --all-targets`: 25 library tests, 41 `node_modules` tests, and 2 provider tests passed.
- `cargo fmt -- --check`: passed after formatting.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed in isolated target directory.

## TDD Note

The current HEAD already contained typed fail-closed Trash behavior from the secure cleanup contract work. New tests therefore passed immediately rather than producing a RED failure; no production implementation was invented or weakened to manufacture a failure. Tests provide missing backend/public coverage for the existing approved behavior.

## Concerns

- macOS and Linux/Android backend tests are cfg-gated and were not executable on this Windows host.
- Initial focused test invocation hit an existing target-directory linker artifact (`LNK1104`); isolated Cargo target directories avoided it.
- Existing untracked `target-*` build artifacts were not modified or removed.
